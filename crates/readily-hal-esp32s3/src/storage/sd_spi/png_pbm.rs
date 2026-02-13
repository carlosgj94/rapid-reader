fn parse_ascii_u32(token: &[u8]) -> Option<u32> {
    if token.is_empty() {
        return None;
    }

    let mut value = 0u32;
    for &byte in token {
        if !byte.is_ascii_digit() {
            return None;
        }
        let digit = (byte - b'0') as u32;
        value = value.checked_mul(10)?.checked_add(digit)?;
    }
    Some(value)
}

fn pbm_next_token<'a>(buf: &'a [u8], cursor: &mut usize) -> Option<&'a [u8]> {
    while *cursor < buf.len() {
        let byte = buf[*cursor];
        if byte.is_ascii_whitespace() {
            *cursor = cursor.saturating_add(1);
            continue;
        }
        if byte == b'#' {
            while *cursor < buf.len() && buf[*cursor] != b'\n' {
                *cursor = cursor.saturating_add(1);
            }
            continue;
        }
        break;
    }
    if *cursor >= buf.len() {
        return None;
    }

    let start = *cursor;
    while *cursor < buf.len() {
        let byte = buf[*cursor];
        if byte.is_ascii_whitespace() || byte == b'#' {
            break;
        }
        *cursor = cursor.saturating_add(1);
    }

    Some(&buf[start..*cursor])
}

fn mono_set_pixel(bits: &mut [u8], width: usize, x: usize, y: usize, on: bool) {
    if width == 0 {
        return;
    }
    let row_bytes = width.div_ceil(8);
    let idx = y.saturating_mul(row_bytes).saturating_add(x / 8);
    if idx >= bits.len() {
        return;
    }
    let mask = 1u8 << (7 - (x % 8));
    if on {
        bits[idx] |= mask;
    } else {
        bits[idx] &= !mask;
    }
}

fn decode_pbm_thumbnail_p4(
    pbm: &[u8],
    thumb_width: u16,
    thumb_height: u16,
    out_bits: &mut [u8],
) -> Option<(u16, u16, usize)> {
    let mut cursor = 0usize;
    let magic = pbm_next_token(pbm, &mut cursor)?;
    if !eq_ascii_case_insensitive(magic, b"P4") {
        return None;
    }

    let src_width = parse_ascii_u32(pbm_next_token(pbm, &mut cursor)?)?;
    let src_height = parse_ascii_u32(pbm_next_token(pbm, &mut cursor)?)?;
    if src_width == 0 || src_height == 0 {
        return None;
    }
    let src_width = src_width.min(u16::MAX as u32) as usize;
    let src_height = src_height.min(u16::MAX as u32) as usize;

    while cursor < pbm.len() && pbm[cursor].is_ascii_whitespace() {
        cursor = cursor.saturating_add(1);
    }
    if cursor >= pbm.len() {
        return None;
    }

    let src_row_bytes = src_width.div_ceil(8);
    let src_bitmap_bytes = src_row_bytes.checked_mul(src_height)?;
    if cursor.saturating_add(src_bitmap_bytes) > pbm.len() {
        return None;
    }
    let src_pixels = &pbm[cursor..cursor + src_bitmap_bytes];

    let tw = thumb_width.max(1) as usize;
    let th = thumb_height.max(1) as usize;
    let dst_row_bytes = tw.div_ceil(8);
    let dst_bytes = dst_row_bytes.checked_mul(th)?;
    if dst_bytes > out_bits.len() {
        return None;
    }
    out_bits[..dst_bytes].fill(0);

    for dy in 0..th {
        let sy = dy.saturating_mul(src_height) / th;
        for dx in 0..tw {
            let sx = dx.saturating_mul(src_width) / tw;
            let src_idx = sy.saturating_mul(src_row_bytes).saturating_add(sx / 8);
            if src_idx >= src_pixels.len() {
                continue;
            }
            let src_mask = 1u8 << (7 - (sx % 8));
            let on = (src_pixels[src_idx] & src_mask) != 0;
            mono_set_pixel(out_bits, tw, dx, dy, on);
        }
    }

    Some((src_width as u16, src_height as u16, dst_bytes))
}

