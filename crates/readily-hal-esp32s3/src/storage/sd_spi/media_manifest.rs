type ManifestItemMatch<'a> = (&'a [u8], Option<&'a [u8]>, Option<&'a [u8]>);

fn find_manifest_item_by_id_in_range<'a>(
    opf: &'a [u8],
    idref: &[u8],
    start_cursor: usize,
    end_cursor: usize,
) -> Option<(usize, ManifestItemMatch<'a>)> {
    let mut cursor = start_cursor.min(opf.len());
    let limit = end_cursor.min(opf.len());
    while let Some((start, end)) = find_xml_element_bounds_in_range(opf, b"item", cursor, limit) {
        let tag = &opf[start..end];
        let Some(item_id) = find_xml_attr_value(tag, b"id") else {
            cursor = end;
            continue;
        };
        if !eq_ascii_case_insensitive(item_id, idref) {
            cursor = end;
            continue;
        }

        let href = find_xml_attr_value(tag, b"href")?;
        let media = find_xml_attr_value(tag, b"media-type");
        let properties = find_xml_attr_value(tag, b"properties");
        return Some((end, (href, media, properties)));
    }

    None
}

fn find_manifest_item_by_id_with_hint<'a>(
    opf: &'a [u8],
    idref: &[u8],
    cursor_hint: &mut usize,
) -> Option<ManifestItemMatch<'a>> {
    let hint = (*cursor_hint).min(opf.len());
    if let Some((next, matched)) = find_manifest_item_by_id_in_range(opf, idref, hint, opf.len()) {
        *cursor_hint = next;
        return Some(matched);
    }

    if hint > 0
        && let Some((next, matched)) = find_manifest_item_by_id_in_range(opf, idref, 0, hint)
    {
        *cursor_hint = next;
        return Some(matched);
    }

    None
}

fn find_manifest_item_by_id<'a>(opf: &'a [u8], idref: &[u8]) -> Option<ManifestItemMatch<'a>> {
    let mut cursor_hint = 0usize;
    find_manifest_item_by_id_with_hint(opf, idref, &mut cursor_hint)
}

fn is_image_media_type(media: &[u8]) -> bool {
    contains_ascii_case_insensitive(media, b"image/")
}

fn strip_resource_suffix(path: &[u8]) -> &[u8] {
    let mut end = path.len();
    for (idx, &byte) in path.iter().enumerate() {
        if byte == b'?' || byte == b'#' {
            end = idx;
            break;
        }
    }
    &path[..end]
}

fn is_cover_media_pbm(media: &[u8], path: &[u8]) -> bool {
    let path = strip_resource_suffix(path);
    contains_ascii_case_insensitive(media, b"image/x-portable-bitmap")
        || contains_ascii_case_insensitive(media, b"image/pbm")
        || ends_with_ascii_case_insensitive(path, b".pbm")
}

fn is_cover_media_png(media: &[u8], path: &[u8]) -> bool {
    let path = strip_resource_suffix(path);
    contains_ascii_case_insensitive(media, b"image/png")
        || ends_with_ascii_case_insensitive(path, b".png")
}

fn is_cover_media_jpeg(media: &[u8], path: &[u8]) -> bool {
    let path = strip_resource_suffix(path);
    contains_ascii_case_insensitive(media, b"image/jpeg")
        || contains_ascii_case_insensitive(media, b"image/jpg")
        || ends_with_ascii_case_insensitive(path, b".jpg")
        || ends_with_ascii_case_insensitive(path, b".jpeg")
        || ends_with_ascii_case_insensitive(path, b".jpe")
        || ends_with_ascii_case_insensitive(path, b".jfif")
}

fn is_cover_like_image_path(path: &[u8]) -> bool {
    contains_ascii_case_insensitive(path, b"cover")
        || contains_ascii_case_insensitive(path, b"portada")
        || contains_ascii_case_insensitive(path, b"front")
        || contains_ascii_case_insensitive(path, b"titlepage")
        || contains_ascii_case_insensitive(path, b"jacket")
}

