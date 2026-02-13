const ZIP_TOC_BYTES: usize = 12_288;
const ZIP_TOC_LABEL_SEARCH_WINDOW: usize = 1_536;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TocFormat {
    Ncx,
    NavXhtml,
}

#[derive(Clone, Debug)]
struct TocChapterSelection {
    path: String<ZIP_PATH_BYTES>,
    fragment: String<ZIP_PATH_BYTES>,
    label: String<SD_CHAPTER_LABEL_BYTES>,
    index: u16,
    total: u16,
}

fn media_is_ncx(media: &[u8]) -> bool {
    contains_ascii_case_insensitive(media, b"ncx")
}

fn path_is_ncx(path: &[u8]) -> bool {
    ends_with_ascii_case_insensitive(path, b".ncx")
}

fn to_toc_format(path: &[u8], media: Option<&[u8]>, properties: Option<&[u8]>) -> TocFormat {
    if media.is_some_and(media_is_ncx)
        || path_is_ncx(path)
        || properties.is_some_and(|value| contains_ascii_case_insensitive(value, b"ncx"))
    {
        TocFormat::Ncx
    } else {
        TocFormat::NavXhtml
    }
}

fn copy_text_to_string<const N: usize>(source: &[u8], out: &mut String<N>) -> bool {
    out.clear();
    copy_utf8_or_ascii(source, out)
}

fn parse_spine_toc_id<const N: usize>(opf: &[u8], out: &mut String<N>) -> bool {
    let Some((start, end)) = find_xml_element_bounds_in_range(opf, b"spine", 0, opf.len()) else {
        return false;
    };
    let tag = &opf[start..end];
    let Some(id) = find_xml_attr_value(tag, b"toc") else {
        return false;
    };
    copy_text_to_string(id, out)
}

fn find_manifest_href_by_predicate<const PATH_BYTES: usize, P>(
    opf: &[u8],
    opf_path: &str,
    mut predicate: P,
    out_path: &mut String<PATH_BYTES>,
    out_format: &mut TocFormat,
) -> bool
where
    P: FnMut(&[u8], Option<&[u8]>, Option<&[u8]>) -> bool,
{
    let mut cursor = 0usize;
    while let Some((start, end)) = find_xml_element_bounds_in_range(opf, b"item", cursor, opf.len())
    {
        let tag = &opf[start..end];
        let Some(href) = find_xml_attr_value(tag, b"href") else {
            cursor = end;
            continue;
        };
        let media = find_xml_attr_value(tag, b"media-type");
        let properties = find_xml_attr_value(tag, b"properties");
        if !predicate(href, media, properties) {
            cursor = end;
            continue;
        }
        if resolve_opf_href(opf_path, href, out_path) {
            *out_format = to_toc_format(out_path.as_bytes(), media, properties);
            return true;
        }
        cursor = end;
    }

    false
}

fn find_toc_resource_from_opf<const PATH_BYTES: usize>(
    opf: &[u8],
    opf_path: &str,
    out_path: &mut String<PATH_BYTES>,
    out_format: &mut TocFormat,
) -> bool {
    let mut spine_toc_id = String::<ZIP_PATH_BYTES>::new();
    if parse_spine_toc_id(opf, &mut spine_toc_id)
        && let Some((href, media, properties)) = find_manifest_item_by_id(opf, spine_toc_id.as_bytes())
        && resolve_opf_href(opf_path, href, out_path)
    {
        *out_format = to_toc_format(out_path.as_bytes(), media, properties);
        return true;
    }

    if find_manifest_href_by_predicate(
        opf,
        opf_path,
        |_href, _media, properties| {
            properties.is_some_and(|value| contains_ascii_case_insensitive(value, b"nav"))
        },
        out_path,
        out_format,
    ) {
        return true;
    }

    if find_manifest_href_by_predicate(
        opf,
        opf_path,
        |_href, media, _properties| media.is_some_and(media_is_ncx),
        out_path,
        out_format,
    ) {
        return true;
    }

    if find_manifest_href_by_predicate(
        opf,
        opf_path,
        |href, media, _properties| {
            contains_ascii_case_insensitive(href, b"toc")
                || contains_ascii_case_insensitive(href, b"nav")
                || media.is_some_and(media_is_ncx)
                || path_is_ncx(href)
        },
        out_path,
        out_format,
    ) {
        return true;
    }

    false
}