fn read_u32_be(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn paeth_predictor(a: u8, b: u8, c: u8) -> u8 {
    let a_i = a as i32;
    let b_i = b as i32;
    let c_i = c as i32;
    let p = a_i + b_i - c_i;
    let pa = (p - a_i).abs();
    let pb = (p - b_i).abs();
    let pc = (p - c_i).abs();
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

fn apply_png_filter(
    filter: u8,
    raw: &[u8],
    prev_row: &[u8],
    bpp: usize,
    out_row: &mut [u8],
) -> bool {
    if raw.len() != out_row.len() || prev_row.len() < raw.len() || bpp == 0 {
        return false;
    }

    match filter {
        0 => {
            out_row.copy_from_slice(raw);
            true
        }
        1 => {
            for i in 0..raw.len() {
                let left = if i >= bpp { out_row[i - bpp] } else { 0 };
                out_row[i] = raw[i].wrapping_add(left);
            }
            true
        }
        2 => {
            for i in 0..raw.len() {
                out_row[i] = raw[i].wrapping_add(prev_row[i]);
            }
            true
        }
        3 => {
            for i in 0..raw.len() {
                let left = if i >= bpp { out_row[i - bpp] } else { 0 };
                let up = prev_row[i];
                let avg = ((left as u16 + up as u16) / 2) as u8;
                out_row[i] = raw[i].wrapping_add(avg);
            }
            true
        }
        4 => {
            for i in 0..raw.len() {
                let left = if i >= bpp { out_row[i - bpp] } else { 0 };
                let up = prev_row[i];
                let up_left = if i >= bpp { prev_row[i - bpp] } else { 0 };
                out_row[i] = raw[i].wrapping_add(paeth_predictor(left, up, up_left));
            }
            true
        }
        _ => false,
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "pixel sampling needs explicit palette + PNG mode params and stays allocation-free"
)]
fn mono_sample_from_row(
    row: &[u8],
    x: usize,
    color_type: u8,
    bit_depth: u8,
    channels: usize,
    palette_rgb: &[u8],
    palette_alpha: &[u8],
    palette_entries: usize,
) -> bool {
    fn sample_bits(row: &[u8], x: usize, bit_depth: u8) -> Option<u8> {
        if bit_depth == 0 || bit_depth > 8 {
            return None;
        }
        if bit_depth == 8 {
            return row.get(x).copied();
        }
        let depth = bit_depth as usize;
        let bits_per_row = row.len().saturating_mul(8);
        let bit_offset = x.saturating_mul(depth);
        if bit_offset + depth > bits_per_row {
            return None;
        }
        let byte_idx = bit_offset / 8;
        let bit_in_byte = bit_offset % 8;
        let shift = 8usize.saturating_sub(bit_in_byte + depth);
        let mask = ((1u16 << depth) - 1) as u8;
        Some((row[byte_idx] >> shift) & mask)
    }

    let pixel = x.saturating_mul(channels);

    let (luma, alpha) = match color_type {
        // grayscale
        0 => {
            let Some(value) = sample_bits(row, x, bit_depth) else {
                return false;
            };
            if bit_depth == 8 {
                (value as u32, 255u32)
            } else {
                let max = (1u32 << bit_depth) - 1;
                (((value as u32).saturating_mul(255) / max.max(1)), 255u32)
            }
        }
        // truecolor (RGB)
        2 => {
            if pixel + channels > row.len() {
                return false;
            }
            let r = row[pixel] as u32;
            let g = row[pixel + 1] as u32;
            let b = row[pixel + 2] as u32;
            (((r * 30 + g * 59 + b * 11) / 100), 255u32)
        }
        // grayscale + alpha
        4 => {
            if pixel + channels > row.len() {
                return false;
            }
            (row[pixel] as u32, row[pixel + 1] as u32)
        }
        // RGBA
        6 => {
            if pixel + channels > row.len() {
                return false;
            }
            let r = row[pixel] as u32;
            let g = row[pixel + 1] as u32;
            let b = row[pixel + 2] as u32;
            let a = row[pixel + 3] as u32;
            (((r * 30 + g * 59 + b * 11) / 100), a)
        }
        // indexed-color palette
        3 => {
            let Some(idx) = sample_bits(row, x, bit_depth).map(|value| value as usize) else {
                return false;
            };
            if idx >= palette_entries {
                return false;
            }
            let base = idx.saturating_mul(3);
            if base + 2 >= palette_rgb.len() {
                return false;
            }
            let r = palette_rgb[base] as u32;
            let g = palette_rgb[base + 1] as u32;
            let b = palette_rgb[base + 2] as u32;
            let a = palette_alpha.get(idx).copied().unwrap_or(255) as u32;
            (((r * 30 + g * 59 + b * 11) / 100), a)
        }
        _ => return false,
    };

    // Composite against white background before thresholding to 1bpp.
    let comp = (luma.saturating_mul(alpha) + 255 * (255 - alpha)) / 255;
    comp < 160
}

#[allow(
    clippy::too_many_arguments,
    reason = "streamed ZIP reads require explicit parser state to avoid heap allocations"
)]
fn zip_entry_stream_read_exact<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
    const STREAM_BYTES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    entry: ZipEntryRef,
    stream_offset: &mut u32,
    stream_buf: &mut [u8; STREAM_BYTES],
    stream_len: &mut usize,
    stream_pos: &mut usize,
    stream_end: &mut bool,
    out: &mut [u8],
) -> Result<bool, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let mut copied = 0usize;
    let mut no_progress_reads = 0u8;
    while copied < out.len() {
        if *stream_pos >= *stream_len {
            if *stream_end {
                return Ok(false);
            }
            let (read_now, end) = read_zip_entry_chunk(file, entry, *stream_offset, stream_buf)?;
            *stream_offset = stream_offset.saturating_add(read_now as u32);
            *stream_len = read_now;
            *stream_pos = 0;
            *stream_end = end;
            if read_now == 0 {
                if end {
                    return Ok(false);
                }
                no_progress_reads = no_progress_reads.saturating_add(1);
                if no_progress_reads >= 4 {
                    return Ok(false);
                }
                continue;
            }
            no_progress_reads = 0;
        }

        let avail = (*stream_len - *stream_pos).min(out.len() - copied);
        out[copied..copied + avail].copy_from_slice(&stream_buf[*stream_pos..*stream_pos + avail]);
        *stream_pos += avail;
        copied += avail;
    }

    Ok(true)
}

