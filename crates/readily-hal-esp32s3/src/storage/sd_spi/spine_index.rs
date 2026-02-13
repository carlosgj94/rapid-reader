fn parse_spine_first_text_href<const PATH_BYTES: usize>(
    opf: &[u8],
    opf_path: &str,
    out: &mut String<PATH_BYTES>,
) -> bool {
    let mut cursor = 0usize;
    let mut manifest_cursor_hint = 0usize;
    while let Some((start, end)) =
        find_xml_element_bounds_in_range(opf, b"itemref", cursor, opf.len())
    {
        let tag = &opf[start..end];
        let Some(idref) = find_xml_attr_value(tag, b"idref") else {
            cursor = end;
            continue;
        };
        let linear = find_xml_attr_value(tag, b"linear");
        if linear.is_some_and(|value| eq_ascii_case_insensitive(trim_ascii(value), b"no")) {
            cursor = end;
            continue;
        }

        let Some((href, media, properties)) =
            find_manifest_item_by_id_with_hint(opf, idref, &mut manifest_cursor_hint)
        else {
            cursor = end;
            continue;
        };
        if !media.is_some_and(is_text_media_type) {
            cursor = end;
            continue;
        }

        let mut resolved = String::<PATH_BYTES>::new();
        if !resolve_opf_href(opf_path, href, &mut resolved) {
            cursor = end;
            continue;
        }

        let skip_by_properties =
            properties.is_some_and(|prop| contains_ascii_case_insensitive(prop, b"nav"));
        if skip_by_properties
            || path_is_probably_front_matter(idref)
            || path_is_probably_front_matter(href)
            || path_is_probably_front_matter(resolved.as_bytes())
        {
            cursor = end;
            continue;
        }

        *out = resolved;
        return true;
    }

    false
}

fn parse_spine_next_text_href<const PATH_BYTES: usize>(
    opf: &[u8],
    opf_path: &str,
    current_path: &str,
    out: &mut String<PATH_BYTES>,
) -> bool {
    if current_path.is_empty() {
        return false;
    }

    let mut cursor = 0usize;
    let mut manifest_cursor_hint = 0usize;
    let mut seen_current = false;
    while let Some((start, end)) =
        find_xml_element_bounds_in_range(opf, b"itemref", cursor, opf.len())
    {
        let tag = &opf[start..end];
        let Some(idref) = find_xml_attr_value(tag, b"idref") else {
            cursor = end;
            continue;
        };
        let linear = find_xml_attr_value(tag, b"linear");
        if linear.is_some_and(|value| eq_ascii_case_insensitive(trim_ascii(value), b"no")) {
            cursor = end;
            continue;
        }

        let Some((href, media, properties)) =
            find_manifest_item_by_id_with_hint(opf, idref, &mut manifest_cursor_hint)
        else {
            cursor = end;
            continue;
        };
        if !media.is_some_and(is_text_media_type) {
            cursor = end;
            continue;
        }

        let mut resolved = String::<PATH_BYTES>::new();
        if !resolve_opf_href(opf_path, href, &mut resolved) {
            cursor = end;
            continue;
        }

        if properties.is_some_and(|prop| contains_ascii_case_insensitive(prop, b"nav")) {
            cursor = end;
            continue;
        }

        if !seen_current {
            if eq_ascii_case_insensitive(resolved.as_bytes(), current_path.as_bytes()) {
                seen_current = true;
            }
            cursor = end;
            continue;
        }

        *out = resolved;
        return true;
    }

    false
}