fn split_href_and_fragment(href: &[u8]) -> (&[u8], &[u8]) {
    let href = trim_ascii(href);
    if let Some(hash_pos) = href.iter().position(|b| *b == b'#') {
        let path = &href[..hash_pos];
        let fragment = href.get(hash_pos.saturating_add(1)..).unwrap_or(&[]);
        (path, fragment)
    } else {
        (href, &[])
    }
}

fn decode_fragment_component<const N: usize>(fragment: &[u8], out: &mut String<N>) -> bool {
    out.clear();
    let fragment = trim_ascii(fragment);
    if fragment.is_empty() {
        return false;
    }

    let mut idx = 0usize;
    while idx < fragment.len() {
        let byte = fragment[idx];
        if byte == b'%' && idx + 2 < fragment.len() {
            let hi = fragment[idx + 1];
            let lo = fragment[idx + 2];
            let decoded = match (hex_to_nibble(hi), hex_to_nibble(lo)) {
                (Some(left), Some(right)) => Some((left << 4) | right),
                _ => None,
            };
            if let Some(value) = decoded {
                let ch = if value.is_ascii() {
                    (value as char).to_ascii_lowercase()
                } else {
                    '?'
                };
                if out.push(ch).is_err() {
                    break;
                }
                idx = idx.saturating_add(3);
                continue;
            }
        }

        let normalized = if byte == b'+' {
            ' '
        } else if byte.is_ascii() {
            (byte as char).to_ascii_lowercase()
        } else {
            '?'
        };
        if out.push(normalized).is_err() {
            break;
        }
        idx = idx.saturating_add(1);
    }

    !out.is_empty()
}

fn hex_to_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn resolve_toc_target<const PATH_BYTES: usize>(
    toc_path: &str,
    href: &[u8],
    out_path: &mut String<PATH_BYTES>,
    out_fragment: &mut String<ZIP_PATH_BYTES>,
) -> bool {
    let (path_part, fragment_part) = split_href_and_fragment(href);
    out_fragment.clear();
    let _ = decode_fragment_component(fragment_part, out_fragment);

    if path_part.is_empty() {
        out_path.clear();
        for ch in toc_path.chars() {
            if out_path.push(ch).is_err() {
                return false;
            }
        }
        return !out_path.is_empty();
    }

    resolve_opf_href(toc_path, path_part, out_path)
}

fn rfind_ascii_case_insensitive(haystack: &[u8], needle: &[u8], before: usize) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    let end = before.min(haystack.len());
    if end < needle.len() {
        return None;
    }

    (0..=end - needle.len())
        .rev()
        .find(|&idx| eq_ascii_case_insensitive(&haystack[idx..idx + needle.len()], needle))
}

fn copy_inline_text_without_tags<const N: usize>(source: &[u8], out: &mut String<N>) -> bool {
    out.clear();
    let mut in_tag = false;
    let mut pending_space = false;

    for &byte in source {
        if byte == b'<' {
            in_tag = true;
            continue;
        }
        if byte == b'>' {
            in_tag = false;
            pending_space = true;
            continue;
        }
        if in_tag {
            continue;
        }

        if byte.is_ascii_whitespace() {
            pending_space = true;
            continue;
        }

        if pending_space && !out.is_empty() && out.push(' ').is_err() {
            break;
        }
        pending_space = false;

        let ch = if byte.is_ascii() { byte as char } else { '?' };
        if out.push(ch).is_err() {
            break;
        }
    }

    !out.trim().is_empty()
}

fn extract_ncx_label_near_content<const N: usize>(
    ncx: &[u8],
    content_start: usize,
    out: &mut String<N>,
) -> bool {
    out.clear();
    let window_start = content_start.saturating_sub(ZIP_TOC_LABEL_SEARCH_WINDOW);
    let navpoint_start =
        rfind_ascii_case_insensitive(ncx, b"<navpoint", content_start).unwrap_or(window_start);
    let mut text_cursor = navpoint_start;
    let mut found = false;
    while let Some((start, end)) =
        find_xml_element_bounds_in_range(ncx, b"text", text_cursor, content_start)
    {
        let text_end = ncx[end..content_start]
            .iter()
            .position(|b| *b == b'<')
            .map(|idx| end.saturating_add(idx))
            .unwrap_or(content_start);
        if text_end > end && copy_text_to_string(&ncx[end..text_end], out) {
            found = true;
        }
        text_cursor = start.saturating_add(1).max(end);
    }

    found
}