#[allow(
    clippy::too_many_arguments,
    reason = "streamed ZIP parser keeps state external and explicit for no_std usage"
)]
fn zip_entry_stream_skip<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
    const STREAM_BYTES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    entry: ZipEntryRef,
    stream_offset: &mut u32,
    stream_buf: &mut [u8; STREAM_BYTES],
    stream_len: &mut usize,
    stream_pos: &mut usize,
    stream_end: &mut bool,
    mut skip_len: usize,
) -> Result<bool, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let mut no_progress_reads = 0u8;
    while skip_len > 0 {
        if *stream_pos >= *stream_len {
            if *stream_end {
                return Ok(false);
            }
            let (read_now, end) = read_zip_entry_chunk(file, entry, *stream_offset, stream_buf)?;
            *stream_offset = stream_offset.saturating_add(read_now as u32);
            *stream_len = read_now;
            *stream_pos = 0;
            *stream_end = end;
            if read_now == 0 {
                if end {
                    return Ok(false);
                }
                no_progress_reads = no_progress_reads.saturating_add(1);
                if no_progress_reads >= 4 {
                    return Ok(false);
                }
                continue;
            }
            no_progress_reads = 0;
        }

        let avail = (*stream_len - *stream_pos).min(skip_len);
        *stream_pos += avail;
        skip_len -= avail;
    }

    Ok(true)
}