fn parse_spine_text_href_at_with_filter<const PATH_BYTES: usize>(
    opf: &[u8],
    opf_path: &str,
    target_index: u16,
    skip_front_matter: bool,
    out: &mut String<PATH_BYTES>,
    out_index: &mut u16,
    out_total: &mut u16,
) -> bool {
    let mut cursor = 0usize;
    let mut manifest_cursor_hint = 0usize;
    let mut total = 0u16;
    let mut selected = String::<PATH_BYTES>::new();
    let mut selected_index = 0u16;
    let mut found = false;
    let mut last = String::<PATH_BYTES>::new();
    let mut last_index = 0u16;
    let mut have_last = false;

    while let Some((start, end)) =
        find_xml_element_bounds_in_range(opf, b"itemref", cursor, opf.len())
    {
        let tag = &opf[start..end];
        let Some(idref) = find_xml_attr_value(tag, b"idref") else {
            cursor = end;
            continue;
        };
        let linear = find_xml_attr_value(tag, b"linear");
        if linear.is_some_and(|value| eq_ascii_case_insensitive(trim_ascii(value), b"no")) {
            cursor = end;
            continue;
        }

        let Some((href, media, properties)) =
            find_manifest_item_by_id_with_hint(opf, idref, &mut manifest_cursor_hint)
        else {
            cursor = end;
            continue;
        };
        if !media.is_some_and(is_text_media_type) {
            cursor = end;
            continue;
        }
        if properties.is_some_and(|prop| contains_ascii_case_insensitive(prop, b"nav")) {
            cursor = end;
            continue;
        }

        let mut resolved = String::<PATH_BYTES>::new();
        if !resolve_opf_href(opf_path, href, &mut resolved) {
            cursor = end;
            continue;
        }

        if skip_front_matter
            && (path_is_probably_front_matter(idref)
                || path_is_probably_front_matter(href)
                || path_is_probably_front_matter(resolved.as_bytes()))
        {
            cursor = end;
            continue;
        }

        if total == target_index && !found {
            selected = resolved.clone();
            selected_index = total;
            found = true;
        }
        last = resolved;
        last_index = total;
        have_last = true;
        total = total.saturating_add(1);
        cursor = end;
    }

    if total == 0 {
        return false;
    }

    if !found {
        if !have_last {
            return false;
        }
        selected = last;
        selected_index = last_index;
    }

    *out = selected;
    *out_index = selected_index;
    *out_total = total.max(1);
    true
}

fn parse_spine_text_href_at<const PATH_BYTES: usize>(
    opf: &[u8],
    opf_path: &str,
    target_index: u16,
    out: &mut String<PATH_BYTES>,
    out_index: &mut u16,
    out_total: &mut u16,
) -> bool {
    parse_spine_text_href_at_with_filter(
        opf,
        opf_path,
        target_index,
        true,
        out,
        out_index,
        out_total,
    ) || parse_spine_text_href_at_with_filter(
        opf,
        opf_path,
        target_index,
        false,
        out,
        out_index,
        out_total,
    )
}

fn parse_spine_position_for_path_with_filter(
    opf: &[u8],
    opf_path: &str,
    target_path: &str,
    skip_front_matter: bool,
) -> Option<(u16, u16)> {
    if target_path.is_empty() {
        return None;
    }

    let mut cursor = 0usize;
    let mut manifest_cursor_hint = 0usize;
    let mut total = 0u16;
    let mut target_index = None;
    while let Some((start, end)) =
        find_xml_element_bounds_in_range(opf, b"itemref", cursor, opf.len())
    {
        let tag = &opf[start..end];
        let Some(idref) = find_xml_attr_value(tag, b"idref") else {
            cursor = end;
            continue;
        };
        let linear = find_xml_attr_value(tag, b"linear");
        if linear.is_some_and(|value| eq_ascii_case_insensitive(trim_ascii(value), b"no")) {
            cursor = end;
            continue;
        }

        let Some((href, media, properties)) =
            find_manifest_item_by_id_with_hint(opf, idref, &mut manifest_cursor_hint)
        else {
            cursor = end;
            continue;
        };
        if !media.is_some_and(is_text_media_type) {
            cursor = end;
            continue;
        }
        if properties.is_some_and(|prop| contains_ascii_case_insensitive(prop, b"nav")) {
            cursor = end;
            continue;
        }

        let mut resolved = String::<ZIP_PATH_BYTES>::new();
        if !resolve_opf_href(opf_path, href, &mut resolved) {
            cursor = end;
            continue;
        }

        if skip_front_matter
            && (path_is_probably_front_matter(idref)
                || path_is_probably_front_matter(href)
                || path_is_probably_front_matter(resolved.as_bytes()))
        {
            cursor = end;
            continue;
        }

        if total < u16::MAX {
            if eq_ascii_case_insensitive(resolved.as_bytes(), target_path.as_bytes()) {
                target_index = Some(total);
            }
            total = total.saturating_add(1);
        }
        cursor = end;
    }

    target_index.map(|index| (index, total.max(1)))
}