fn fallback_label_from_target<const N: usize>(path: &str, fragment: &str, out: &mut String<N>) {
    out.clear();

    let source = if !fragment.trim().is_empty() {
        fragment
    } else {
        path.rsplit('/').next().unwrap_or(path)
    };
    let source = source.rsplit_once('.').map(|(left, _)| left).unwrap_or(source);

    let mut word_start = true;
    let mut wrote_any = false;
    for byte in source.as_bytes() {
        let mapped = if *byte == b'_' || *byte == b'-' || *byte == b'.' {
            b' '
        } else {
            *byte
        };

        if mapped == b' ' {
            if !wrote_any || word_start {
                continue;
            }
            if out.push(' ').is_err() {
                break;
            }
            word_start = true;
            continue;
        }

        let ch = if mapped.is_ascii_alphabetic() {
            if word_start {
                (mapped as char).to_ascii_uppercase()
            } else {
                (mapped as char).to_ascii_lowercase()
            }
        } else if mapped.is_ascii_digit() {
            mapped as char
        } else {
            continue;
        };

        if out.push(ch).is_err() {
            break;
        }
        word_start = false;
        wrote_any = true;
    }

    if !wrote_any {
        let _ = out.push_str("Chapter");
    }
}

fn label_looks_like_non_chapter(label: &[u8]) -> bool {
    contains_ascii_case_insensitive(label, b"title page")
        || contains_ascii_case_insensitive(label, b"contents")
        || contains_ascii_case_insensitive(label, b"table of")
        || contains_ascii_case_insensitive(label, b"copyright")
        || contains_ascii_case_insensitive(label, b"license")
        || contains_ascii_case_insensitive(label, b"gutenberg")
        || contains_ascii_case_insensitive(label, b"colophon")
        || contains_ascii_case_insensitive(label, b"about")
        || contains_ascii_case_insensitive(label, b"front matter")
}

fn fragment_looks_like_non_chapter(fragment: &[u8]) -> bool {
    contains_ascii_case_insensitive(fragment, b"pgepubid")
        || contains_ascii_case_insensitive(fragment, b"toc")
        || contains_ascii_case_insensitive(fragment, b"contents")
        || contains_ascii_case_insensitive(fragment, b"footer")
        || contains_ascii_case_insensitive(fragment, b"license")
        || contains_ascii_case_insensitive(fragment, b"copyright")
}

fn toc_entry_is_probable_chapter(path: &str, fragment: &str, label: &str) -> bool {
    if path.trim().is_empty() {
        return false;
    }
    if !is_text_resource_name(path.as_bytes()) {
        return false;
    }
    if path_is_probably_front_matter(path.as_bytes()) {
        return false;
    }
    if fragment_looks_like_non_chapter(fragment.as_bytes()) {
        return false;
    }
    if label_looks_like_non_chapter(label.as_bytes()) {
        return false;
    }
    true
}

fn for_each_ncx_target<F>(toc: &[u8], toc_path: &str, mut on_target: F)
where
    F: FnMut(&str, &str, &str) -> bool,
{
    let mut cursor = 0usize;
    while let Some((start, end)) = find_xml_element_bounds_in_range(toc, b"content", cursor, toc.len())
    {
        let tag = &toc[start..end];
        let Some(src) = find_xml_attr_value(tag, b"src") else {
            cursor = end;
            continue;
        };

        let mut path = String::<ZIP_PATH_BYTES>::new();
        let mut fragment = String::<ZIP_PATH_BYTES>::new();
        if !resolve_toc_target(toc_path, src, &mut path, &mut fragment) {
            cursor = end;
            continue;
        }

        let mut label = String::<SD_CHAPTER_LABEL_BYTES>::new();
        if !extract_ncx_label_near_content(toc, start, &mut label) {
            fallback_label_from_target(path.as_str(), fragment.as_str(), &mut label);
        }
        if label.trim().is_empty() {
            fallback_label_from_target(path.as_str(), fragment.as_str(), &mut label);
        }

        if !on_target(path.as_str(), fragment.as_str(), label.as_str()) {
            return;
        }
        cursor = end;
    }
}

