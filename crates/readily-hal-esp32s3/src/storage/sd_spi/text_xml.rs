fn read_u16_le(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn eq_ascii_case_insensitive(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b.iter())
            .all(|(left, right)| left.eq_ignore_ascii_case(right))
}

fn ends_with_ascii_case_insensitive(bytes: &[u8], suffix: &[u8]) -> bool {
    if suffix.len() > bytes.len() {
        return false;
    }
    eq_ascii_case_insensitive(&bytes[bytes.len() - suffix.len()..], suffix)
}

fn find_ascii_case_insensitive(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from >= haystack.len() || needle.len() > haystack.len() {
        return None;
    }

    let max_start = haystack.len().saturating_sub(needle.len());
    if from > max_start {
        return None;
    }

    (from..=max_start)
        .find(|&idx| eq_ascii_case_insensitive(&haystack[idx..idx + needle.len()], needle))
}

fn contains_ascii_case_insensitive(haystack: &[u8], needle: &[u8]) -> bool {
    find_ascii_case_insensitive(haystack, needle, 0).is_some()
}

fn is_text_resource_name(name: &[u8]) -> bool {
    if name.is_empty() || name[name.len() - 1] == b'/' {
        return false;
    }

    if contains_ascii_case_insensitive(name, b"META-INF/")
        || contains_ascii_case_insensitive(name, b"/META-INF/")
    {
        return false;
    }

    ends_with_ascii_case_insensitive(name, b".xhtml")
        || ends_with_ascii_case_insensitive(name, b".html")
        || ends_with_ascii_case_insensitive(name, b".htm")
        || ends_with_ascii_case_insensitive(name, b".txt")
}

fn copy_ascii_or_lossy<const N: usize>(source: &[u8], out: &mut String<N>) {
    out.clear();
    for &byte in source {
        let ch = if byte.is_ascii() { byte as char } else { '?' };
        if out.push(ch).is_err() {
            break;
        }
    }
}

