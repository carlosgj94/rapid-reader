fn parse_opf_metadata<const TITLE_BYTES: usize>(
    opf: &[u8],
    opf_path: &str,
    title_out: &mut String<TITLE_BYTES>,
) -> EpubMetadata {
    let mut metadata = EpubMetadata::default();

    let _ = parse_xml_tag_text(opf, b"<dc:title", title_out)
        || parse_xml_tag_text(opf, b"<title", title_out)
        || parse_opf_meta_content(opf, b"property", title_out)
        || parse_opf_meta_content(opf, b"name", title_out);

    let mut cover_path = String::<ZIP_PATH_BYTES>::new();
    let mut cover_media = String::<ZIP_MEDIA_BYTES>::new();
    metadata.has_cover = parse_opf_cover_resource(opf, opf_path, &mut cover_path, &mut cover_media)
        || find_ascii_case_insensitive(opf, b"cover-image", 0).is_some()
        || find_ascii_case_insensitive(opf, b"name=\"cover\"", 0).is_some()
        || find_ascii_case_insensitive(opf, b"name='cover'", 0).is_some();

    metadata
}

fn read_file_at<D, T, const MAX_DIRS: usize, const MAX_FILES: usize, const MAX_VOLUMES: usize>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    offset: u32,
    out: &mut [u8],
) -> Result<usize, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    file.seek_from_start(offset)?;
    let mut total = 0usize;
    while total < out.len() {
        let read_now = file.read(&mut out[total..])?;
        if read_now == 0 {
            break;
        }
        total = total.saturating_add(read_now);
    }
    Ok(total)
}

fn entry_data_offset<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    entry: ZipEntryRef,
) -> Result<Option<u32>, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let mut local = [0u8; ZIP_LOCAL_HEADER_BYTES];
    let local_read = read_file_at(file, entry.local_header_offset, &mut local)?;
    if local_read < ZIP_LOCAL_HEADER_BYTES || !local.starts_with(&ZIP_LOCAL_SIG) {
        return Ok(None);
    }

    let name_len = read_u16_le(&local, 26) as u32;
    let extra_len = read_u16_le(&local, 28) as u32;
    let data_offset = entry
        .local_header_offset
        .checked_add(ZIP_LOCAL_HEADER_BYTES as u32)
        .and_then(|value| value.checked_add(name_len))
        .and_then(|value| value.checked_add(extra_len));

    Ok(data_offset)
}

fn read_stored_entry_chunk<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    entry: ZipEntryRef,
    start_offset: u32,
    out: &mut [u8],
) -> Result<(usize, bool), embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    if out.is_empty() || entry.uncompressed_size == 0 {
        return Ok((0, true));
    }

    if start_offset >= entry.uncompressed_size {
        return Ok((0, true));
    }

    let Some(data_offset) = entry_data_offset(file, entry)? else {
        return Ok((0, true));
    };

    let Some(read_start) = data_offset.checked_add(start_offset) else {
        return Ok((0, true));
    };
    let remaining = (entry.uncompressed_size - start_offset) as usize;
    let read_len = remaining.min(out.len());
    let read_now = read_file_at(file, read_start, &mut out[..read_len])?;
    let end_of_resource = start_offset.saturating_add(read_now as u32) >= entry.uncompressed_size;
    Ok((read_now, end_of_resource))
}

fn read_deflated_entry_prefix_fast<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    entry: ZipEntryRef,
    out: &mut [u8],
) -> Result<usize, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    if out.is_empty() || entry.uncompressed_size == 0 || entry.compressed_size == 0 {
        return Ok(0);
    }

    let Some(data_offset) = entry_data_offset(file, entry)? else {
        return Ok(0);
    };

    file.seek_from_start(data_offset)?;

    let mut inflater = DecompressorOxide::new();
    let mut input = [0u8; ZIP_DEFLATE_INPUT_BYTES];
    let mut input_len = 0usize;
    let mut input_pos = 0usize;
    let mut compressed_remaining = entry.compressed_size;
    let mut out_pos = 0usize;

    loop {
        if input_pos >= input_len && compressed_remaining > 0 {
            let read_len = input.len().min(compressed_remaining as usize);
            let read_now = file.read(&mut input[..read_len])?;
            if read_now == 0 {
                return Ok(out_pos);
            }
            input_len = read_now;
            input_pos = 0;
            compressed_remaining = compressed_remaining.saturating_sub(read_now as u32);
        }

        let has_more_input = input_pos < input_len || compressed_remaining > 0;
        let mut flags = inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF;
        if has_more_input {
            flags |= inflate_flags::TINFL_FLAG_HAS_MORE_INPUT;
        }

        let (status, consumed, written) = decompress(
            &mut inflater,
            &input[input_pos..input_len],
            out,
            out_pos,
            flags,
        );
        input_pos = input_pos.saturating_add(consumed);
        out_pos = out_pos.saturating_add(written);

        match status {
            TINFLStatus::Done | TINFLStatus::HasMoreOutput => return Ok(out_pos),
            TINFLStatus::NeedsMoreInput => {
                if !has_more_input {
                    return Ok(out_pos);
                }
            }
            _ => return Ok(0),
        }

        if out_pos >= out.len() {
            return Ok(out_pos);
        }
    }
}