fn for_each_nav_target<F>(toc: &[u8], toc_path: &str, mut on_target: F)
where
    F: FnMut(&str, &str, &str) -> bool,
{
    let mut cursor = 0usize;
    while let Some((start, end)) = find_xml_element_bounds_in_range(toc, b"a", cursor, toc.len()) {
        let tag = &toc[start..end];
        let Some(href) = find_xml_attr_value(tag, b"href") else {
            cursor = end;
            continue;
        };

        let mut path = String::<ZIP_PATH_BYTES>::new();
        let mut fragment = String::<ZIP_PATH_BYTES>::new();
        if !resolve_toc_target(toc_path, href, &mut path, &mut fragment) {
            cursor = end;
            continue;
        }

        let close_anchor = find_ascii_case_insensitive(toc, b"</a", end).unwrap_or(toc.len());
        let mut label = String::<SD_CHAPTER_LABEL_BYTES>::new();
        let label_slice = if close_anchor > end {
            &toc[end..close_anchor]
        } else {
            &toc[end..end]
        };
        if !copy_inline_text_without_tags(label_slice, &mut label) {
            fallback_label_from_target(path.as_str(), fragment.as_str(), &mut label);
        }
        if label.trim().is_empty() {
            fallback_label_from_target(path.as_str(), fragment.as_str(), &mut label);
        }

        if !on_target(path.as_str(), fragment.as_str(), label.as_str()) {
            return;
        }
        cursor = end;
    }
}

fn for_each_toc_target<F>(toc: &[u8], toc_path: &str, format: TocFormat, on_target: F)
where
    F: FnMut(&str, &str, &str) -> bool,
{
    match format {
        TocFormat::Ncx => for_each_ncx_target(toc, toc_path, on_target),
        TocFormat::NavXhtml => for_each_nav_target(toc, toc_path, on_target),
    }
}

fn with_toc_bytes<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
    R,
    F,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    file_size: u32,
    mut on_toc: F,
) -> Result<Option<R>, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
    F: FnMut(&[u8], &str, TocFormat) -> Option<R>,
{
    let Some((opf_entry, opf_path)) =
        find_opf_entry_and_path::<_, _, _, _, _, ZIP_PATH_BYTES>(file, file_size)?
    else {
        return Ok(None);
    };

    let mut opf_buf = [0u8; ZIP_OPF_BYTES];
    let opf_read = read_zip_entry_prefix(file, opf_entry, &mut opf_buf)?;
    if opf_read == 0 {
        return Ok(None);
    }

    let mut toc_path = String::<ZIP_PATH_BYTES>::new();
    let mut toc_format = TocFormat::Ncx;
    if !find_toc_resource_from_opf(
        &opf_buf[..opf_read],
        opf_path.as_str(),
        &mut toc_path,
        &mut toc_format,
    ) {
        return Ok(None);
    }

    let Some(toc_entry) = find_entry_by_path(file, file_size, toc_path.as_bytes())? else {
        return Ok(None);
    };
    let mut toc_buf = [0u8; ZIP_TOC_BYTES];
    let toc_read = read_zip_entry_prefix(file, toc_entry, &mut toc_buf)?;
    if toc_read == 0 {
        return Ok(None);
    }

    Ok(on_toc(&toc_buf[..toc_read], toc_path.as_str(), toc_format))
}

