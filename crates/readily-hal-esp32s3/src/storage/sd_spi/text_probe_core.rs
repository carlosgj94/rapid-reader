fn scan_epub_metadata<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
    const TITLE_BYTES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    file_size: u32,
    title_out: &mut String<TITLE_BYTES>,
) -> Result<EpubMetadata, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let Some((opf_entry, opf_path)) =
        find_opf_entry_and_path::<_, _, _, _, _, ZIP_PATH_BYTES>(file, file_size)?
    else {
        return Ok(EpubMetadata::default());
    };

    let mut opf_buf = [0u8; ZIP_OPF_BYTES];
    let opf_read = read_zip_entry_prefix(file, opf_entry, &mut opf_buf)?;
    if opf_read == 0 {
        return Ok(EpubMetadata::default());
    }

    Ok(parse_opf_metadata(
        &opf_buf[..opf_read],
        opf_path.as_str(),
        title_out,
    ))
}

fn find_first_text_entry<
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
    let mut opf_buf = [0u8; ZIP_OPF_BYTES];
    let mut spine_candidate: Option<(ZipEntryRef, String<PATH_BYTES>)> = None;
    if let Some((opf_entry, opf_path)) =
        find_opf_entry_and_path::<_, _, _, _, _, PATH_BYTES>(file, file_size)?
    {
        let opf_read = read_zip_entry_prefix(file, opf_entry, &mut opf_buf)?;
        if opf_read > 0 {
            let mut spine_path = String::<PATH_BYTES>::new();
            if parse_spine_first_text_href(&opf_buf[..opf_read], opf_path.as_str(), &mut spine_path)
                && let Some(entry) = find_entry_by_path(file, file_size, spine_path.as_bytes())?
            {
                if entry.uncompressed_size >= ZIP_MIN_PRIMARY_TEXT_BYTES {
                    return Ok(Some((entry, spine_path)));
                }
                spine_candidate = Some((entry, spine_path));
            }
        }
    }

    let Some((cdir_offset, cdir_entries)) = cdir_info(file, file_size)? else {
        return Ok(None);
    };

    let mut cdir_header = [0u8; ZIP_CDIR_HEADER_BYTES];
    let mut cdir_name = [0u8; ZIP_NAME_BYTES];
    let mut cdir_cursor = cdir_offset;
    let mut fallback_non_front_matter: Option<(ZipEntryRef, String<PATH_BYTES>)> = None;
    let mut preferred_non_front_matter: Option<(ZipEntryRef, String<PATH_BYTES>)> = None;
    let mut fallback_front_matter: Option<(ZipEntryRef, String<PATH_BYTES>)> = None;

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
            let entry_ref = ZipEntryRef {
                compression,
                compressed_size,
                uncompressed_size,
                local_header_offset,
            };
            let mut resource = String::<PATH_BYTES>::new();
            copy_ascii_or_lossy(name_slice, &mut resource);
            if path_is_probably_front_matter(name_slice) {
                if fallback_front_matter.is_none() {
                    fallback_front_matter = Some((entry_ref, resource));
                }
            } else {
                if fallback_non_front_matter.is_none() {
                    fallback_non_front_matter = Some((entry_ref, resource.clone()));
                }
                if preferred_non_front_matter.is_none()
                    && entry_ref.uncompressed_size >= ZIP_MIN_PRIMARY_TEXT_BYTES
                {
                    preferred_non_front_matter = Some((entry_ref, resource));
                }
            }
        }

        cdir_cursor = next_cursor;
    }

    if let Some(candidate) = preferred_non_front_matter {
        return Ok(Some(candidate));
    }
    if let Some(candidate) = spine_candidate {
        return Ok(Some(candidate));
    }
    if let Some(candidate) = fallback_non_front_matter {
        return Ok(Some(candidate));
    }
    if let Some(candidate) = fallback_front_matter {
        return Ok(Some(candidate));
    }

    Ok(None)
}

