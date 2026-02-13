fn cdir_info<D, T, const MAX_DIRS: usize, const MAX_FILES: usize, const MAX_VOLUMES: usize>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    file_size: u32,
) -> Result<Option<(u32, usize)>, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    if file_size < ZIP_EOCD_MIN_BYTES as u32 {
        return Ok(None);
    }

    let tail_len = (file_size as usize).min(ZIP_EOCD_SEARCH_WINDOW);
    let tail_start = file_size.saturating_sub(tail_len as u32);
    let mut tail = [0u8; ZIP_EOCD_SEARCH_WINDOW];
    let tail_read = read_file_at(file, tail_start, &mut tail[..tail_len])?;
    if tail_read < ZIP_EOCD_MIN_BYTES {
        return Ok(None);
    }

    let Some(eocd_pos) = (0..=tail_read.saturating_sub(4))
        .rev()
        .find(|idx| tail[*idx..].starts_with(&ZIP_EOCD_SIG))
    else {
        return Ok(None);
    };
    if eocd_pos + ZIP_EOCD_MIN_BYTES > tail_read {
        return Ok(None);
    }

    let cdir_offset = read_u32_le(&tail, eocd_pos + 16);
    let cdir_entries = read_u16_le(&tail, eocd_pos + 10) as usize;
    if cdir_offset >= file_size {
        return Ok(None);
    }

    Ok(Some((cdir_offset, cdir_entries)))
}

fn find_opf_entry_and_path<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
    const PATH_BYTES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    file_size: u32,
) -> Result<Option<(ZipEntryRef, String<PATH_BYTES>)>, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let Some((cdir_offset, cdir_entries)) = cdir_info(file, file_size)? else {
        return Ok(None);
    };

    let mut cdir_header = [0u8; ZIP_CDIR_HEADER_BYTES];
    let mut cdir_name = [0u8; ZIP_NAME_BYTES];
    let mut container_buf = [0u8; ZIP_CONTAINER_BYTES];
    let mut opf_path_from_container = String::<ZIP_PATH_BYTES>::new();

    let mut selected: Option<(ZipEntryRef, String<PATH_BYTES>)> = None;
    let mut fallback: Option<(ZipEntryRef, String<PATH_BYTES>)> = None;

    let mut cdir_cursor = cdir_offset;
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
        let name_slice = &cdir_name[..name_read_len];
        let name_complete = name_len <= cdir_name.len();
        if !name_complete {
            cdir_cursor = next_cursor;
            continue;
        }

        let entry = ZipEntryRef {
            compression,
            compressed_size,
            uncompressed_size,
            local_header_offset,
        };
        let mut name_text = String::<PATH_BYTES>::new();
        copy_ascii_or_lossy(name_slice, &mut name_text);

        if opf_path_from_container.is_empty()
            && eq_ascii_case_insensitive(name_slice, b"META-INF/container.xml")
        {
            let read_now = read_zip_entry_prefix(file, entry, &mut container_buf)?;
            if read_now > 0 {
                let _ = parse_container_full_path(
                    &container_buf[..read_now],
                    &mut opf_path_from_container,
                );
            }
        }

        if selected.is_none()
            && !opf_path_from_container.is_empty()
            && eq_ascii_case_insensitive(name_slice, opf_path_from_container.as_bytes())
        {
            selected = Some((entry, name_text.clone()));
        }

        if fallback.is_none() && ends_with_ascii_case_insensitive(name_slice, b".opf") {
            fallback = Some((entry, name_text));
        }

        cdir_cursor = next_cursor;
    }

    Ok(selected.or(fallback))
}

fn find_entry_by_path<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    file_size: u32,
    path: &[u8],
) -> Result<Option<ZipEntryRef>, embedded_sdmmc::Error<D::Error>>
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
        if name_read_len == path.len() {
            let name_read = read_file_at(
                file,
                cdir_cursor.saturating_add(ZIP_CDIR_HEADER_BYTES as u32),
                &mut cdir_name[..name_read_len],
            )?;
            if name_read == name_read_len
                && eq_ascii_case_insensitive(&cdir_name[..name_read_len], path)
            {
                return Ok(Some(ZipEntryRef {
                    compression,
                    compressed_size,
                    uncompressed_size,
                    local_header_offset,
                }));
            }
        }

        cdir_cursor = next_cursor;
    }

    Ok(None)
}

fn find_fallback_cover_image_entry<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
    const PATH_BYTES: usize,
    const MEDIA_BYTES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    file_size: u32,
    exclude_path: Option<&[u8]>,
) -> CoverFallbackEntryResult<D::Error, PATH_BYTES, MEDIA_BYTES>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    find_fallback_cover_image_entry_with_skip::<
        D,
        T,
        MAX_DIRS,
        MAX_FILES,
        MAX_VOLUMES,
        PATH_BYTES,
        MEDIA_BYTES,
    >(file, file_size, exclude_path, 0)
}