fn chapter_position_for_resource<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    file_size: u32,
    resource_path: &str,
    chapter_label_out: &mut String<SD_CHAPTER_LABEL_BYTES>,
) -> Result<Option<(u16, u16)>, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    chapter_label_out.clear();

    let toc_pos = with_toc_bytes(file, file_size, |toc, toc_path, toc_format| {
        let mut total = 0u16;
        let mut target = None;
        let mut target_label = String::<SD_CHAPTER_LABEL_BYTES>::new();
        let mut last_path = String::<ZIP_PATH_BYTES>::new();
        let mut last_fragment = String::<ZIP_PATH_BYTES>::new();

        for_each_toc_target(toc, toc_path, toc_format, |path, fragment, label| {
            if !toc_entry_is_probable_chapter(path, fragment, label) {
                return true;
            }

            if eq_ascii_case_insensitive(last_path.as_bytes(), path.as_bytes())
                && eq_ascii_case_insensitive(last_fragment.as_bytes(), fragment.as_bytes())
            {
                return true;
            }

            if target.is_none() && eq_ascii_case_insensitive(path.as_bytes(), resource_path.as_bytes()) {
                target = Some(total);
                target_label.clear();
                for ch in label.chars() {
                    if target_label.push(ch).is_err() {
                        break;
                    }
                }
            }

            if total < u16::MAX {
                total = total.saturating_add(1);
            }

            last_path.clear();
            for ch in path.chars() {
                if last_path.push(ch).is_err() {
                    break;
                }
            }
            last_fragment.clear();
            for ch in fragment.chars() {
                if last_fragment.push(ch).is_err() {
                    break;
                }
            }
            true
        });

        target.map(|index| (index, total.max(1), target_label))
    })?;

    if let Some((index, total, label)) = toc_pos {
        chapter_label_out.clear();
        for ch in label.chars() {
            if chapter_label_out.push(ch).is_err() {
                break;
            }
        }
        return Ok(Some((index, total)));
    }
    spine_position_for_resource(file, file_size, resource_path)
}

#[allow(
    clippy::too_many_arguments,
    reason = "targeted chapter lookup returns multiple outputs to avoid heap allocations"
)]
fn find_toc_chapter_entry_at_index<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
    const PATH_BYTES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    file_size: u32,
    target_chapter: u16,
    resource_out: &mut String<PATH_BYTES>,
    fragment_out: &mut String<ZIP_PATH_BYTES>,
    label_out: &mut String<SD_CHAPTER_LABEL_BYTES>,
    chapter_index_out: &mut u16,
    chapter_total_out: &mut u16,
) -> Result<Option<ZipEntryRef>, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let selected = with_toc_bytes(file, file_size, |toc, toc_path, toc_format| {
        let mut total = 0u16;
        let mut selected: Option<TocChapterSelection> = None;
        let mut fallback_last: Option<TocChapterSelection> = None;
        let mut last_path = String::<ZIP_PATH_BYTES>::new();
        let mut last_fragment = String::<ZIP_PATH_BYTES>::new();

        for_each_toc_target(toc, toc_path, toc_format, |path, fragment, label| {
            if !toc_entry_is_probable_chapter(path, fragment, label) {
                return true;
            }

            if eq_ascii_case_insensitive(last_path.as_bytes(), path.as_bytes())
                && eq_ascii_case_insensitive(last_fragment.as_bytes(), fragment.as_bytes())
            {
                return true;
            }

            let mut path_buf = String::<ZIP_PATH_BYTES>::new();
            let mut fragment_buf = String::<ZIP_PATH_BYTES>::new();
            let mut label_buf = String::<SD_CHAPTER_LABEL_BYTES>::new();
            for ch in path.chars() {
                if path_buf.push(ch).is_err() {
                    break;
                }
            }
            for ch in fragment.chars() {
                if fragment_buf.push(ch).is_err() {
                    break;
                }
            }
            for ch in label.chars() {
                if label_buf.push(ch).is_err() {
                    break;
                }
            }

            if total == target_chapter && selected.is_none() {
                selected = Some(TocChapterSelection {
                    path: path_buf.clone(),
                    fragment: fragment_buf.clone(),
                    label: label_buf.clone(),
                    index: total,
                    total: 1,
                });
            }
            fallback_last = Some(TocChapterSelection {
                path: path_buf,
                fragment: fragment_buf,
                label: label_buf,
                index: total,
                total: 1,
            });
            if total < u16::MAX {
                total = total.saturating_add(1);
            }

            last_path.clear();
            for ch in path.chars() {
                if last_path.push(ch).is_err() {
                    break;
                }
            }
            last_fragment.clear();
            for ch in fragment.chars() {
                if last_fragment.push(ch).is_err() {
                    break;
                }
            }
            true
        });

        if total == 0 {
            return None;
        }

        let mut chosen = selected.or(fallback_last)?;
        chosen.total = total.max(1);
        Some(chosen)
    })?;

    let Some(selected) = selected else {
        return Ok(None);
    };

    resource_out.clear();
    for ch in selected.path.chars() {
        if resource_out.push(ch).is_err() {
            break;
        }
    }

    fragment_out.clear();
    for ch in selected.fragment.chars() {
        if fragment_out.push(ch).is_err() {
            break;
        }
    }

    label_out.clear();
    for ch in selected.label.chars() {
        if label_out.push(ch).is_err() {
            break;
        }
    }

    *chapter_index_out = selected.index;
    *chapter_total_out = selected.total.max(1);

    find_entry_by_path(file, file_size, resource_out.as_bytes())
}

