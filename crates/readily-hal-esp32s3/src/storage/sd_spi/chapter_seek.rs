const ANCHOR_SCAN_CHUNK_BYTES: usize = ZIP_DEFLATE_OUTPUT_BYTES;
const ANCHOR_SCAN_TAIL_BYTES: usize = ZIP_PATH_BYTES + 48;
const ANCHOR_ATTR_SEARCH_BACK_BYTES: usize = 20;

#[derive(Debug)]
struct AnchorScanner {
    fragment: String<ZIP_PATH_BYTES>,
    tail: Vec<u8, ANCHOR_SCAN_TAIL_BYTES>,
    consumed: usize,
}

impl AnchorScanner {
    fn new(fragment: &str) -> Option<Self> {
        let mut normalized = String::<ZIP_PATH_BYTES>::new();
        if !decode_fragment_for_match(fragment, &mut normalized) {
            return None;
        }
        Some(Self {
            fragment: normalized,
            tail: Vec::new(),
            consumed: 0,
        })
    }

    fn feed(&mut self, chunk: &[u8]) -> Option<u32> {
        if chunk.is_empty() {
            return None;
        }

        let mut merged = [0u8; ANCHOR_SCAN_TAIL_BYTES + ANCHOR_SCAN_CHUNK_BYTES];
        let mut merged_len = 0usize;

        for &byte in self.tail.iter() {
            if merged_len >= merged.len() {
                break;
            }
            merged[merged_len] = byte;
            merged_len = merged_len.saturating_add(1);
        }
        for &byte in chunk {
            if merged_len >= merged.len() {
                break;
            }
            merged[merged_len] = byte.to_ascii_lowercase();
            merged_len = merged_len.saturating_add(1);
        }

        if merged_len >= self.fragment.len() {
            let max_start = merged_len - self.fragment.len();
            for idx in 0..=max_start {
                if !eq_ascii_case_insensitive(
                    &merged[idx..idx + self.fragment.len()],
                    self.fragment.as_bytes(),
                ) {
                    continue;
                }
                if !looks_like_anchor_attribute(&merged[..merged_len], idx, self.fragment.len()) {
                    continue;
                }

                let anchor_local = nearest_tag_start(&merged[..merged_len], idx);
                let base = self.consumed.saturating_sub(self.tail.len());
                let absolute = base.saturating_add(anchor_local);
                return Some(absolute.min(u32::MAX as usize) as u32);
            }
        }

        self.consumed = self.consumed.saturating_add(chunk.len());

        self.tail.clear();
        let keep = ANCHOR_SCAN_TAIL_BYTES.min(merged_len);
        let start = merged_len.saturating_sub(keep);
        for &byte in &merged[start..merged_len] {
            if self.tail.push(byte).is_err() {
                break;
            }
        }

        None
    }
}

fn decode_fragment_for_match<const N: usize>(fragment: &str, out: &mut String<N>) -> bool {
    out.clear();
    let source = fragment.trim_start_matches('#').trim();
    if source.is_empty() {
        return false;
    }

    let bytes = source.as_bytes();
    let mut idx = 0usize;
    while idx < bytes.len() {
        let byte = bytes[idx];
        if byte == b'%' && idx + 2 < bytes.len() {
            let hi = bytes[idx + 1];
            let lo = bytes[idx + 2];
            if let (Some(left), Some(right)) = (hex_nibble(hi), hex_nibble(lo)) {
                let value = (left << 4) | right;
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

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn nearest_tag_start(haystack: &[u8], from: usize) -> usize {
    let start = from.saturating_sub(192);
    haystack[start..from]
        .iter()
        .rposition(|byte| *byte == b'<')
        .map(|pos| start.saturating_add(pos))
        .unwrap_or(from.saturating_sub(24))
}

fn looks_like_anchor_attribute(haystack: &[u8], match_start: usize, fragment_len: usize) -> bool {
    if fragment_len == 0 {
        return false;
    }
    if match_start == 0 || match_start + fragment_len >= haystack.len() {
        return false;
    }

    let quote = haystack[match_start - 1];
    if quote != b'"' && quote != b'\'' {
        return false;
    }
    if haystack[match_start + fragment_len] != quote {
        return false;
    }

    let attr_start = match_start.saturating_sub(ANCHOR_ATTR_SEARCH_BACK_BYTES);
    let attr_bytes = &haystack[attr_start..match_start];
    contains_ascii_case_insensitive(attr_bytes, b"id=")
        || contains_ascii_case_insensitive(attr_bytes, b"xml:id=")
        || contains_ascii_case_insensitive(attr_bytes, b"name=")
}

fn find_fragment_offset_in_stored_entry<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    entry: ZipEntryRef,
    fragment: &str,
) -> Result<Option<u32>, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let Some(data_offset) = entry_data_offset(file, entry)? else {
        return Ok(None);
    };

    let Some(mut scanner) = AnchorScanner::new(fragment) else {
        return Ok(None);
    };

    file.seek_from_start(data_offset)?;
    let mut chunk = [0u8; ANCHOR_SCAN_CHUNK_BYTES];
    loop {
        let read_now = file.read(&mut chunk)?;
        if read_now == 0 {
            break;
        }
        if let Some(offset) = scanner.feed(&chunk[..read_now]) {
            return Ok(Some(offset));
        }
    }

    Ok(None)
}

fn find_fragment_offset_in_deflated_entry<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    entry: ZipEntryRef,
    fragment: &str,
) -> Result<Option<u32>, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    if entry.compressed_size == 0 {
        return Ok(None);
    }
    let Some(data_offset) = entry_data_offset(file, entry)? else {
        return Ok(None);
    };
    let Some(mut scanner) = AnchorScanner::new(fragment) else {
        return Ok(None);
    };

    file.seek_from_start(data_offset)?;

    let mut inflater = InflateState::new(DataFormat::Raw);
    let mut input = [0u8; ZIP_DEFLATE_INPUT_BYTES];
    let mut output = [0u8; ZIP_DEFLATE_OUTPUT_BYTES];
    let mut input_len = 0usize;
    let mut input_pos = 0usize;
    let mut compressed_remaining = entry.compressed_size;
    let mut no_progress_loops = 0u8;

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

        if stream.bytes_written > 0
            && let Some(offset) = scanner.feed(&output[..stream.bytes_written])
        {
            return Ok(Some(offset));
        }

        match stream.status {
            Ok(MZStatus::StreamEnd) => break,
            Ok(MZStatus::Ok) => {}
            Err(MZError::Buf) => {
                if compressed_remaining == 0 && input_pos >= input_len {
                    break;
                }
            }
            _ => return Ok(None),
        }

        if compressed_remaining == 0 && input_pos >= input_len && stream.bytes_written == 0 {
            break;
        }
    }

    Ok(None)
}

fn find_fragment_offset_in_zip_entry<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    entry: ZipEntryRef,
    fragment: &str,
) -> Result<Option<u32>, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    if fragment.trim().is_empty() {
        return Ok(None);
    }

    match entry.compression {
        0 => find_fragment_offset_in_stored_entry(file, entry, fragment),
        8 => find_fragment_offset_in_deflated_entry(file, entry, fragment),
        _ => Ok(None),
    }
}