fn find_fallback_cover_image_entry_with_skip<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
    const PATH_BYTES: usize,
    const MEDIA_BYTES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    file_size: u32,
    exclude_path: Option<&[u8]>,
    skip_count: usize,
) -> CoverFallbackEntryResult<D::Error, PATH_BYTES, MEDIA_BYTES>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let Some((cdir_offset, cdir_entries)) = cdir_info(file, file_size)? else {
        return Ok(None);
    };

    let mut cover_like_total = 0usize;
    let mut cdir_header = [0u8; ZIP_CDIR_HEADER_BYTES];
    let mut cdir_name = [0u8; ZIP_NAME_BYTES];
    let mut cdir_cursor = cdir_offset;

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
        if name_read_len > 0 {
            let name_read = read_file_at(
                file,
                cdir_cursor.saturating_add(ZIP_CDIR_HEADER_BYTES as u32),
                &mut cdir_name[..name_read_len],
            )?;
            if name_read == name_read_len {
                let name_slice = &cdir_name[..name_read_len];
                let is_dir = name_slice.last().copied() == Some(b'/');
                if !is_dir {
                    if exclude_path
                        .is_some_and(|excluded| eq_ascii_case_insensitive(excluded, name_slice))
                    {
                        cdir_cursor = next_cursor;
                        continue;
                    }

                    let entry = ZipEntryRef {
                        compression,
                        compressed_size,
                        uncompressed_size,
                        local_header_offset,
                    };
                    let mut is_image = is_image_resource_name(name_slice)
                        || is_probable_image_resource_path(name_slice);
                    if !is_image {
                        is_image = sniff_cover_media_from_entry(file, entry)?.is_some();
                    }

                    if is_image && is_cover_like_image_path(name_slice) {
                        cover_like_total = cover_like_total.saturating_add(1);
                    }
                }
            }
        }

        cdir_cursor = next_cursor;
    }

    let target_cover_like = if skip_count < cover_like_total {
        Some(skip_count)
    } else {
        None
    };
    let target_other = if skip_count >= cover_like_total {
        Some(skip_count - cover_like_total)
    } else {
        None
    };

    let mut seen_cover_like = 0usize;
    let mut seen_other = 0usize;
    cdir_cursor = cdir_offset;

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
        if name_read_len > 0 {
            let name_read = read_file_at(
                file,
                cdir_cursor.saturating_add(ZIP_CDIR_HEADER_BYTES as u32),
                &mut cdir_name[..name_read_len],
            )?;
            if name_read == name_read_len {
                let name_slice = &cdir_name[..name_read_len];
                let is_dir = name_slice.last().copied() == Some(b'/');
                if !is_dir {
                    if exclude_path
                        .is_some_and(|excluded| eq_ascii_case_insensitive(excluded, name_slice))
                    {
                        cdir_cursor = next_cursor;
                        continue;
                    }

                    let entry = ZipEntryRef {
                        compression,
                        compressed_size,
                        uncompressed_size,
                        local_header_offset,
                    };
                    let mut sniffed_media = None;
                    let mut is_image = is_image_resource_name(name_slice)
                        || is_probable_image_resource_path(name_slice);
                    if !is_image {
                        sniffed_media = sniff_cover_media_from_entry(file, entry)?;
                        is_image = sniffed_media.is_some();
                    }

                    if is_image {
                        let cover_like = is_cover_like_image_path(name_slice);
                        let match_candidate = if cover_like {
                            if let Some(target_idx) = target_cover_like {
                                if seen_cover_like == target_idx {
                                    true
                                } else {
                                    seen_cover_like = seen_cover_like.saturating_add(1);
                                    false
                                }
                            } else {
                                false
                            }
                        } else if let Some(target_idx) = target_other {
                            if seen_other == target_idx {
                                true
                            } else {
                                seen_other = seen_other.saturating_add(1);
                                false
                            }
                        } else {
                            false
                        };

                        if match_candidate {
                            let mut path = String::<PATH_BYTES>::new();
                            copy_ascii_or_lossy(name_slice, &mut path);
                            let mut media = String::<MEDIA_BYTES>::new();
                            copy_media_type_or_default(None, name_slice, &mut media);
                            if let Some(sniffed) = sniffed_media {
                                media.clear();
                                let _ = media.push_str(sniffed);
                            }
                            return Ok(Some((entry, path, media)));
                        }
                    }
                }
            }
        }

        cdir_cursor = next_cursor;
    }

    Ok(None)
}

