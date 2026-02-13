use core::fmt::Debug;

use embassy_time::Timer;
use embedded_hal::{delay::DelayNs, digital::OutputPin, spi::SpiBus};
use heapless::{String as HeaplessString, Vec as HeaplessVec};
use log::info;
use readily_core::content::sd_catalog::SdCatalogSource;
use readily_hal_esp32s3::{
    render::rsvp::RsvpRenderer,
    storage::sd_spi::{
        SdEpubCoverStatus, SdEpubTextChunkStatus, SdProbeError,
        probe_and_read_epub_cover_thumbnail, probe_and_read_epub_text_chunk, probe_and_scan_epubs,
    },
};

use super::{
    SD_BOOKS_DIR, SD_COVER_MEDIA_BYTES, SD_COVER_THUMB_BYTES, SD_COVER_THUMB_HEIGHT,
    SD_COVER_THUMB_WIDTH, SD_PROBE_ATTEMPTS, SD_PROBE_RETRY_DELAY_MS, SD_SCAN_MAX_CANDIDATES,
    SD_SCAN_MAX_EPUBS, SD_SCAN_NAME_BYTES, SD_SPI_HZ_CANDIDATES, SD_TEXT_CHUNK_BYTES,
    SD_TEXT_PATH_BYTES, SD_TEXT_PREVIEW_BYTES, SdBookStreamState,
};