#[cfg(all(test, not(target_arch = "xtensa")))]
mod tests {
    extern crate std;

    use super::*;
    use miniz_oxide::inflate::{
        TINFLStatus,
        core::{DecompressorOxide, decompress, inflate_flags},
    };
    use std::{string::String as StdString, vec::Vec as StdVec};

    const BOOK_01_EPUB: &[u8] = include_bytes!("../../../../../tests/fixtures/epub/book_01.epub");

    #[test]
    fn book_01_toc_chapters_match_expected_outline() {
        let opf = extract_zip_entry(BOOK_01_EPUB, b"OEBPS/content.opf")
            .expect("fixture should contain OEBPS/content.opf");
        let mut title = String::<64>::new();
        let _ = parse_opf_metadata(&opf, "OEBPS/content.opf", &mut title);
        assert_eq!(title.as_str(), "The Great Gatsby");

        let mut toc_path = String::<ZIP_PATH_BYTES>::new();
        let mut toc_format = TocFormat::Ncx;
        assert!(find_toc_resource_from_opf(
            &opf,
            "OEBPS/content.opf",
            &mut toc_path,
            &mut toc_format
        ));
        assert_eq!(toc_path.as_str(), "OEBPS/toc.ncx");
        assert_eq!(toc_format, TocFormat::Ncx);

        let toc = extract_zip_entry(BOOK_01_EPUB, toc_path.as_bytes())
            .expect("fixture should contain TOC resource");

        let mut all_targets = 0usize;
        for_each_toc_target(&toc, toc_path.as_str(), toc_format, |_path, _fragment, _label| {
            all_targets = all_targets.saturating_add(1);
            true
        });
        assert!(all_targets > 9);

        let mut chapters: StdVec<(StdString, StdString, StdString)> = StdVec::new();
        let mut last_path = String::<ZIP_PATH_BYTES>::new();
        let mut last_fragment = String::<ZIP_PATH_BYTES>::new();
        for_each_toc_target(&toc, toc_path.as_str(), toc_format, |path, fragment, label| {
            if !toc_entry_is_probable_chapter(path, fragment, label) {
                return true;
            }
            if eq_ascii_case_insensitive(last_path.as_bytes(), path.as_bytes())
                && eq_ascii_case_insensitive(last_fragment.as_bytes(), fragment.as_bytes())
            {
                return true;
            }

            chapters.push((
                StdString::from(path),
                StdString::from(fragment),
                StdString::from(label),
            ));

            last_path.clear();
            for ch in path.chars() {
                if last_path.push(ch).is_err() {
                    break;
                }
            }
            last_fragment.clear();
            for ch in fragment.chars() {
                if last_fragment.push(ch).is_err() {
                    break;
                }
            }
            true
        });

        let labels: StdVec<&str> = chapters.iter().map(|entry| entry.2.as_str()).collect();
        assert_eq!(labels, ["I", "II", "III", "IV", "V", "VI", "VII", "VIII", "IX"]);
        assert_eq!(chapters.len(), 9);
        assert!(chapters[0].0.ends_with("64317-h-0.htm.xhtml"));
        assert!(chapters[4].0.ends_with("64317-h-0.htm.xhtml"));
        assert!(chapters[5].0.ends_with("64317-h-1.htm.xhtml"));
        assert!(chapters[8].0.ends_with("64317-h-1.htm.xhtml"));
    }

