/// Probe card + mount FAT + read a text chunk from a specific EPUB resource.
///
/// When `text_resource_hint` is empty, the first text resource discovered from
/// the EPUB spine/manifest heuristics is used.
#[allow(
    clippy::too_many_arguments,
    reason = "embedded call-site clarity; explicit bus/cs/delay/resource/offset params"
)]
pub fn probe_and_read_epub_text_chunk_from_resource<BUS, CS, DELAY, const PATH_BYTES: usize>(
    bus: &mut BUS,
    cs: &mut CS,
    delay: &mut DELAY,
    books_dir_name: &str,
    short_name: &str,
    text_resource_hint: &str,
    start_offset: u32,
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
        chapter_index: 0,
        chapter_total: 1,
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
    let selected_entry = if text_resource_hint.is_empty() {
        find_first_text_entry::<_, _, _, _, _, PATH_BYTES>(&mut file, file_size)
            .map_err(SdProbeError::Filesystem)?
    } else {
        let mut resource = String::<PATH_BYTES>::new();
        for ch in text_resource_hint.chars() {
            if resource.push(ch).is_err() {
                break;
            }
        }
        find_entry_by_path(&mut file, file_size, text_resource_hint.as_bytes())
            .map_err(SdProbeError::Filesystem)?
            .map(|entry| (entry, resource))
    };

    if let Some((entry, resource)) = selected_entry {
        result.compression = entry.compression;
        result.text_resource = resource;
        if let Ok(Some((chapter_index, chapter_total))) =
            spine_position_for_resource(&mut file, file_size, result.text_resource.as_str())
        {
            result.chapter_index = chapter_index;
            result.chapter_total = chapter_total;
        }

        if matches!(entry.compression, 0 | 8) {
            let (bytes_read, end_of_resource) =
                read_zip_entry_chunk(&mut file, entry, start_offset, out)
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

/// Probe card + mount FAT + read the first chunk from the next text resource
/// after `current_resource`.
pub fn probe_and_read_next_epub_text_chunk<BUS, CS, DELAY, const PATH_BYTES: usize>(
    bus: &mut BUS,
    cs: &mut CS,
    delay: &mut DELAY,
    books_dir_name: &str,
    short_name: &str,
    current_resource: &str,
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
        chapter_index: 0,
        chapter_total: 1,
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
    if let Some((entry, resource)) =
        find_next_text_entry::<_, _, _, _, _, PATH_BYTES>(&mut file, file_size, current_resource)
            .map_err(SdProbeError::Filesystem)?
    {
        result.compression = entry.compression;
        result.text_resource = resource;
        if let Ok(Some((chapter_index, chapter_total))) =
            spine_position_for_resource(&mut file, file_size, result.text_resource.as_str())
        {
            result.chapter_index = chapter_index;
            result.chapter_total = chapter_total;
        }

        if matches!(entry.compression, 0 | 8) {
            let (bytes_read, end_of_resource) =
                read_zip_entry_chunk(&mut file, entry, 0, out).map_err(SdProbeError::Filesystem)?;
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

/// Probe card + mount FAT + scan `/books` for ZIP signatures (EPUB candidates).
pub fn probe_and_scan_epubs<
    BUS,
    CS,
    DELAY,
    const MAX_EPUBS: usize,
    const NAME_BYTES: usize,
    const MAX_CANDIDATES: usize,
>(
    bus: &mut BUS,
    cs: &mut CS,
    delay: &mut DELAY,
    books_dir_name: &str,
) -> Result<SdEpubScanResult<MAX_EPUBS, NAME_BYTES>, SdProbeError<BUS::Error, CS::Error>>
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

    let mut scan = SdEpubScanResult {
        card_size_bytes,
        books_dir_found: false,
        scanned_file_count: 0,
        epub_count_total: 0,
        epub_entries: Vec::new(),
        truncated: false,
    };

    let mut volume_mgr = VolumeManager::new(sd_card, FixedTimeSource);
    let mut volume = volume_mgr
        .open_volume(VolumeIdx(0))
        .map_err(SdProbeError::Filesystem)?;
    let mut root_dir = volume.open_root_dir().map_err(SdProbeError::Filesystem)?;

    let mut books_dir = match root_dir.open_dir(books_dir_name) {
        Ok(dir) => {
            scan.books_dir_found = true;
            dir
        }
        Err(embedded_sdmmc::Error::NotFound) => return Ok(scan),
        Err(err) => return Err(SdProbeError::Filesystem(err)),
    };

    let mut candidates: Vec<ShortFileName, MAX_CANDIDATES> = Vec::new();
    books_dir
        .iterate_dir(|entry| {
            if entry.attributes.is_directory() || entry.attributes.is_volume() {
                return;
            }

            scan.scanned_file_count = scan.scanned_file_count.saturating_add(1);
            if candidates.push(entry.name.clone()).is_err() {
                scan.truncated = true;
            }
        })
        .map_err(SdProbeError::Filesystem)?;

    for name in candidates.iter() {
        let mut file = books_dir
            .open_file_in_dir(name, Mode::ReadOnly)
            .map_err(SdProbeError::Filesystem)?;
        let size_bytes = file.length();
        let short_name = short_file_name_to_string(name);
        let mut display_title = display_title_from_file_name(short_name.as_str());
        let mut has_cover = false;

        let mut header = [0u8; 4];
        let read_now = file.read(&mut header).map_err(SdProbeError::Filesystem)?;

        if has_zip_signature(header, read_now)
            && let Ok(metadata) = scan_epub_metadata(&mut file, size_bytes, &mut display_title)
        {
            has_cover = metadata.has_cover;
        }

        file.close().map_err(SdProbeError::Filesystem)?;

        if !has_zip_signature(header, read_now) {
            continue;
        }

        scan.epub_count_total = scan.epub_count_total.saturating_add(1);
        let entry = SdEpubEntry {
            short_name,
            display_title,
            has_cover,
            size_bytes,
        };
        if scan.epub_entries.push(entry).is_err() {
            scan.truncated = true;
        }
    }

    books_dir.close().map_err(SdProbeError::Filesystem)?;
    root_dir.close().map_err(SdProbeError::Filesystem)?;
    volume.close().map_err(SdProbeError::Filesystem)?;

    Ok(scan)
}

/// Probe card + mount FAT + read one fixed root file into `out`.
pub fn probe_and_read_test_file<BUS, CS, DELAY>(
    bus: &mut BUS,
    cs: &mut CS,
    delay: &mut DELAY,
    test_file_name: &str,
    out: &mut [u8],
) -> Result<SdProbeResult, SdProbeError<BUS::Error, CS::Error>>
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

    let mut volume_mgr = VolumeManager::new(sd_card, FixedTimeSource);
    let mut volume = volume_mgr
        .open_volume(VolumeIdx(0))
        .map_err(SdProbeError::Filesystem)?;
    let mut root_dir = volume.open_root_dir().map_err(SdProbeError::Filesystem)?;
    let mut file = root_dir
        .open_file_in_dir(test_file_name, Mode::ReadOnly)
        .map_err(SdProbeError::Filesystem)?;

    let mut read_total = 0usize;
    while read_total < out.len() {
        let read_now = file
            .read(&mut out[read_total..])
            .map_err(SdProbeError::Filesystem)?;
        if read_now == 0 || file.is_eof() {
            break;
        }
        read_total = read_total.saturating_add(read_now);
    }

    file.close().map_err(SdProbeError::Filesystem)?;
    root_dir.close().map_err(SdProbeError::Filesystem)?;
    volume.close().map_err(SdProbeError::Filesystem)?;

    Ok(SdProbeResult {
        card_size_bytes,
        bytes_read: read_total,
    })
}