fn find_next_text_entry<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
    const PATH_BYTES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    file_size: u32,
    current_path: &str,
) -> Result<Option<(ZipEntryRef, String<PATH_BYTES>)>, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    if current_path.is_empty() {
        return find_first_text_entry::<_, _, _, _, _, PATH_BYTES>(file, file_size);
    }

    let mut opf_buf = [0u8; ZIP_OPF_BYTES];
    if let Some((opf_entry, opf_path)) =
        find_opf_entry_and_path::<_, _, _, _, _, PATH_BYTES>(file, file_size)?
    {
        let opf_read = read_zip_entry_prefix(file, opf_entry, &mut opf_buf)?;
        if opf_read > 0 {
            let mut next_path = String::<PATH_BYTES>::new();
            if parse_spine_next_text_href(
                &opf_buf[..opf_read],
                opf_path.as_str(),
                current_path,
                &mut next_path,
            ) && let Some(entry) = find_entry_by_path(file, file_size, next_path.as_bytes())?
            {
                return Ok(Some((entry, next_path)));
            }
        }
    }

    let Some((cdir_offset, cdir_entries)) = cdir_info(file, file_size)? else {
        return Ok(None);
    };

    let mut cdir_header = [0u8; ZIP_CDIR_HEADER_BYTES];
    let mut cdir_name = [0u8; ZIP_NAME_BYTES];
    let mut cdir_cursor = cdir_offset;
    let mut seen_current = false;
    let mut fallback_any_text: Option<(ZipEntryRef, String<PATH_BYTES>)> = None;
    let mut fallback_non_front_matter: Option<(ZipEntryRef, String<PATH_BYTES>)> = None;
    let mut preferred_non_front_matter: Option<(ZipEntryRef, String<PATH_BYTES>)> = None;

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
        if !seen_current {
            if name_complete && eq_ascii_case_insensitive(name_slice, current_path.as_bytes()) {
                seen_current = true;
            }
            cdir_cursor = next_cursor;
            continue;
        }

        if name_complete && is_text_resource_name(name_slice) {
            let entry_ref = ZipEntryRef {
                compression,
                compressed_size,
                uncompressed_size,
                local_header_offset,
            };
            let mut resource = String::<PATH_BYTES>::new();
            copy_ascii_or_lossy(name_slice, &mut resource);

            if fallback_any_text.is_none() {
                fallback_any_text = Some((entry_ref, resource.clone()));
            }

            if !path_is_probably_front_matter(name_slice) {
                if fallback_non_front_matter.is_none() {
                    fallback_non_front_matter = Some((entry_ref, resource.clone()));
                }
                if preferred_non_front_matter.is_none()
                    && entry_ref.uncompressed_size >= ZIP_MIN_PRIMARY_TEXT_BYTES
                {
                    preferred_non_front_matter = Some((entry_ref, resource));
                }
            }
        }

        cdir_cursor = next_cursor;
    }

    if let Some(candidate) = preferred_non_front_matter {
        return Ok(Some(candidate));
    }
    if let Some(candidate) = fallback_non_front_matter {
        return Ok(Some(candidate));
    }
    if let Some(candidate) = fallback_any_text {
        return Ok(Some(candidate));
    }

    Ok(None)
}