fn parse_spine_position_for_path(
    opf: &[u8],
    opf_path: &str,
    target_path: &str,
) -> Option<(u16, u16)> {
    parse_spine_position_for_path_with_filter(opf, opf_path, target_path, true)
        .or_else(|| parse_spine_position_for_path_with_filter(opf, opf_path, target_path, false))
}

fn spine_position_for_resource<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    file_size: u32,
    resource_path: &str,
) -> Result<Option<(u16, u16)>, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let mut opf_buf = [0u8; ZIP_OPF_BYTES];
    let Some((opf_entry, opf_path)) =
        find_opf_entry_and_path::<_, _, _, _, _, ZIP_PATH_BYTES>(file, file_size)?
    else {
        return cdir_position_for_resource(file, file_size, resource_path);
    };
    let opf_read = read_zip_entry_prefix(file, opf_entry, &mut opf_buf)?;
    if opf_read == 0 {
        return cdir_position_for_resource(file, file_size, resource_path);
    }

    if let Some(spine_pos) =
        parse_spine_position_for_path(&opf_buf[..opf_read], opf_path.as_str(), resource_path)
    {
        return Ok(Some(spine_pos));
    }

    cdir_position_for_resource(file, file_size, resource_path)
}

fn cdir_position_for_resource_with_filter<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    file_size: u32,
    resource_path: &str,
    skip_front_matter: bool,
) -> Result<Option<(u16, u16)>, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    if resource_path.is_empty() {
        return Ok(None);
    }

    let Some((cdir_offset, cdir_entries)) = cdir_info(file, file_size)? else {
        return Ok(None);
    };

    let mut cdir_header = [0u8; ZIP_CDIR_HEADER_BYTES];
    let mut cdir_name = [0u8; ZIP_NAME_BYTES];
    let mut cdir_cursor = cdir_offset;
    let mut total = 0u16;
    let mut target = None;

    for _ in 0..cdir_entries.min(ZIP_MAX_CDIR_ENTRIES) {
        let header_read = read_file_at(file, cdir_cursor, &mut cdir_header)?;
        if header_read < ZIP_CDIR_HEADER_BYTES || !cdir_header.starts_with(&ZIP_CDIR_SIG) {
            break;
        }

        let name_len = read_u16_le(&cdir_header, 28) as usize;
        let extra_len = read_u16_le(&cdir_header, 30) as usize;
        let comment_len = read_u16_le(&cdir_header, 32) as usize;
        let Some(next_cursor) = cdir_cursor
            .checked_add(ZIP_CDIR_HEADER_BYTES as u32)
            .and_then(|value| value.checked_add(name_len as u32))
            .and_then(|value| value.checked_add(extra_len as u32))
            .and_then(|value| value.checked_add(comment_len as u32))
        else {
            break;
        };

        let name_read_len = name_len.min(cdir_name.len());
        if name_read_len == 0 {
            cdir_cursor = next_cursor;
            continue;
        }

        let name_read = read_file_at(
            file,
            cdir_cursor.saturating_add(ZIP_CDIR_HEADER_BYTES as u32),
            &mut cdir_name[..name_read_len],
        )?;
        if name_read < name_read_len {
            break;
        }

        let name_complete = name_len <= cdir_name.len();
        let name_slice = &cdir_name[..name_read_len];
        if name_complete && is_text_resource_name(name_slice) {
            if skip_front_matter && path_is_probably_front_matter(name_slice) {
                cdir_cursor = next_cursor;
                continue;
            }

            if total < u16::MAX {
                if eq_ascii_case_insensitive(name_slice, resource_path.as_bytes()) {
                    target = Some(total);
                }
                total = total.saturating_add(1);
            }
        }

        cdir_cursor = next_cursor;
    }

    Ok(target.map(|index| (index, total.max(1))))
}