fn trim_ascii(slice: &[u8]) -> &[u8] {
    let mut start = 0usize;
    let mut end = slice.len();
    while start < end && slice[start].is_ascii_whitespace() {
        start += 1;
    }
    while end > start && slice[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    &slice[start..end]
}

fn copy_utf8_or_ascii<const N: usize>(source: &[u8], out: &mut String<N>) -> bool {
    out.clear();
    let source = trim_ascii(source);
    if source.is_empty() {
        return false;
    }

    if let Ok(text) = str::from_utf8(source) {
        for ch in text.chars() {
            if out.push(ch).is_err() {
                break;
            }
        }
    } else {
        for &byte in source {
            let ch = if byte.is_ascii() { byte as char } else { '?' };
            if out.push(ch).is_err() {
                break;
            }
        }
    }

    !out.is_empty()
}

fn parse_xml_tag_text<const N: usize>(xml: &[u8], tag: &[u8], out: &mut String<N>) -> bool {
    let mut search_from = 0usize;
    while let Some(tag_pos) = find_ascii_case_insensitive(xml, tag, search_from) {
        let after_tag = tag_pos.saturating_add(tag.len());
        let Some(open_end_rel) = xml[after_tag..].iter().position(|b| *b == b'>') else {
            break;
        };
        let text_start = after_tag + open_end_rel + 1;
        let Some(text_end_rel) = xml[text_start..].iter().position(|b| *b == b'<') else {
            break;
        };
        let text_end = text_start + text_end_rel;
        if copy_utf8_or_ascii(&xml[text_start..text_end], out) {
            return true;
        }
        search_from = text_end.saturating_add(1);
    }
    false
}

fn parse_container_full_path<const N: usize>(xml: &[u8], out: &mut String<N>) -> bool {
    let mut search_from = 0usize;
    while let Some(attr_pos) = find_ascii_case_insensitive(xml, b"full-path", search_from) {
        let mut idx = attr_pos + b"full-path".len();
        while idx < xml.len() && xml[idx].is_ascii_whitespace() {
            idx += 1;
        }
        if idx >= xml.len() || xml[idx] != b'=' {
            search_from = idx.saturating_add(1);
            continue;
        }
        idx += 1;
        while idx < xml.len() && xml[idx].is_ascii_whitespace() {
            idx += 1;
        }
        if idx >= xml.len() {
            break;
        }
        let quote = xml[idx];
        if quote != b'\'' && quote != b'"' {
            search_from = idx.saturating_add(1);
            continue;
        }
        idx += 1;
        let start = idx;
        while idx < xml.len() && xml[idx] != quote {
            idx += 1;
        }
        if idx <= start {
            search_from = idx.saturating_add(1);
            continue;
        }

        out.clear();
        for &byte in &xml[start..idx] {
            if out.push(byte as char).is_err() {
                break;
            }
        }
        if !out.is_empty() {
            return true;
        }
        search_from = idx.saturating_add(1);
    }
    false
}

fn find_xml_element_bounds_in_range(
    xml: &[u8],
    local_name: &[u8],
    from: usize,
    end: usize,
) -> Option<(usize, usize)> {
    if from >= end || end > xml.len() {
        return None;
    }

    let mut cursor = from;
    while cursor < end {
        let lt_rel = xml[cursor..end].iter().position(|b| *b == b'<')?;
        let start = cursor + lt_rel;
        let mut name_start = start.saturating_add(1);
        if name_start >= end {
            return None;
        }

        // Ignore closing/doctype/pi tags.
        if matches!(xml[name_start], b'/' | b'!' | b'?') {
            cursor = name_start.saturating_add(1);
            continue;
        }

        while name_start < end && xml[name_start].is_ascii_whitespace() {
            name_start += 1;
        }
        if name_start >= end {
            return None;
        }

        let mut name_end = name_start;
        while name_end < end
            && !xml[name_end].is_ascii_whitespace()
            && xml[name_end] != b'/'
            && xml[name_end] != b'>'
        {
            name_end += 1;
        }
        if name_end <= name_start {
            cursor = start.saturating_add(1);
            continue;
        }

        let full_name = &xml[name_start..name_end];
        let local = full_name
            .iter()
            .rposition(|b| *b == b':')
            .map(|idx| &full_name[idx + 1..])
            .unwrap_or(full_name);

        if eq_ascii_case_insensitive(local, local_name) {
            let end_rel = xml[name_end..end].iter().position(|b| *b == b'>')?;
            return Some((start, name_end + end_rel + 1));
        }

        cursor = name_end.saturating_add(1);
    }

    None
}

fn find_xml_attr_value<'a>(tag: &'a [u8], attr: &[u8]) -> Option<&'a [u8]> {
    let mut search_from = 0usize;
    while let Some(attr_pos) = find_ascii_case_insensitive(tag, attr, search_from) {
        let prev_ok = attr_pos == 0
            || tag[attr_pos - 1].is_ascii_whitespace()
            || tag[attr_pos - 1] == b'<'
            || tag[attr_pos - 1] == b'/';
        if !prev_ok {
            search_from = attr_pos.saturating_add(1);
            continue;
        }

        let mut idx = attr_pos + attr.len();
        while idx < tag.len() && tag[idx].is_ascii_whitespace() {
            idx += 1;
        }
        if idx >= tag.len() || tag[idx] != b'=' {
            search_from = attr_pos.saturating_add(1);
            continue;
        }
        idx += 1;
        while idx < tag.len() && tag[idx].is_ascii_whitespace() {
            idx += 1;
        }
        if idx >= tag.len() {
            return None;
        }

        let quote = tag[idx];
        if quote == b'"' || quote == b'\'' {
            idx += 1;
            let start = idx;
            while idx < tag.len() && tag[idx] != quote {
                idx += 1;
            }
            if idx > start {
                return Some(&tag[start..idx]);
            }
            return None;
        }

        let start = idx;
        while idx < tag.len() && !tag[idx].is_ascii_whitespace() && tag[idx] != b'>' {
            idx += 1;
        }
        if idx > start {
            return Some(&tag[start..idx]);
        }
        return None;
    }
    None
}

fn parse_opf_meta_content<const N: usize>(
    opf: &[u8],
    attr_name: &[u8],
    out: &mut String<N>,
) -> bool {
    let mut cursor = 0usize;
    while let Some((start, end)) = find_xml_element_bounds_in_range(opf, b"meta", cursor, opf.len())
    {
        let tag = &opf[start..end];
        let name = find_xml_attr_value(tag, attr_name);
        let content = find_xml_attr_value(tag, b"content");
        if let (Some(name), Some(content)) = (name, content)
            && contains_ascii_case_insensitive(name, b"title")
            && copy_utf8_or_ascii(content, out)
        {
            return true;
        }
        cursor = end;
    }
    false
}