fn find_text_entry_by_chapter_index<
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
) -> Result<Option<ChapterProbeTarget<PATH_BYTES>>, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let mut toc_resource = String::<PATH_BYTES>::new();
    let mut toc_fragment = String::<ZIP_PATH_BYTES>::new();
    let mut toc_label = String::<SD_CHAPTER_LABEL_BYTES>::new();
    let mut toc_index = 0u16;
    let mut toc_total = 1u16;
    if let Some(entry) = find_toc_chapter_entry_at_index::<_, _, _, _, _, PATH_BYTES>(
        file,
        file_size,
        target_chapter,
        &mut toc_resource,
        &mut toc_fragment,
        &mut toc_label,
        &mut toc_index,
        &mut toc_total,
    )? {
        let start_offset = find_fragment_offset_in_zip_entry(file, entry, toc_fragment.as_str())?
            .unwrap_or(0);
        return Ok(Some(ChapterProbeTarget {
            entry,
            resource: toc_resource,
            chapter_index: toc_index,
            chapter_total: toc_total.max(1),
            chapter_label: toc_label,
            start_offset,
        }));
    }

    let mut opf_buf = [0u8; ZIP_OPF_BYTES];
    if let Some((opf_entry, opf_path)) =
        find_opf_entry_and_path::<_, _, _, _, _, PATH_BYTES>(file, file_size)?
    {
        let opf_read = read_zip_entry_prefix(file, opf_entry, &mut opf_buf)?;
        if opf_read > 0 {
            let mut chapter_path = String::<PATH_BYTES>::new();
            let mut chapter_index = 0u16;
            let mut chapter_total = 1u16;
            if parse_spine_text_href_at(
                &opf_buf[..opf_read],
                opf_path.as_str(),
                target_chapter,
                &mut chapter_path,
                &mut chapter_index,
                &mut chapter_total,
            ) && let Some(entry) = find_entry_by_path(file, file_size, chapter_path.as_bytes())?
            {
                return Ok(Some(ChapterProbeTarget {
                    entry,
                    resource: chapter_path,
                    chapter_index,
                    chapter_total: chapter_total.max(1),
                    chapter_label: String::new(),
                    start_offset: 0,
                }));
            }
        }
    }

    let Some((entry, resource, chapter_index, chapter_total)) =
        cdir_text_entry_at::<_, _, _, _, _, PATH_BYTES>(file, file_size, target_chapter)?
    else {
        return Ok(None);
    };
    Ok(Some(ChapterProbeTarget {
        entry,
        resource,
        chapter_index,
        chapter_total: chapter_total.max(1),
        chapter_label: String::new(),
        start_offset: 0,
    }))
}

#[derive(Debug)]
struct ChapterProbeTarget<const PATH_BYTES: usize> {
    entry: ZipEntryRef,
    resource: String<PATH_BYTES>,
    chapter_index: u16,
    chapter_total: u16,
    chapter_label: String<SD_CHAPTER_LABEL_BYTES>,
    start_offset: u32,
}

/// Probe card + mount FAT + read a first text chunk from one EPUB file.
///
/// `short_name` must be an 8.3 short filename (for example `BOOK~1.EPU`).
pub fn probe_and_read_epub_text_chunk<BUS, CS, DELAY, const PATH_BYTES: usize>(
    bus: &mut BUS,
    cs: &mut CS,
    delay: &mut DELAY,
    books_dir_name: &str,
    short_name: &str,
    out: &mut [u8],
) -> Result<SdEpubTextChunkResult<PATH_BYTES>, SdProbeError<BUS::Error, CS::Error>>
where
    BUS: SpiBus<u8>,
    CS: OutputPin,
    DELAY: DelayNs,
    BUS::Error: core::fmt::Debug,
    CS::Error: core::fmt::Debug,
{
    probe_and_read_epub_text_chunk_from_resource::<_, _, _, PATH_BYTES>(
        bus,
        cs,
        delay,
        books_dir_name,
        short_name,
        "",
        0,
        out,
    )
}