fn read_deflated_entry_chunk<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    entry: ZipEntryRef,
    start_offset: u32,
    out: &mut [u8],
) -> Result<(usize, bool), embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    if out.is_empty() || entry.uncompressed_size == 0 || entry.compressed_size == 0 {
        return Ok((0, true));
    }

    if start_offset == 0 {
        let bytes = read_deflated_entry_prefix_fast(file, entry, out)?;
        let end_of_resource = bytes >= entry.uncompressed_size as usize;
        return Ok((bytes, end_of_resource));
    }

    if start_offset >= entry.uncompressed_size {
        return Ok((0, true));
    }

    let Some(data_offset) = entry_data_offset(file, entry)? else {
        return Ok((0, true));
    };

    file.seek_from_start(data_offset)?;

    let mut inflater = InflateState::new(DataFormat::Raw);
    let mut input = [0u8; ZIP_DEFLATE_INPUT_BYTES];
    let mut output = [0u8; ZIP_DEFLATE_OUTPUT_BYTES];
    let mut input_len = 0usize;
    let mut input_pos = 0usize;
    let mut compressed_remaining = entry.compressed_size;
    let mut uncompressed_total = 0u32;
    let mut written_total = 0usize;
    let mut no_progress_loops = 0u8;
    let mut stream_done = false;

    loop {
        if input_pos >= input_len && compressed_remaining > 0 {
            let read_len = input.len().min(compressed_remaining as usize);
            let read_now = file.read(&mut input[..read_len])?;
            if read_now == 0 {
                break;
            }
            input_len = read_now;
            input_pos = 0;
            compressed_remaining = compressed_remaining.saturating_sub(read_now as u32);
        }

        let compressed_input = &input[input_pos..input_len];
        let flush = if compressed_remaining == 0 && compressed_input.is_empty() {
            MZFlush::Finish
        } else {
            MZFlush::None
        };
        let stream = inflate(&mut inflater, compressed_input, &mut output, flush);
        input_pos = input_pos.saturating_add(stream.bytes_consumed);

        if stream.bytes_consumed == 0 && stream.bytes_written == 0 {
            no_progress_loops = no_progress_loops.saturating_add(1);
            if no_progress_loops >= 2 {
                break;
            }
        } else {
            no_progress_loops = 0;
        }

        if stream.bytes_written > 0 {
            let chunk_start = uncompressed_total;
            let chunk_end = chunk_start.saturating_add(stream.bytes_written as u32);

            if chunk_end > start_offset && written_total < out.len() {
                let copy_start = start_offset.saturating_sub(chunk_start) as usize;
                let available = stream.bytes_written.saturating_sub(copy_start);
                let copy_len = available.min(out.len().saturating_sub(written_total));
                if copy_len > 0 {
                    out[written_total..written_total + copy_len]
                        .copy_from_slice(&output[copy_start..copy_start + copy_len]);
                    written_total = written_total.saturating_add(copy_len);
                }
            }

            uncompressed_total = chunk_end;
        }

        match stream.status {
            Ok(MZStatus::StreamEnd) => {
                stream_done = true;
                break;
            }
            Ok(MZStatus::Ok) => {}
            Err(MZError::Buf) => {
                if compressed_remaining == 0 && input_pos >= input_len {
                    break;
                }
            }
            _ => return Ok((0, false)),
        }

        if written_total >= out.len() {
            return Ok((written_total, false));
        }

        if compressed_remaining == 0 && input_pos >= input_len && stream.bytes_written == 0 {
            break;
        }
    }

    let end = stream_done
        || uncompressed_total >= entry.uncompressed_size
        || (compressed_remaining == 0 && input_pos >= input_len);
    if written_total == 0 && !end {
        return Ok((0, false));
    }
    Ok((written_total, end))
}

fn read_zip_entry_chunk<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    entry: ZipEntryRef,
    start_offset: u32,
    out: &mut [u8],
) -> Result<(usize, bool), embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    match entry.compression {
        0 => read_stored_entry_chunk(file, entry, start_offset, out),
        8 => read_deflated_entry_chunk(file, entry, start_offset, out),
        _ => Ok((0, false)),
    }
}

fn read_zip_entry_prefix<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    entry: ZipEntryRef,
    out: &mut [u8],
) -> Result<usize, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let (read_now, _) = read_zip_entry_chunk(file, entry, 0, out)?;
    Ok(read_now)
}