    fn extract_zip_entry(archive: &[u8], path: &[u8]) -> Option<StdVec<u8>> {
        let eocd_offset = find_eocd_offset(archive)?;
        let cdir_entries = read_u16_le(archive, eocd_offset + 10) as usize;
        let cdir_offset = read_u32_le(archive, eocd_offset + 16) as usize;

        let mut cursor = cdir_offset;
        for _ in 0..cdir_entries.min(ZIP_MAX_CDIR_ENTRIES) {
            if cursor + ZIP_CDIR_HEADER_BYTES > archive.len() {
                return None;
            }
            if archive[cursor..cursor + 4] != ZIP_CDIR_SIG {
                return None;
            }

            let compression = read_u16_le(archive, cursor + 10);
            let compressed_size = read_u32_le(archive, cursor + 20) as usize;
            let uncompressed_size = read_u32_le(archive, cursor + 24) as usize;
            let name_len = read_u16_le(archive, cursor + 28) as usize;
            let extra_len = read_u16_le(archive, cursor + 30) as usize;
            let comment_len = read_u16_le(archive, cursor + 32) as usize;
            let local_offset = read_u32_le(archive, cursor + 42) as usize;

            let name_start = cursor + ZIP_CDIR_HEADER_BYTES;
            let name_end = name_start + name_len;
            if name_end > archive.len() {
                return None;
            }
            let name = &archive[name_start..name_end];

            let next_cursor = name_end
                .checked_add(extra_len)
                .and_then(|value| value.checked_add(comment_len))?;

            if eq_ascii_case_insensitive(name, path) {
                if local_offset + ZIP_LOCAL_HEADER_BYTES > archive.len() {
                    return None;
                }
                if archive[local_offset..local_offset + 4] != ZIP_LOCAL_SIG {
                    return None;
                }
                let local_name_len = read_u16_le(archive, local_offset + 26) as usize;
                let local_extra_len = read_u16_le(archive, local_offset + 28) as usize;
                let data_start = local_offset
                    .checked_add(ZIP_LOCAL_HEADER_BYTES)?
                    .checked_add(local_name_len)?
                    .checked_add(local_extra_len)?;
                let data_end = data_start.checked_add(compressed_size)?;
                if data_end > archive.len() {
                    return None;
                }
                let compressed = &archive[data_start..data_end];

                return match compression {
                    0 => Some(compressed.to_vec()),
                    8 => inflate_raw_entry(compressed, uncompressed_size),
                    _ => None,
                };
            }

            cursor = next_cursor;
        }

        None
    }

    fn find_eocd_offset(archive: &[u8]) -> Option<usize> {
        if archive.len() < ZIP_EOCD_MIN_BYTES {
            return None;
        }
        (0..=archive.len() - ZIP_EOCD_MIN_BYTES)
            .rev()
            .find(|offset| archive[*offset..].starts_with(&ZIP_EOCD_SIG))
    }

    fn inflate_raw_entry(compressed: &[u8], expected_size: usize) -> Option<StdVec<u8>> {
        let mut inflater = DecompressorOxide::new();
        let mut output = StdVec::new();
        output.resize(expected_size.max(1), 0);

        let mut input_pos = 0usize;
        let mut output_pos = 0usize;
        loop {
            let has_more_input = input_pos < compressed.len();
            let mut flags = inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF;
            if has_more_input {
                flags |= inflate_flags::TINFL_FLAG_HAS_MORE_INPUT;
            }

            let (status, consumed, written) = decompress(
                &mut inflater,
                &compressed[input_pos..],
                output.as_mut_slice(),
                output_pos,
                flags,
            );
            input_pos = input_pos.saturating_add(consumed);
            output_pos = output_pos.saturating_add(written);

            match status {
                TINFLStatus::Done => break,
                TINFLStatus::NeedsMoreInput => {
                    if !has_more_input {
                        return None;
                    }
                }
                TINFLStatus::HasMoreOutput => return None,
                _ => return None,
            }
        }

        output.truncate(output_pos.min(output.len()));
        Some(output)
    }
}