/// Probe card + mount FAT + read a text chunk from a chapter index.
///
/// `target_chapter` is zero-based. When the requested chapter is out of range,
/// the reader falls back to the last available chapter.
pub fn probe_and_read_epub_text_chunk_at_chapter<BUS, CS, DELAY, const PATH_BYTES: usize>(
    bus: &mut BUS,
    cs: &mut CS,
    delay: &mut DELAY,
    books_dir_name: &str,
    short_name: &str,
    target_chapter: u16,
    out: &mut [u8],
) -> Result<SdEpubTextChunkResult<PATH_BYTES>, SdProbeError<BUS::Error, CS::Error>>
where
    BUS: SpiBus<u8>,
    CS: OutputPin,
    DELAY: DelayNs,
    BUS::Error: core::fmt::Debug,
    CS::Error: core::fmt::Debug,
{
    cs.set_high().map_err(SdProbeError::ChipSelect)?;

    // SD SPI init requires >=74 clock cycles with CS deasserted.
    let preclock = [0xFFu8; 10];
    bus.write(&preclock).map_err(SdProbeError::Spi)?;

    let spi_device = ExclusiveSpiDevice::new(bus, cs);
    let mut delay_ref = DelayRef(delay);
    let sd_card = SdCard::new(spi_device, &mut delay_ref);
    let card_size_bytes = sd_card.num_bytes().map_err(SdProbeError::Card)?;

    let mut result = SdEpubTextChunkResult {
        card_size_bytes,
        text_resource: String::new(),
        start_offset: 0,
        chapter_index: 0,
        chapter_total: 1,
        chapter_label: String::new(),
        compression: 0,
        bytes_read: 0,
        end_of_resource: false,
        status: SdEpubTextChunkStatus::NoTextResource,
    };

    let mut volume_mgr = VolumeManager::new(sd_card, FixedTimeSource);
    let mut volume = volume_mgr
        .open_volume(VolumeIdx(0))
        .map_err(SdProbeError::Filesystem)?;
    let mut root_dir = volume.open_root_dir().map_err(SdProbeError::Filesystem)?;

    let mut books_dir = match root_dir.open_dir(books_dir_name) {
        Ok(dir) => dir,
        Err(embedded_sdmmc::Error::NotFound) => return Ok(result),
        Err(err) => return Err(SdProbeError::Filesystem(err)),
    };

    let short_name = match ShortFileName::create_from_str(short_name) {
        Ok(name) => name,
        Err(_) => return Ok(result),
    };

    let mut file = match books_dir.open_file_in_dir(&short_name, Mode::ReadOnly) {
        Ok(file) => file,
        Err(embedded_sdmmc::Error::NotFound) => return Ok(result),
        Err(err) => return Err(SdProbeError::Filesystem(err)),
    };

    let mut header = [0u8; 4];
    let read_now = file.read(&mut header).map_err(SdProbeError::Filesystem)?;
    if !has_zip_signature(header, read_now) {
        result.status = SdEpubTextChunkStatus::NotZip;
        file.close().map_err(SdProbeError::Filesystem)?;
        books_dir.close().map_err(SdProbeError::Filesystem)?;
        root_dir.close().map_err(SdProbeError::Filesystem)?;
        volume.close().map_err(SdProbeError::Filesystem)?;
        return Ok(result);
    }

    let file_size = file.length();
    if let Some(target) =
        find_text_entry_by_chapter_index::<_, _, _, _, _, PATH_BYTES>(
            &mut file,
            file_size,
            target_chapter,
        )
        .map_err(SdProbeError::Filesystem)?
    {
        result.compression = target.entry.compression;
        result.text_resource = target.resource;
        result.start_offset = target.start_offset;
        result.chapter_index = target.chapter_index;
        result.chapter_total = target.chapter_total.max(1);
        result.chapter_label = target.chapter_label;

        if matches!(target.entry.compression, 0 | 8) {
            let (bytes_read, end_of_resource) =
                read_zip_entry_chunk(&mut file, target.entry, target.start_offset, out)
                    .map_err(SdProbeError::Filesystem)?;
            result.bytes_read = bytes_read;
            result.end_of_resource = end_of_resource;
            result.status = if bytes_read > 0 || end_of_resource {
                SdEpubTextChunkStatus::ReadOk
            } else {
                SdEpubTextChunkStatus::DecodeFailed
            };
        } else {
            result.status = SdEpubTextChunkStatus::UnsupportedCompression;
        }
    }

    file.close().map_err(SdProbeError::Filesystem)?;
    books_dir.close().map_err(SdProbeError::Filesystem)?;
    root_dir.close().map_err(SdProbeError::Filesystem)?;
    volume.close().map_err(SdProbeError::Filesystem)?;

    Ok(result)
}