fn is_image_resource_name(path: &[u8]) -> bool {
    let path = strip_resource_suffix(path);
    ends_with_ascii_case_insensitive(path, b".pbm")
        || ends_with_ascii_case_insensitive(path, b".png")
        || ends_with_ascii_case_insensitive(path, b".jpg")
        || ends_with_ascii_case_insensitive(path, b".jpeg")
        || ends_with_ascii_case_insensitive(path, b".jpe")
        || ends_with_ascii_case_insensitive(path, b".jfif")
        || ends_with_ascii_case_insensitive(path, b".gif")
        || ends_with_ascii_case_insensitive(path, b".webp")
        || ends_with_ascii_case_insensitive(path, b".svg")
        || ends_with_ascii_case_insensitive(path, b".bmp")
        || ends_with_ascii_case_insensitive(path, b".tif")
        || ends_with_ascii_case_insensitive(path, b".tiff")
}

fn is_probable_image_resource_path(path: &[u8]) -> bool {
    let path = strip_resource_suffix(path);
    contains_ascii_case_insensitive(path, b"/images/")
        || contains_ascii_case_insensitive(path, b"/image/")
        || contains_ascii_case_insensitive(path, b"/img/")
        || contains_ascii_case_insensitive(path, b"illustration")
        || contains_ascii_case_insensitive(path, b"artwork")
        || is_cover_like_image_path(path)
}

fn sniff_cover_media_flags(bytes: &[u8]) -> (bool, bool, bool) {
    let png = bytes.starts_with(&[137, 80, 78, 71, 13, 10, 26, 10]);
    let jpeg = bytes.starts_with(&[0xFF, 0xD8]);

    let mut idx = 0usize;
    while idx < bytes.len() && bytes[idx].is_ascii_whitespace() {
        idx = idx.saturating_add(1);
    }
    let pbm = idx + 1 < bytes.len() && bytes[idx] == b'P' && bytes[idx + 1] == b'4';

    (pbm, png, jpeg)
}

fn sniff_cover_media_from_entry<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    entry: ZipEntryRef,
) -> Result<Option<&'static str>, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let mut sniff = [0u8; 16];
    let sniff_read = read_zip_entry_prefix(file, entry, &mut sniff)?;
    if sniff_read == 0 {
        return Ok(None);
    }

    let (pbm, png, jpeg) = sniff_cover_media_flags(&sniff[..sniff_read]);
    if pbm {
        return Ok(Some("image/x-portable-bitmap"));
    }
    if png {
        return Ok(Some("image/png"));
    }
    if jpeg {
        return Ok(Some("image/jpeg"));
    }

    Ok(None)
}

fn is_text_media_document(media: &[u8], path: &[u8]) -> bool {
    is_text_media_type(media)
        || ends_with_ascii_case_insensitive(path, b".xhtml")
        || ends_with_ascii_case_insensitive(path, b".html")
}

fn copy_media_type_or_default<const MEDIA_BYTES: usize>(
    media: Option<&[u8]>,
    path: &[u8],
    out: &mut String<MEDIA_BYTES>,
) {
    let path = strip_resource_suffix(path);
    out.clear();
    if media.is_some_and(|value| copy_utf8_or_ascii(value, out)) {
        return;
    }

    let inferred = if ends_with_ascii_case_insensitive(path, b".pbm") {
        "image/x-portable-bitmap"
    } else if ends_with_ascii_case_insensitive(path, b".png") {
        "image/png"
    } else if ends_with_ascii_case_insensitive(path, b".jpg")
        || ends_with_ascii_case_insensitive(path, b".jpeg")
        || ends_with_ascii_case_insensitive(path, b".jpe")
        || ends_with_ascii_case_insensitive(path, b".jfif")
    {
        "image/jpeg"
    } else if ends_with_ascii_case_insensitive(path, b".gif") {
        "image/gif"
    } else if ends_with_ascii_case_insensitive(path, b".webp") {
        "image/webp"
    } else if ends_with_ascii_case_insensitive(path, b".svg") {
        "image/svg+xml"
    } else if ends_with_ascii_case_insensitive(path, b".bmp") {
        "image/bmp"
    } else if ends_with_ascii_case_insensitive(path, b".tif")
        || ends_with_ascii_case_insensitive(path, b".tiff")
    {
        "image/tiff"
    } else if ends_with_ascii_case_insensitive(path, b".xhtml")
        || ends_with_ascii_case_insensitive(path, b".html")
    {
        "application/xhtml+xml"
    } else {
        ""
    };

    if !inferred.is_empty() {
        let _ = out.push_str(inferred);
    }
}

