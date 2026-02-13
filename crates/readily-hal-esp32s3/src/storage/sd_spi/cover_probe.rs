fn decode_cover_entry_thumbnail<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    entry: ZipEntryRef,
    media: &[u8],
    path: &[u8],
    thumb_width: u16,
    thumb_height: u16,
    out: &mut [u8],
) -> Result<(SdEpubCoverStatus, u16, u16, usize), embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let mut media_is_pbm = is_cover_media_pbm(media, path);
    let mut media_is_png = is_cover_media_png(media, path);
    let mut media_is_jpeg = is_cover_media_jpeg(media, path);
    if !media_is_pbm && !media_is_png && !media_is_jpeg {
        let mut sniff = [0u8; 16];
        let sniff_read = read_zip_entry_prefix(file, entry, &mut sniff)?;
        if sniff_read > 0 {
            let (pbm_guess, png_guess, jpeg_guess) = sniff_cover_media_flags(&sniff[..sniff_read]);
            media_is_pbm = pbm_guess;
            media_is_png = png_guess;
            media_is_jpeg = jpeg_guess;
        }
        if !media_is_pbm && !media_is_png && !media_is_jpeg {
            return Ok((SdEpubCoverStatus::UnsupportedMediaType, 0, 0, 0));
        }
    }

    if media_is_pbm {
        let mut cover_bytes = [0u8; ZIP_COVER_BYTES];
        let read_cover = read_zip_entry_prefix(file, entry, &mut cover_bytes)?;
        if read_cover > 0
            && let Some((source_width, source_height, bytes_written)) =
                decode_pbm_thumbnail_p4(&cover_bytes[..read_cover], thumb_width, thumb_height, out)
        {
            return Ok((
                SdEpubCoverStatus::ReadOk,
                source_width,
                source_height,
                bytes_written,
            ));
        }
        return Ok((SdEpubCoverStatus::DecodeFailed, 0, 0, 0));
    }

    if media_is_png {
        if let Some((source_width, source_height, bytes_written)) =
            decode_png_thumbnail_stream(file, entry, thumb_width, thumb_height, out)?
        {
            return Ok((
                SdEpubCoverStatus::ReadOk,
                source_width,
                source_height,
                bytes_written,
            ));
        }
        return Ok((SdEpubCoverStatus::DecodeFailed, 0, 0, 0));
    }

    if let Some((source_width, source_height, bytes_written)) =
        decode_jpeg_thumbnail_stream(file, entry, thumb_width, thumb_height, out)?
    {
        return Ok((
            SdEpubCoverStatus::ReadOk,
            source_width,
            source_height,
            bytes_written,
        ));
    }
    Ok((SdEpubCoverStatus::DecodeFailed, 0, 0, 0))
}

/// Probe card + mount FAT + decode EPUB cover into a 1bpp thumbnail buffer.
///
/// Current decoder support:
/// - `image/x-portable-bitmap` / `.pbm` (binary `P4`)
/// - `image/png` / `.png` (non-interlaced)
/// - `image/jpeg` / `.jpg` / `.jpeg` (ESP32-S3 ROM TJPGD path)
///
/// When cover metadata points to an XHTML/HTML resource, the first `<img src>`
/// is followed before decoding.
#[allow(
    clippy::too_many_arguments,
    reason = "embedded call-site clarity; explicit bus/cs/delay/book/thumb params"
)]
pub fn probe_and_read_epub_cover_thumbnail<
    BUS,
    CS,
    DELAY,
    const PATH_BYTES: usize,
    const MEDIA_BYTES: usize,