fn is_text_media_type(media: &[u8]) -> bool {
    contains_ascii_case_insensitive(media, b"xhtml")
        || contains_ascii_case_insensitive(media, b"html")
        || contains_ascii_case_insensitive(media, b"text/plain")
}

fn path_is_probably_front_matter(path: &[u8]) -> bool {
    contains_ascii_case_insensitive(path, b"cover")
        || contains_ascii_case_insensitive(path, b"portada")
        || contains_ascii_case_insensitive(path, b"cubierta")
        || contains_ascii_case_insensitive(path, b"info")
        || contains_ascii_case_insensitive(path, b"about")
        || contains_ascii_case_insensitive(path, b"acerca")
        || contains_ascii_case_insensitive(path, b"title")
        || contains_ascii_case_insensitive(path, b"frontmatter")
        || contains_ascii_case_insensitive(path, b"toc")
        || contains_ascii_case_insensitive(path, b"indice")
        || contains_ascii_case_insensitive(path, b"index")
        || contains_ascii_case_insensitive(path, b"nav")
        || contains_ascii_case_insensitive(path, b"contents")
        || contains_ascii_case_insensitive(path, b"credit")
        || contains_ascii_case_insensitive(path, b"license")
        || contains_ascii_case_insensitive(path, b"licencia")
        || contains_ascii_case_insensitive(path, b"imprint")
        || contains_ascii_case_insensitive(path, b"preface")
        || contains_ascii_case_insensitive(path, b"foreword")
        || contains_ascii_case_insensitive(path, b"prologue")
        || contains_ascii_case_insensitive(path, b"prologo")
        || contains_ascii_case_insensitive(path, b"dedicat")
        || contains_ascii_case_insensitive(path, b"introduc")
        || contains_ascii_case_insensitive(path, b"nota")
        || contains_ascii_case_insensitive(path, b"note")
        || contains_ascii_case_insensitive(path, b"warning")
        || contains_ascii_case_insensitive(path, b"advert")
        || contains_ascii_case_insensitive(path, b"copyright")
        || contains_ascii_case_insensitive(path, b"colophon")
        || contains_ascii_case_insensitive(path, b"legal")
        || contains_ascii_case_insensitive(path, b"acknowledg")
}

fn resolve_opf_href<const PATH_BYTES: usize>(
    opf_path: &str,
    href: &[u8],
    out: &mut String<PATH_BYTES>,
) -> bool {
    let href = trim_ascii(href);
    if href.is_empty() {
        return false;
    }

    let mut href_clean = href;
    if let Some(hash_pos) = href_clean.iter().position(|b| *b == b'#') {
        href_clean = &href_clean[..hash_pos];
    }
    if let Some(query_pos) = href_clean.iter().position(|b| *b == b'?') {
        href_clean = &href_clean[..query_pos];
    }
    let href_clean = trim_ascii(href_clean);
    if href_clean.is_empty() {
        return false;
    }

    let base_dir = opf_path
        .rsplit_once('/')
        .map(|(left, _)| left)
        .unwrap_or("");

    let mut provisional = String::<PATH_BYTES>::new();
    if !href_clean.starts_with(b"/") && !base_dir.is_empty() {
        for ch in base_dir.chars() {
            if provisional.push(ch).is_err() {
                return false;
            }
        }
        let _ = provisional.push('/');
    }
    for &byte in href_clean {
        let ch = if byte.is_ascii() { byte as char } else { '?' };
        if provisional.push(ch).is_err() {
            return false;
        }
    }

    let mut segments: Vec<&str, ZIP_PATH_SEGMENTS_MAX> = Vec::new();
    for seg in provisional.as_str().split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            let _ = segments.pop();
            continue;
        }
        if segments.push(seg).is_err() {
            return false;
        }
    }

    out.clear();
    for (idx, seg) in segments.iter().enumerate() {
        if idx > 0 {
            let _ = out.push('/');
        }
        for ch in seg.chars() {
            if out.push(ch).is_err() {
                return false;
            }
        }
    }

    !out.is_empty()
}