fn decode_png_thumbnail_stream<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    entry: ZipEntryRef,
    thumb_width: u16,
    thumb_height: u16,
    out_bits: &mut [u8],
) -> PngThumbDecodeResult<D::Error>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let tw = thumb_width.max(1) as usize;
    let th = thumb_height.max(1) as usize;
    let dst_row_bytes = tw.div_ceil(8);
    let dst_bytes = dst_row_bytes.saturating_mul(th);
    if dst_bytes == 0 || dst_bytes > out_bits.len() {
        return Ok(None);
    }
    out_bits[..dst_bytes].fill(0);

    let mut stream_buf = [0u8; PNG_STREAM_BYTES];
    let mut stream_offset = 0u32;
    let mut stream_len = 0usize;
    let mut stream_pos = 0usize;
    let mut stream_end = false;

    let mut sig = [0u8; 8];
    if !zip_entry_stream_read_exact::<_, _, _, _, _, PNG_STREAM_BYTES>(
        file,
        entry,
        &mut stream_offset,
        &mut stream_buf,
        &mut stream_len,
        &mut stream_pos,
        &mut stream_end,
        &mut sig,
    )? {
        return Ok(None);
    }
    if sig != [137, 80, 78, 71, 13, 10, 26, 10] {
        info!("sd: png decode fail reason=bad_signature");
        return Ok(None);
    }

    let mut src_width = 0usize;
    let mut src_height = 0usize;
    let mut channels = 0usize;
    let mut color_type = 0u8;
    let mut bit_depth = 0u8;
    let mut bpp = 0usize;
    let mut row_raw_len = 0usize;
    let mut row_len = 0usize;
    let mut header_ok = false;
    let mut palette_rgb = [0u8; 256 * 3];
    let mut palette_alpha = [255u8; 256];
    let mut palette_entries = 0usize;

    let mut inflater = InflateState::new(DataFormat::Zlib);
    let mut inflate_out = [0u8; PNG_INFLATE_OUT_BYTES];
    let mut idat_input = [0u8; PNG_IDAT_IN_BYTES];
    let mut row_accum = [0u8; PNG_ROW_BYTES_MAX];
    let mut row_prev = [0u8; PNG_ROW_BYTES_MAX];
    let mut row_cur = [0u8; PNG_ROW_BYTES_MAX];
    let mut row_fill = 0usize;
    let mut row_index = 0usize;
    let mut saw_idat = false;

    let mut chunk_header = [0u8; 8];
    let mut chunk_crc = [0u8; 4];
    loop {
        if !zip_entry_stream_read_exact::<_, _, _, _, _, PNG_STREAM_BYTES>(
            file,
            entry,
            &mut stream_offset,
            &mut stream_buf,
            &mut stream_len,
            &mut stream_pos,
            &mut stream_end,
            &mut chunk_header,
        )? {
            break;
        }

        let chunk_len = read_u32_be(&chunk_header, 0) as usize;
        let chunk_type = &chunk_header[4..8];
        if chunk_type == b"IHDR" {
            if chunk_len != 13 {
                return Ok(None);
            }
            let mut ihdr = [0u8; 13];
            if !zip_entry_stream_read_exact::<_, _, _, _, _, PNG_STREAM_BYTES>(
                file,
                entry,
                &mut stream_offset,
                &mut stream_buf,
                &mut stream_len,
                &mut stream_pos,
                &mut stream_end,
                &mut ihdr,
            )? {
                return Ok(None);
            }
            src_width = read_u32_be(&ihdr, 0) as usize;
            src_height = read_u32_be(&ihdr, 4) as usize;
            bit_depth = ihdr[8];
            color_type = ihdr[9];
            let compression_method = ihdr[10];
            let filter_method = ihdr[11];
            let interlace_method = ihdr[12];
            if src_width == 0
                || src_height == 0
                || compression_method != 0
                || filter_method != 0
                || interlace_method != 0
            {
                info!(
                    "sd: png decode fail reason=unsupported_ihdr width={} height={} bit_depth={} color_type={} compression={} filter={} interlace={}",
                    src_width,
                    src_height,
                    bit_depth,
                    color_type,
                    compression_method,
                    filter_method,
                    interlace_method
                );
                return Ok(None);
            }
            channels = match color_type {
                0 => 1,
                2 => 3,
                3 => 1,
                4 => 2,
                6 => 4,
                _ => {
                    info!(
                        "sd: png decode fail reason=unsupported_color_type color_type={}",
                        color_type
                    );
                    return Ok(None);
                }
            };
            let bits_per_pixel = channels.saturating_mul(bit_depth as usize);
            if bits_per_pixel == 0 {
                return Ok(None);
            }
            // Keep implementation compact: packed samples only for grayscale/palette.
            if bit_depth != 8 && !matches!(color_type, 0 | 3) {
                info!(
                    "sd: png decode fail reason=unsupported_bit_depth color_type={} bit_depth={}",
                    color_type, bit_depth
                );
                return Ok(None);
            }
            let row_bits = match src_width.checked_mul(bits_per_pixel) {
                Some(value) => value,
                None => return Ok(None),
            };
            row_raw_len = row_bits.div_ceil(8);
            row_len = match row_raw_len.checked_add(1) {
                Some(value) => value,
                None => return Ok(None),
            };
            if row_len == 0 || row_len > PNG_ROW_BYTES_MAX {
                info!(
                    "sd: png decode fail reason=row_too_wide row_len={} max={}",
                    row_len, PNG_ROW_BYTES_MAX
                );
                return Ok(None);
            }
            bpp = bits_per_pixel.div_ceil(8).max(1);
            header_ok = true;
        } else if chunk_type == b"PLTE" {
            if chunk_len == 0 || !chunk_len.is_multiple_of(3) {
                info!(
                    "sd: png decode fail reason=invalid_plte_len chunk_len={}",
                    chunk_len
                );
                return Ok(None);
            }
            if chunk_len > palette_rgb.len() {
                info!(
                    "sd: png decode fail reason=plte_too_large chunk_len={}",
                    chunk_len
                );
                return Ok(None);
            }
            let entries = chunk_len / 3;
            if !zip_entry_stream_read_exact::<_, _, _, _, _, PNG_STREAM_BYTES>(
                file,
                entry,
                &mut stream_offset,
                &mut stream_buf,
                &mut stream_len,
                &mut stream_pos,
                &mut stream_end,
                &mut palette_rgb[..chunk_len],
            )? {
                return Ok(None);
            }
            palette_entries = entries;
            palette_alpha.fill(255);
        } else if chunk_type == b"tRNS" {
            if color_type != 3 {
                if chunk_len > 0
                    && !zip_entry_stream_skip::<_, _, _, _, _, PNG_STREAM_BYTES>(
                        file,
                        entry,
                        &mut stream_offset,
                        &mut stream_buf,
                        &mut stream_len,
                        &mut stream_pos,
                        &mut stream_end,
                        chunk_len,
                    )?
                {
                    return Ok(None);
                }
            } else {
                let take = chunk_len.min(palette_alpha.len());
                if take > 0
                    && !zip_entry_stream_read_exact::<_, _, _, _, _, PNG_STREAM_BYTES>(
                        file,
                        entry,
                        &mut stream_offset,
                        &mut stream_buf,
                        &mut stream_len,
                        &mut stream_pos,
                        &mut stream_end,
                        &mut palette_alpha[..take],
                    )?
                {
                    return Ok(None);
                }
                if chunk_len > take
                    && !zip_entry_stream_skip::<_, _, _, _, _, PNG_STREAM_BYTES>(
                        file,
                        entry,
                        &mut stream_offset,
                        &mut stream_buf,
                        &mut stream_len,
                        &mut stream_pos,
                        &mut stream_end,
                        chunk_len - take,
                    )?
                {
                    return Ok(None);
                }
            }
        } else if chunk_type == b"IDAT" {
            if !header_ok {
                info!("sd: png decode fail reason=idat_before_ihdr");
                return Ok(None);
            }
            if color_type == 3 && palette_entries == 0 {
                info!("sd: png decode fail reason=missing_palette");
                return Ok(None);
            }
            saw_idat = true;

            let mut remaining = chunk_len;
            while remaining > 0 {
                let take = remaining.min(idat_input.len());
                if !zip_entry_stream_read_exact::<_, _, _, _, _, PNG_STREAM_BYTES>(
                    file,
                    entry,
                    &mut stream_offset,
                    &mut stream_buf,
                    &mut stream_len,
                    &mut stream_pos,
                    &mut stream_end,
                    &mut idat_input[..take],
                )? {
                    return Ok(None);
                }
                remaining -= take;

                let mut in_pos = 0usize;
                while in_pos < take {
                    let stream = inflate(
                        &mut inflater,
                        &idat_input[in_pos..take],
                        &mut inflate_out,
                        MZFlush::None,
                    );
                    in_pos = in_pos.saturating_add(stream.bytes_consumed);
                    if stream.bytes_written > 0 {
                        for &byte in &inflate_out[..stream.bytes_written] {
                            if row_fill < row_len {
                                row_accum[row_fill] = byte;
                                row_fill += 1;
                            }
                            if row_fill == row_len {
                                let filter = row_accum[0];
                                if !apply_png_filter(
                                    filter,
                                    &row_accum[1..row_len],
                                    &row_prev[..row_raw_len],
                                    bpp,
                                    &mut row_cur[..row_raw_len],
                                ) {
                                    info!(
                                        "sd: png decode fail reason=bad_filter filter={}",
                                        filter
                                    );
                                    return Ok(None);
                                }

                                for dy in 0..th {
                                    let sy = dy.saturating_mul(src_height) / th;
                                    if sy != row_index {
                                        continue;
                                    }
                                    for dx in 0..tw {
                                        let sx = dx.saturating_mul(src_width) / tw;
                                        if mono_sample_from_row(
                                            &row_cur[..row_raw_len],
                                            sx,
                                            color_type,
                                            bit_depth,
                                            channels,
                                            &palette_rgb,
                                            &palette_alpha,
                                            palette_entries,
                                        ) {
                                            mono_set_pixel(out_bits, tw, dx, dy, true);
                                        }
                                    }
                                }

                                row_prev[..row_raw_len].copy_from_slice(&row_cur[..row_raw_len]);
                                row_fill = 0;
                                row_index = row_index.saturating_add(1);
                            }
                        }
                    }

                    match stream.status {
                        Ok(MZStatus::Ok) | Ok(MZStatus::NeedDict) | Ok(MZStatus::StreamEnd) => {}
                        Err(MZError::Buf) => {}
                        _ => {
                            info!(
                                "sd: png decode fail reason=inflate_status status={:?}",
                                stream.status
                            );
                            return Ok(None);
                        }
                    }

                    if stream.bytes_consumed == 0 && stream.bytes_written == 0 {
                        break;
                    }
                }
            }
        } else if chunk_type == b"IEND" {
            if chunk_len > 0
                && !zip_entry_stream_skip::<_, _, _, _, _, PNG_STREAM_BYTES>(
                    file,
                    entry,
                    &mut stream_offset,
                    &mut stream_buf,
                    &mut stream_len,
                    &mut stream_pos,
                    &mut stream_end,
                    chunk_len,
                )?
            {
                return Ok(None);
            }
            if !zip_entry_stream_read_exact::<_, _, _, _, _, PNG_STREAM_BYTES>(
                file,
                entry,
                &mut stream_offset,
                &mut stream_buf,
                &mut stream_len,
                &mut stream_pos,
                &mut stream_end,
                &mut chunk_crc,
            )? {
                return Ok(None);
            }
            break;
        } else if chunk_len > 0
            && !zip_entry_stream_skip::<_, _, _, _, _, PNG_STREAM_BYTES>(
                file,
                entry,
                &mut stream_offset,
                &mut stream_buf,
                &mut stream_len,
                &mut stream_pos,
                &mut stream_end,
                chunk_len,
            )?
        {
            return Ok(None);
        }

        if !zip_entry_stream_read_exact::<_, _, _, _, _, PNG_STREAM_BYTES>(
            file,
            entry,
            &mut stream_offset,
            &mut stream_buf,
            &mut stream_len,
            &mut stream_pos,
            &mut stream_end,
            &mut chunk_crc,
        )? {
            return Ok(None);
        }
    }

    if !header_ok || !saw_idat || src_width == 0 || src_height == 0 {
        info!(
            "sd: png decode fail reason=incomplete_stream header_ok={} saw_idat={} width={} height={}",
            header_ok, saw_idat, src_width, src_height
        );
        return Ok(None);
    }

    Ok(Some((src_width as u16, src_height as u16, dst_bytes)))
}