fn cdir_position_for_resource<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    file_size: u32,
    resource_path: &str,
) -> Result<Option<(u16, u16)>, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let with_filter = cdir_position_for_resource_with_filter(file, file_size, resource_path, true)?;
    if with_filter.is_some() {
        return Ok(with_filter);
    }
    cdir_position_for_resource_with_filter(file, file_size, resource_path, false)
}

fn cdir_text_entry_at_with_filter<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
    const PATH_BYTES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    file_size: u32,
    target_index: u16,
    skip_front_matter: bool,
) -> Result<Option<ChapterEntry<PATH_BYTES>>, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let Some((cdir_offset, cdir_entries)) = cdir_info(file, file_size)? else {
        return Ok(None);
    };

    let mut cdir_header = [0u8; ZIP_CDIR_HEADER_BYTES];
    let mut cdir_name = [0u8; ZIP_NAME_BYTES];
    let mut cdir_cursor = cdir_offset;
    let mut total = 0u16;
    let mut selected: Option<(ZipEntryRef, String<PATH_BYTES>, u16)> = None;
    let mut fallback_last: Option<(ZipEntryRef, String<PATH_BYTES>, u16)> = None;

    for _ in 0..cdir_entries.min(ZIP_MAX_CDIR_ENTRIES) {
        let header_read = read_file_at(file, cdir_cursor, &mut cdir_header)?;
        if header_read < ZIP_CDIR_HEADER_BYTES || !cdir_header.starts_with(&ZIP_CDIR_SIG) {
            break;
        }

        let compression = read_u16_le(&cdir_header, 10);
        let compressed_size = read_u32_le(&cdir_header, 20);
        let uncompressed_size = read_u32_le(&cdir_header, 24);
        let name_len = read_u16_le(&cdir_header, 28) as usize;
        let extra_len = read_u16_le(&cdir_header, 30) as usize;
        let comment_len = read_u16_le(&cdir_header, 32) as usize;
        let local_header_offset = read_u32_le(&cdir_header, 42);

        let Some(next_cursor) = cdir_cursor
            .checked_add(ZIP_CDIR_HEADER_BYTES as u32)
            .and_then(|value| value.checked_add(name_len as u32))
            .and_then(|value| value.checked_add(extra_len as u32))
            .and_then(|value| value.checked_add(comment_len as u32))
        else {
            break;
        };

        let name_read_len = name_len.min(cdir_name.len());
        if name_read_len == 0 {
            cdir_cursor = next_cursor;
            continue;
        }

        let name_read = read_file_at(
            file,
            cdir_cursor.saturating_add(ZIP_CDIR_HEADER_BYTES as u32),
            &mut cdir_name[..name_read_len],
        )?;
        if name_read < name_read_len {
            break;
        }

        let name_complete = name_len <= cdir_name.len();
        let name_slice = &cdir_name[..name_read_len];
        if name_complete && is_text_resource_name(name_slice) {
            if skip_front_matter && path_is_probably_front_matter(name_slice) {
                cdir_cursor = next_cursor;
                continue;
            }

            let entry_ref = ZipEntryRef {
                compression,
                compressed_size,
                uncompressed_size,
                local_header_offset,
            };
            let mut resource = String::<PATH_BYTES>::new();
            copy_ascii_or_lossy(name_slice, &mut resource);
            if total == target_index && selected.is_none() {
                selected = Some((entry_ref, resource.clone(), total));
            }
            fallback_last = Some((entry_ref, resource, total));
            total = total.saturating_add(1);
        }

        cdir_cursor = next_cursor;
    }

    if total == 0 {
        return Ok(None);
    }

    if let Some((entry, resource, index)) = selected {
        return Ok(Some((entry, resource, index, total.max(1))));
    }
    if let Some((entry, resource, index)) = fallback_last {
        return Ok(Some((entry, resource, index, total.max(1))));
    }
    Ok(None)
}

fn cdir_text_entry_at<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
    const PATH_BYTES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    file_size: u32,
    target_index: u16,
) -> Result<Option<ChapterEntry<PATH_BYTES>>, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let with_filter = cdir_text_entry_at_with_filter(file, file_size, target_index, true)?;
    if with_filter.is_some() {
        return Ok(with_filter);
    }
    cdir_text_entry_at_with_filter(file, file_size, target_index, false)
}