>(
    bus: &mut BUS,
    cs: &mut CS,
    delay: &mut DELAY,
    books_dir_name: &str,
    short_name: &str,
    thumb_width: u16,
    thumb_height: u16,
    out: &mut [u8],
) -> Result<SdEpubCoverResult<PATH_BYTES, MEDIA_BYTES>, SdProbeError<BUS::Error, CS::Error>>
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

    let mut result = SdEpubCoverResult {
        card_size_bytes,
        cover_resource: String::new(),
        media_type: String::new(),
        source_width: 0,
        source_height: 0,
        thumb_width: thumb_width.max(1),
        thumb_height: thumb_height.max(1),
        bytes_written: 0,
        status: SdEpubCoverStatus::NoCoverResource,
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
        result.status = SdEpubCoverStatus::NotZip;
        file.close().map_err(SdProbeError::Filesystem)?;
        books_dir.close().map_err(SdProbeError::Filesystem)?;
        root_dir.close().map_err(SdProbeError::Filesystem)?;
        volume.close().map_err(SdProbeError::Filesystem)?;
        return Ok(result);
    }

    let file_size = file.length();
    let mut opf_buf = [0u8; ZIP_OPF_BYTES];
    let mut cover_path = String::<PATH_BYTES>::new();
    let mut cover_media = String::<MEDIA_BYTES>::new();
    let Some((opf_entry, opf_path)) =
        find_opf_entry_and_path::<_, _, _, _, _, PATH_BYTES>(&mut file, file_size)
            .map_err(SdProbeError::Filesystem)?
    else {
        file.close().map_err(SdProbeError::Filesystem)?;
        books_dir.close().map_err(SdProbeError::Filesystem)?;
        root_dir.close().map_err(SdProbeError::Filesystem)?;
        volume.close().map_err(SdProbeError::Filesystem)?;
        return Ok(result);
    };
    let opf_read = read_zip_entry_prefix(&mut file, opf_entry, &mut opf_buf)
        .map_err(SdProbeError::Filesystem)?;
    if opf_read == 0 {
        file.close().map_err(SdProbeError::Filesystem)?;
        books_dir.close().map_err(SdProbeError::Filesystem)?;
        root_dir.close().map_err(SdProbeError::Filesystem)?;
        volume.close().map_err(SdProbeError::Filesystem)?;
        return Ok(result);
    }

    let mut parsed_cover = parse_opf_cover_resource(
        &opf_buf[..opf_read],
        opf_path.as_str(),
        &mut cover_path,
        &mut cover_media,
    );

    let mut cover_entry = if parsed_cover {
        find_entry_by_path(&mut file, file_size, cover_path.as_bytes())
            .map_err(SdProbeError::Filesystem)?
    } else {
        None
    };
    if cover_entry.is_none()
        && let Some((fallback_entry, fallback_path, fallback_media)) =
            find_fallback_cover_image_entry::<_, _, _, _, _, PATH_BYTES, MEDIA_BYTES>(
                &mut file, file_size, None,
            )
            .map_err(SdProbeError::Filesystem)?
    {
        parsed_cover = true;
        cover_entry = Some(fallback_entry);
        cover_path = fallback_path;
        cover_media = fallback_media;
    }

    let Some(mut cover_entry) = cover_entry else {
        if parsed_cover {
            result.cover_resource = cover_path;
            result.media_type = cover_media;
            result.status = SdEpubCoverStatus::DecodeFailed;
        }
        file.close().map_err(SdProbeError::Filesystem)?;
        books_dir.close().map_err(SdProbeError::Filesystem)?;
        root_dir.close().map_err(SdProbeError::Filesystem)?;
        volume.close().map_err(SdProbeError::Filesystem)?;
        return Ok(result);
    };

    if is_text_media_document(cover_media.as_bytes(), cover_path.as_bytes()) {
        let mut cover_doc = [0u8; ZIP_CONTAINER_BYTES];
        let read_cover_doc = read_zip_entry_prefix(&mut file, cover_entry, &mut cover_doc)
            .map_err(SdProbeError::Filesystem)?;
        let mut nested_cover_path = String::<PATH_BYTES>::new();
        if read_cover_doc > 0
            && parse_html_first_img_src(
                &cover_doc[..read_cover_doc],
                cover_path.as_str(),
                &mut nested_cover_path,
            )
            && let Some(nested_entry) =
                find_entry_by_path(&mut file, file_size, nested_cover_path.as_bytes())
                    .map_err(SdProbeError::Filesystem)?
        {
            cover_entry = nested_entry;
            cover_path = nested_cover_path;
            copy_media_type_or_default(None, cover_path.as_bytes(), &mut cover_media);
        }
    }

    result.cover_resource = cover_path;
    result.media_type = cover_media;

    let (status, source_width, source_height, bytes_written) = decode_cover_entry_thumbnail(
        &mut file,
        cover_entry,
        result.media_type.as_bytes(),
        result.cover_resource.as_bytes(),
        result.thumb_width,
        result.thumb_height,
        out,
    )
    .map_err(SdProbeError::Filesystem)?;
    result.status = status;
    if status == SdEpubCoverStatus::ReadOk {
        result.source_width = source_width;
        result.source_height = source_height;
        result.bytes_written = bytes_written;
    } else {
        let mut fallback_attempts = 0usize;
        for skip_count in 0..16usize {
            let Some((fallback_entry, fallback_path, fallback_media)) =
                find_fallback_cover_image_entry_with_skip::<
                    _,
                    _,
                    _,
                    _,
                    _,
                    PATH_BYTES,
                    MEDIA_BYTES,
                >(
                    &mut file,
                    file_size,
                    Some(result.cover_resource.as_bytes()),
                    skip_count,
                )
                .map_err(SdProbeError::Filesystem)?
            else {
                break;
            };

            fallback_attempts = fallback_attempts.saturating_add(1);
            info!(
                "sd: cover fallback candidate path={} media={} order={}",
                fallback_path.as_str(),
                fallback_media.as_str(),
                skip_count.saturating_add(1)
            );
            let (fallback_status, fallback_sw, fallback_sh, fallback_bytes) =
                decode_cover_entry_thumbnail(
                    &mut file,
                    fallback_entry,
                    fallback_media.as_bytes(),
                    fallback_path.as_bytes(),
                    result.thumb_width,
                    result.thumb_height,
                    out,
                )
                .map_err(SdProbeError::Filesystem)?;
            if fallback_status == SdEpubCoverStatus::ReadOk {
                result.cover_resource = fallback_path;
                result.media_type = fallback_media;
                result.status = fallback_status;
                result.source_width = fallback_sw;
                result.source_height = fallback_sh;
                result.bytes_written = fallback_bytes;
                info!(
                    "sd: cover fallback applied resource={} media={} source={}x{} bytes={}",
                    result.cover_resource.as_str(),
                    result.media_type.as_str(),
                    result.source_width,
                    result.source_height,
                    result.bytes_written
                );
                break;
            }

            info!(
                "sd: cover fallback decode_failed path={} media={} status={:?}",
                fallback_path.as_str(),
                fallback_media.as_str(),
                fallback_status
            );
        }

        if result.status != SdEpubCoverStatus::ReadOk {
            if fallback_attempts == 0 {
                info!(
                    "sd: cover fallback none base_resource={} base_media={}",
                    result.cover_resource.as_str(),
                    result.media_type.as_str()
                );
            } else {
                info!(
                    "sd: cover fallback exhausted base_resource={} attempts={}",
                    result.cover_resource.as_str(),
                    fallback_attempts
                );
            }
        }
    }

    file.close().map_err(SdProbeError::Filesystem)?;
    books_dir.close().map_err(SdProbeError::Filesystem)?;
    root_dir.close().map_err(SdProbeError::Filesystem)?;
    volume.close().map_err(SdProbeError::Filesystem)?;

    Ok(result)
}