fn parse_meta_cover_id<const ID_BYTES: usize>(opf: &[u8], out: &mut String<ID_BYTES>) -> bool {
    let mut cursor = 0usize;
    while let Some((start, end)) = find_xml_element_bounds_in_range(opf, b"meta", cursor, opf.len())
    {
        let tag = &opf[start..end];
        let name = find_xml_attr_value(tag, b"name");
        let content = find_xml_attr_value(tag, b"content");
        if let (Some(name), Some(content)) = (name, content)
            && eq_ascii_case_insensitive(trim_ascii(name), b"cover")
            && copy_utf8_or_ascii(content, out)
        {
            return true;
        }

        cursor = end;
    }

    false
}

fn parse_manifest_image_href_with_filter<const PATH_BYTES: usize, const MEDIA_BYTES: usize, F>(
    opf: &[u8],
    opf_path: &str,
    mut matches: F,
    out_path: &mut String<PATH_BYTES>,
    out_media: &mut String<MEDIA_BYTES>,
) -> bool
where
    F: FnMut(&[u8], &[u8], Option<&[u8]>, Option<&[u8]>) -> bool,
{
    let mut cursor = 0usize;
    while let Some((start, end)) = find_xml_element_bounds_in_range(opf, b"item", cursor, opf.len())
    {
        let tag = &opf[start..end];
        let id = find_xml_attr_value(tag, b"id").unwrap_or(b"");
        let Some(href) = find_xml_attr_value(tag, b"href") else {
            cursor = end;
            continue;
        };
        let media = find_xml_attr_value(tag, b"media-type");
        let properties = find_xml_attr_value(tag, b"properties");
        let href_looks_image =
            is_image_resource_name(href) || is_probable_image_resource_path(href);
        if !media.is_some_and(is_image_media_type) && !href_looks_image {
            cursor = end;
            continue;
        }
        if !matches(id, href, media, properties) {
            cursor = end;
            continue;
        }

        if !resolve_opf_href(opf_path, href, out_path) {
            cursor = end;
            continue;
        }
        copy_media_type_or_default(media, out_path.as_bytes(), out_media);
        return true;
    }

    false
}

fn parse_opf_cover_resource<const PATH_BYTES: usize, const MEDIA_BYTES: usize>(
    opf: &[u8],
    opf_path: &str,
    out_path: &mut String<PATH_BYTES>,
    out_media: &mut String<MEDIA_BYTES>,
) -> bool {
    out_path.clear();
    out_media.clear();

    let mut cover_id = String::<ZIP_PATH_BYTES>::new();
    if parse_meta_cover_id(opf, &mut cover_id)
        && let Some((href, media, _properties)) = find_manifest_item_by_id(opf, cover_id.as_bytes())
        && resolve_opf_href(opf_path, href, out_path)
    {
        copy_media_type_or_default(media, out_path.as_bytes(), out_media);
        return true;
    }

    if parse_manifest_image_href_with_filter(
        opf,
        opf_path,
        |_id, _href, _media, properties| {
            properties.is_some_and(|value| contains_ascii_case_insensitive(value, b"cover-image"))
        },
        out_path,
        out_media,
    ) {
        return true;
    }

    if parse_manifest_image_href_with_filter(
        opf,
        opf_path,
        |id, _href, _media, _properties| contains_ascii_case_insensitive(id, b"cover"),
        out_path,
        out_media,
    ) {
        return true;
    }

    if parse_manifest_image_href_with_filter(
        opf,
        opf_path,
        |_id, href, _media, _properties| contains_ascii_case_insensitive(href, b"cover"),
        out_path,
        out_media,
    ) {
        return true;
    }

    parse_manifest_image_href_with_filter(
        opf,
        opf_path,
        |_id, _href, _media, _properties| true,
        out_path,
        out_media,
    )
}

fn parse_html_first_img_src<const PATH_BYTES: usize>(
    html: &[u8],
    html_path: &str,
    out: &mut String<PATH_BYTES>,
) -> bool {
    let mut cursor = 0usize;
    while let Some(start) = find_ascii_case_insensitive(html, b"<img", cursor) {
        let rel_end = html[start..]
            .iter()
            .position(|b| *b == b'>')
            .map(|idx| start.saturating_add(idx).saturating_add(1));
        let Some(end) = rel_end else {
            break;
        };
        let tag = &html[start..end];
        if let Some(src) = find_xml_attr_value(tag, b"src")
            && resolve_opf_href(html_path, src, out)
        {
            return true;
        }
        cursor = end;
    }

    false
}

