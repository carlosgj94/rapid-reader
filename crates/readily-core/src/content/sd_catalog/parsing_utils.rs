use super::*;

pub(super) fn next_word_bounds(text: &str, mut cursor: usize) -> Option<(usize, usize, usize)> {
    let bytes = text.as_bytes();
    let len = bytes.len();

    while cursor < len && bytes[cursor].is_ascii_whitespace() {
        cursor += 1;
    }
    if cursor >= len {
        return None;
    }

    let start = cursor;
    while cursor < len && !bytes[cursor].is_ascii_whitespace() {
        cursor += 1;
    }

    Some((start, cursor, cursor))
}

impl HtmlParseState {
    fn has(self, flag: u8) -> bool {
        self.flags & flag != 0
    }

    fn set(&mut self, flag: u8, enabled: bool) {
        if enabled {
            self.flags |= flag;
        } else {
            self.flags &= !flag;
        }
    }

    pub(super) fn should_emit_text(self, treat_as_plain_text: bool) -> bool {
        if self.has(HTML_FLAG_IN_SCRIPT) || self.has(HTML_FLAG_IN_STYLE) {
            return false;
        }

        if treat_as_plain_text {
            return true;
        }

        if self.has(HTML_FLAG_BODY_SEEN) {
            return self.has(HTML_FLAG_IN_BODY);
        }

        !self.has(HTML_FLAG_IN_HEAD)
    }
}

fn eq_ascii_case_insensitive(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right.iter())
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
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

pub(super) fn path_is_plain_text(path: &[u8]) -> bool {
    ends_with_ascii_case_insensitive(path, b".txt")
        || ends_with_ascii_case_insensitive(path, b".text")
}

fn ends_with_ascii_case_insensitive(path: &[u8], suffix: &[u8]) -> bool {
    if suffix.len() > path.len() {
        return false;
    }
    eq_ascii_case_insensitive(&path[path.len() - suffix.len()..], suffix)
}

#[derive(Clone, Copy)]
struct HtmlTagInfo<'a> {
    local_name: &'a [u8],
    is_closing: bool,
    is_self_closing: bool,
}

fn parse_html_tag_info(tag: &[u8]) -> Option<HtmlTagInfo<'_>> {
    let tag = trim_ascii(tag);
    if tag.is_empty() {
        return None;
    }

    if tag.starts_with(b"!--") || tag.starts_with(b"!") || tag.starts_with(b"?") {
        return None;
    }

    let (is_closing, name_start) = if tag[0] == b'/' {
        (true, 1usize)
    } else {
        (false, 0usize)
    };
    let rest = trim_ascii(&tag[name_start..]);
    if rest.is_empty() {
        return None;
    }

    let is_self_closing = rest.ends_with(b"/");
    let mut name_end = 0usize;
    while name_end < rest.len()
        && !rest[name_end].is_ascii_whitespace()
        && rest[name_end] != b'/'
        && rest[name_end] != b'>'
    {
        name_end += 1;
    }
    if name_end == 0 {
        return None;
    }

    let name = &rest[..name_end];
    let local_name = name
        .iter()
        .rposition(|b| *b == b':')
        .map(|idx| &name[idx + 1..])
        .unwrap_or(name);

    Some(HtmlTagInfo {
        local_name,
        is_closing,
        is_self_closing,
    })
}

pub(super) fn apply_html_tag_state(tag: &[u8], state: &mut HtmlParseState) {
    let Some(tag_info) = parse_html_tag_info(tag) else {
        return;
    };
    let local_name = tag_info.local_name;
    let is_closing = tag_info.is_closing;
    let is_self_closing = tag_info.is_self_closing;

    if eq_ascii_case_insensitive(local_name, b"head") {
        state.set(HTML_FLAG_IN_HEAD, !is_closing && !is_self_closing);
        return;
    }

    if eq_ascii_case_insensitive(local_name, b"body") {
        if is_closing {
            state.set(HTML_FLAG_IN_BODY, false);
        } else {
            state.set(HTML_FLAG_BODY_SEEN, true);
            state.set(HTML_FLAG_IN_HEAD, false);
            state.set(HTML_FLAG_IN_BODY, !is_self_closing);
        }
        return;
    }

    if eq_ascii_case_insensitive(local_name, b"script") {
        state.set(HTML_FLAG_IN_SCRIPT, !is_closing && !is_self_closing);
        return;
    }

    if eq_ascii_case_insensitive(local_name, b"style") {
        state.set(HTML_FLAG_IN_STYLE, !is_closing && !is_self_closing);
    }
}