pub(super) async fn preload_initial_catalog<BUS, CS, DELAY, F>(
    content: &mut SdCatalogSource,
    renderer: &mut RsvpRenderer,
    sd_stream_states: &mut HeaplessVec<SdBookStreamState, SD_SCAN_MAX_EPUBS>,
    sd_spi: &mut BUS,
    sd_cs: &mut CS,
    sd_delay: &mut DELAY,
    start_speed_index: usize,
    mut try_set_speed: F,
) -> usize
where
    BUS: SpiBus<u8>,
    CS: OutputPin,
    DELAY: DelayNs,
    BUS::Error: Debug,
    CS::Error: Debug,
    F: FnMut(&mut BUS, usize) -> bool,
{
    let mut sd_spi_speed_index = start_speed_index;
    let mut initial_scan_result = None;
    'boot_speed_scan: for speed_index in sd_spi_speed_index..SD_SPI_HZ_CANDIDATES.len() {
        let speed_hz = SD_SPI_HZ_CANDIDATES[speed_index];

        if !try_set_speed(sd_spi, speed_index) {
            info!(
                "sd: initial catalog spi config failed (spi_hz={})",
                speed_hz
            );
            continue;
        }

        for attempt in 1..=SD_PROBE_ATTEMPTS {
            match probe_and_scan_epubs::<
                _,
                _,
                _,
                SD_SCAN_MAX_EPUBS,
                SD_SCAN_NAME_BYTES,
                SD_SCAN_MAX_CANDIDATES,
            >(sd_spi, sd_cs, sd_delay, SD_BOOKS_DIR)
            {
                Ok(scan) => {
                    sd_spi_speed_index = speed_index;
                    initial_scan_result = Some(scan);
                    info!(
                        "sd: initial catalog probe ok (attempt={} spi_hz={})",
                        attempt, speed_hz
                    );
                    break 'boot_speed_scan;
                }
                Err(err) => {
                    match &err {
                        SdProbeError::ChipSelect(_) => info!(
                            "sd: initial catalog probe failed attempt={} spi_hz={} (chip-select pin)",
                            attempt, speed_hz
                        ),
                        SdProbeError::Spi(_) => info!(
                            "sd: initial catalog probe failed attempt={} spi_hz={} (spi transfer)",
                            attempt, speed_hz
                        ),
                        SdProbeError::Card(card_err) => info!(
                            "sd: initial catalog probe failed attempt={} spi_hz={} (card init): {:?}",
                            attempt, speed_hz, card_err
                        ),
                        SdProbeError::Filesystem(fs_err) => info!(
                            "sd: initial catalog probe failed attempt={} spi_hz={} (filesystem): {:?}",
                            attempt, speed_hz, fs_err
                        ),
                    }

                    if attempt < SD_PROBE_ATTEMPTS {
                        Timer::after_millis(SD_PROBE_RETRY_DELAY_MS).await;
                    }
                }
            }
        }
    }

    match initial_scan_result {
        Some(scan) => {
            if scan.books_dir_found {
                let catalog_load = content.set_catalog_entries_from_iter(
                    scan.epub_entries
                        .iter()
                        .map(|entry| (entry.display_title.as_str(), entry.has_cover)),
                );
                info!(
                    "sd: initial catalog loaded card_bytes={} books_dir={} epub_total={} listed={} titles_loaded={} scan_truncated={} title_truncated={} spi_hz={}",
                    scan.card_size_bytes,
                    SD_BOOKS_DIR,
                    scan.epub_count_total,
                    scan.epub_entries.len(),
                    catalog_load.loaded,
                    scan.truncated,
                    catalog_load.truncated,
                    SD_SPI_HZ_CANDIDATES[sd_spi_speed_index]
                );
                if catalog_load.loaded == 0 {
                    info!("sd: initial catalog has no EPUB titles");
                } else {
                    let mut text_chunks_loaded = 0u16;
                    let mut text_chunks_truncated = 0u16;
                    let mut covers_loaded = 0u16;
                    sd_stream_states.clear();

                    for (index, epub) in scan
                        .epub_entries
                        .iter()
                        .take(catalog_load.loaded as usize)
                        .enumerate()
                    {
                        let mut stream_state = SdBookStreamState {
                            short_name: epub.short_name.clone(),
                            text_resource: HeaplessString::new(),
                            next_offset: 0,
                            end_of_resource: true,
                            ready: false,
                        };

                        let mut text_chunk = [0u8; SD_TEXT_CHUNK_BYTES];
                        match probe_and_read_epub_text_chunk::<_, _, _, SD_TEXT_PATH_BYTES>(
                            sd_spi,
                            sd_cs,
                            sd_delay,
                            SD_BOOKS_DIR,
                            epub.short_name.as_str(),
                            &mut text_chunk,
                        ) {
                            Ok(text_probe) => match text_probe.status {
                                SdEpubTextChunkStatus::ReadOk => {
                                    let preview_len =
                                        text_probe.bytes_read.min(SD_TEXT_PREVIEW_BYTES);
                                    let preview = core::str::from_utf8(&text_chunk[..preview_len])
                                        .unwrap_or("");
                                    for ch in text_probe.text_resource.chars() {
                                        if stream_state.text_resource.push(ch).is_err() {
                                            break;
                                        }
                                    }
                                    stream_state.next_offset = text_probe
                                        .start_offset
                                        .saturating_add(text_probe.bytes_read as u32);
                                    stream_state.end_of_resource = text_probe.end_of_resource;
                                    stream_state.ready = !stream_state.text_resource.is_empty();
                                    match content.set_catalog_text_chunk_from_bytes(
                                        index as u16,
                                        &text_chunk[..text_probe.bytes_read],
                                        text_probe.end_of_resource,
                                        text_probe.text_resource.as_str(),
                                    ) {
                                        Ok(applied) => {
                                            let _ = content.set_catalog_stream_chapter_metadata(
                                                index as u16,
                                                text_probe.chapter_index,
                                                text_probe.chapter_total,
                                                Some(text_probe.chapter_label.as_str()),
                                            );
                                            if applied.loaded {
                                                text_chunks_loaded =
                                                    text_chunks_loaded.saturating_add(1);
                                            }
                                            if applied.truncated {
                                                text_chunks_truncated =
                                                    text_chunks_truncated.saturating_add(1);
                                            }
                                            info!(
                                                "sd: initial text chunk short_name={} resource={} chapter={}/{} chapter_label={:?} start_offset={} compression={} bytes_read={} end={} applied_loaded={} applied_truncated={} preview={:?}",
                                                epub.short_name,
                                                text_probe.text_resource,
                                                text_probe.chapter_index.saturating_add(1),
                                                text_probe.chapter_total.max(1),
                                                text_probe.chapter_label.as_str(),
                                                text_probe.start_offset,
                                                text_probe.compression,
                                                text_probe.bytes_read,
                                                text_probe.end_of_resource,
                                                applied.loaded,
                                                applied.truncated,
                                                preview
                                            );
                                        }
                                        Err(_) => {
                                            info!(
                                                "sd: initial text chunk ignored short_name={} status=invalid_catalog_index",
                                                epub.short_name
                                            );
                                        }
                                    }
                                }
                                SdEpubTextChunkStatus::NotZip => {
                                    info!(
                                        "sd: initial text chunk skipped short_name={} status=not_zip",
                                        epub.short_name
                                    );
                                }
                                SdEpubTextChunkStatus::NoTextResource => {
                                    info!(
                                        "sd: initial text chunk missing short_name={} status=no_text_resource",
                                        epub.short_name
                                    );
                                }
                                SdEpubTextChunkStatus::UnsupportedCompression => {
                                    info!(
                                        "sd: initial text chunk unsupported short_name={} resource={} compression={}",
                                        epub.short_name,
                                        text_probe.text_resource,
                                        text_probe.compression
                                    );
                                }
                                SdEpubTextChunkStatus::DecodeFailed => {
                                    info!(
                                        "sd: initial text chunk decode_failed short_name={} resource={} compression={}",
                                        epub.short_name,
                                        text_probe.text_resource,
                                        text_probe.compression
                                    );
                                }
                            },
                            Err(err) => match err {
                                SdProbeError::ChipSelect(_) => {
                                    info!("sd: initial text chunk failed (chip-select pin)")
                                }
                                SdProbeError::Spi(_) => {
                                    info!("sd: initial text chunk failed (spi transfer)")
                                }
                                SdProbeError::Card(card_err) => {
                                    info!(
                                        "sd: initial text chunk failed (card init): {:?}",
                                        card_err
                                    )
                                }
                                SdProbeError::Filesystem(fs_err) => {
                                    info!(
                                        "sd: initial text chunk failed (filesystem): {:?}",
                                        fs_err
                                    )
                                }
                            },
                        }

                        let mut cover_thumb = [0u8; SD_COVER_THUMB_BYTES];
                        match probe_and_read_epub_cover_thumbnail::<
                            _,
                            _,
                            _,
                            SD_TEXT_PATH_BYTES,
                            SD_COVER_MEDIA_BYTES,
                        >(
                            sd_spi,
                            sd_cs,
                            sd_delay,
                            SD_BOOKS_DIR,
                            epub.short_name.as_str(),
                            SD_COVER_THUMB_WIDTH,
                            SD_COVER_THUMB_HEIGHT,
                            &mut cover_thumb,
                        ) {
                            Ok(cover_probe) => match cover_probe.status {
                                SdEpubCoverStatus::ReadOk => {
                                    let applied = renderer.set_cover_thumbnail(
                                        index as u16,
                                        cover_probe.thumb_width,
                                        cover_probe.thumb_height,
                                        &cover_thumb[..cover_probe.bytes_written],
                                    );
                                    if applied {
                                        covers_loaded = covers_loaded.saturating_add(1);
                                    }
                                    info!(
                                        "sd: initial cover short_name={} resource={} media={} source={}x{} thumb={}x{} bytes={} applied={}",
                                        epub.short_name,
                                        cover_probe.cover_resource,
                                        cover_probe.media_type,
                                        cover_probe.source_width,
                                        cover_probe.source_height,
                                        cover_probe.thumb_width,
                                        cover_probe.thumb_height,
                                        cover_probe.bytes_written,
                                        applied
                                    );
                                }
                                SdEpubCoverStatus::NoCoverResource => {
                                    info!(
                                        "sd: initial cover missing short_name={} status=no_cover_resource",
                                        epub.short_name
                                    );
                                }
                                SdEpubCoverStatus::UnsupportedMediaType => {
                                    info!(
                                        "sd: initial cover unsupported short_name={} resource={} media={}",
                                        epub.short_name,
                                        cover_probe.cover_resource,
                                        cover_probe.media_type
                                    );
                                }
                                SdEpubCoverStatus::DecodeFailed => {
                                    info!(
                                        "sd: initial cover decode_failed short_name={} resource={} media={}",
                                        epub.short_name,
                                        cover_probe.cover_resource,
                                        cover_probe.media_type
                                    );
                                }
                                SdEpubCoverStatus::NotZip => {
                                    info!(
                                        "sd: initial cover skipped short_name={} status=not_zip",
                                        epub.short_name
                                    );
                                }
                            },
                            Err(err) => match err {
                                SdProbeError::ChipSelect(_) => {
                                    info!("sd: initial cover failed (chip-select pin)")
                                }
                                SdProbeError::Spi(_) => {
                                    info!("sd: initial cover failed (spi transfer)")
                                }
                                SdProbeError::Card(card_err) => {
                                    info!("sd: initial cover failed (card init): {:?}", card_err)
                                }
                                SdProbeError::Filesystem(fs_err) => {
                                    info!("sd: initial cover failed (filesystem): {:?}", fs_err)
                                }
                            },
                        }

                        if sd_stream_states.push(stream_state).is_err() {
                            info!("sd: initial stream-state list truncated at index={}", index);
                            break;
                        }
                    }

                    info!(
                        "sd: initial text chunks applied loaded={} truncated={} covers_loaded={}",
                        text_chunks_loaded, text_chunks_truncated, covers_loaded
                    );
                }
            } else {
                info!(
                    "sd: initial catalog fallback to built-in titles; books_dir={} missing",
                    SD_BOOKS_DIR
                );
            }
        }
        None => {
            info!(
                "sd: initial catalog fallback after trying all spi_hz candidates ({}, {}, {}, {})",
                SD_SPI_HZ_CANDIDATES[0],
                SD_SPI_HZ_CANDIDATES[1],
                SD_SPI_HZ_CANDIDATES[2],
                SD_SPI_HZ_CANDIDATES[3]
            );
        }
    }

    sd_spi_speed_index
}