fn is_block_level_tag(local_name: &[u8]) -> bool {
    eq_ascii_case_insensitive(local_name, b"p")
        || eq_ascii_case_insensitive(local_name, b"div")
        || eq_ascii_case_insensitive(local_name, b"section")
        || eq_ascii_case_insensitive(local_name, b"article")
        || eq_ascii_case_insensitive(local_name, b"aside")
        || eq_ascii_case_insensitive(local_name, b"header")
        || eq_ascii_case_insensitive(local_name, b"footer")
        || eq_ascii_case_insensitive(local_name, b"nav")
        || eq_ascii_case_insensitive(local_name, b"li")
        || eq_ascii_case_insensitive(local_name, b"ul")
        || eq_ascii_case_insensitive(local_name, b"ol")
        || eq_ascii_case_insensitive(local_name, b"h1")
        || eq_ascii_case_insensitive(local_name, b"h2")
        || eq_ascii_case_insensitive(local_name, b"h3")
        || eq_ascii_case_insensitive(local_name, b"h4")
        || eq_ascii_case_insensitive(local_name, b"h5")
        || eq_ascii_case_insensitive(local_name, b"h6")
        || eq_ascii_case_insensitive(local_name, b"blockquote")
        || eq_ascii_case_insensitive(local_name, b"pre")
        || eq_ascii_case_insensitive(local_name, b"table")
        || eq_ascii_case_insensitive(local_name, b"tr")
        || eq_ascii_case_insensitive(local_name, b"br")
        || eq_ascii_case_insensitive(local_name, b"hr")
}

pub(super) fn tag_inserts_paragraph_break(tag: &[u8]) -> bool {
    parse_html_tag_info(tag)
        .map(|info| is_block_level_tag(info.local_name))
        .unwrap_or(false)
}

pub(super) fn update_chapter_label_from_resource<const N: usize>(
    resource_path: &str,
    out: &mut String<N>,
) {
    out.clear();
    let stem = resource_path
        .rsplit('/')
        .next()
        .unwrap_or(resource_path)
        .rsplit_once('.')
        .map(|(left, _)| left)
        .unwrap_or(resource_path)
        .trim();

    if let Some(chapter_number) = inferred_chapter_number(stem) {
        let _ = out.push_str("Chapter ");
        push_u32_as_ascii(out, chapter_number.max(1));
        return;
    }

    let mut word_start = true;
    let mut wrote_any = false;
    for byte in stem.as_bytes() {
        let mut out_byte = *byte;
        if out_byte == b'_' || out_byte == b'-' || out_byte == b'.' {
            out_byte = b' ';
        }

        if out_byte == b' ' {
            if !wrote_any || word_start {
                continue;
            }
            if out.push(' ').is_err() {
                break;
            }
            word_start = true;
            continue;
        }

        let ch = if out_byte.is_ascii_alphabetic() {
            if word_start {
                (out_byte as char).to_ascii_uppercase()
            } else {
                (out_byte as char).to_ascii_lowercase()
            }
        } else if out_byte.is_ascii_digit() {
            out_byte as char
        } else {
            continue;
        };

        if out.push(ch).is_err() {
            break;
        }
        wrote_any = true;
        word_start = false;
    }

    if !wrote_any {
        let _ = out.push_str("Section");
    }
}

fn inferred_chapter_number(stem: &str) -> Option<u32> {
    let bytes = stem.as_bytes();
    if bytes.is_empty() {
        return None;
    }

    if let Some(pos) = find_ascii_case_insensitive(bytes, b"-h-", 0) {
        let suffix = &bytes[pos + 3..];
        if let Some(value) = parse_leading_ascii_u32(suffix) {
            return Some(value.saturating_add(1));
        }
    }

    if bytes.iter().all(|b| b.is_ascii_digit()) {
        return parse_ascii_u32(bytes);
    }

    if contains_ascii_case_insensitive(bytes, b"chapter")
        || contains_ascii_case_insensitive(bytes, b"capitulo")
        || contains_ascii_case_insensitive(bytes, b"cap")
    {
        let mut end = bytes.len();
        while end > 0 && bytes[end - 1].is_ascii_digit() {
            end -= 1;
        }
        if end < bytes.len() {
            return parse_ascii_u32(&bytes[end..]);
        }
    }

    None
}

fn parse_ascii_u32(bytes: &[u8]) -> Option<u32> {
    if bytes.is_empty() || !bytes.iter().all(|b| b.is_ascii_digit()) {
        return None;
    }

    let mut value = 0u32;
    for &digit in bytes {
        value = value
            .saturating_mul(10)
            .saturating_add((digit - b'0') as u32);
    }
    Some(value)
}

fn parse_leading_ascii_u32(bytes: &[u8]) -> Option<u32> {
    let mut len = 0usize;
    while len < bytes.len() && bytes[len].is_ascii_digit() {
        len += 1;
    }
    if len == 0 {
        return None;
    }
    parse_ascii_u32(&bytes[..len])
}

pub(super) fn push_u32_as_ascii<const N: usize>(out: &mut String<N>, mut value: u32) {
    let mut digits = [0u8; 10];
    let mut len = 0usize;
    if value == 0 {
        let _ = out.push('0');
        return;
    }
    while value > 0 && len < digits.len() {
        digits[len] = (value % 10) as u8;
        value /= 10;
        len += 1;
    }
    for idx in (0..len).rev() {
        let _ = out.push((b'0' + digits[idx]) as char);
    }
}
