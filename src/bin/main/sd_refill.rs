use core::fmt::Debug;

use embedded_hal::{delay::DelayNs, digital::OutputPin, spi::SpiBus};
use heapless::{String as HeaplessString, Vec as HeaplessVec};
use log::{debug, info};
use readily_core::{
    app::ReaderApp,
    content::{
        NavigationCatalog,
        sd_catalog::{SdCatalogError, SdCatalogSource},
    },
    input::InputProvider,
};
use readily_hal_esp32s3::storage::sd_spi::{
    SdEpubTextChunkResult, SdEpubTextChunkStatus, SdProbeError,
    probe_and_read_epub_text_chunk_at_chapter, probe_and_read_epub_text_chunk_from_resource,
    probe_and_read_next_epub_text_chunk,
};

use super::{
    SD_BOOKS_DIR, SD_SCAN_MAX_EPUBS, SD_SPI_HZ_CANDIDATES, SD_TEXT_CHUNK_BYTES, SD_TEXT_PATH_BYTES,
    SdBookStreamState,
};

const SD_REFILL_SEEK_ATTEMPTS: u8 = 3;
const SD_REFILL_SEEK_RETRY_DELAY_MS: u32 = 24;

pub(super) fn handle_pending_refill<IN, BUS, CS, DELAY, F>(
    app: &mut ReaderApp<SdCatalogSource, IN>,
    sd_stream_states: &mut HeaplessVec<SdBookStreamState, SD_SCAN_MAX_EPUBS>,
    sd_spi: &mut BUS,
    sd_cs: &mut CS,
    sd_delay: &mut DELAY,
    sd_spi_speed_index: usize,
    mut try_set_speed: F,
) where
    IN: InputProvider,
    BUS: SpiBus<u8>,
    CS: OutputPin,
    DELAY: DelayNs,
    BUS::Error: Debug,
    CS::Error: Debug,
    F: FnMut(&mut BUS, usize) -> bool,
{
    let Some(refill_request) = app.with_content_mut(|content| content.take_chunk_refill_request())
    else {
        return;
    };

    let book_index = refill_request.book_index;
    let seek_target_chapter = refill_request.target_chapter;
    debug!(
        "sd: refill dispatch requested book_index={} seek_target_chapter={:?} known_stream_states={}",
        book_index,
        seek_target_chapter.map(|chapter| chapter.saturating_add(1)),
        sd_stream_states.len()
    );

    let Some(stream_state) = sd_stream_states.get_mut(book_index as usize) else {
        info!(
            "sd: refill dispatch marking exhausted book_index={} reason=stream_state_missing",
            book_index
        );
        let _ = app.with_content_mut(|content| content.mark_catalog_stream_exhausted(book_index));
        return;
    };

    debug!(
        "sd: refill dispatch state short_name={} path={} offset={} end_of_resource={} ready={}",
        stream_state.short_name,
        stream_state.text_resource,
        stream_state.next_offset,
        stream_state.end_of_resource,
        stream_state.ready
    );

    if !(stream_state.ready || seek_target_chapter.is_some()) {
        info!(
            "sd: refill dispatch marking exhausted book_index={} short_name={} reason=stream_state_not_ready path={} offset={} end_of_resource={}",
            book_index,
            stream_state.short_name,
            stream_state.text_resource,
            stream_state.next_offset,
            stream_state.end_of_resource
        );
        let _ = app.with_content_mut(|content| content.mark_catalog_stream_exhausted(book_index));
        return;
    }

    if !try_set_speed(sd_spi, sd_spi_speed_index) {
        info!(
            "sd: refill failed (spi config) spi_hz={}",
            SD_SPI_HZ_CANDIDATES[sd_spi_speed_index]
        );
        let _ = app.with_content_mut(|content| content.mark_catalog_stream_exhausted(book_index));
        return;
    }

    let mut text_chunk = [0u8; SD_TEXT_CHUNK_BYTES];
    let mut moving_to_next_resource = stream_state.end_of_resource;
    let mut current_resource = HeaplessString::<SD_TEXT_PATH_BYTES>::new();
    for ch in stream_state.text_resource.chars() {
        if current_resource.push(ch).is_err() {
            break;
        }
    }

    let mut selected_probe: Option<(SdEpubTextChunkResult<SD_TEXT_PATH_BYTES>, bool)> = None;
    let mut exhausted = false;
    if let Some(target_chapter) = seek_target_chapter {
        debug!(
            "sd: refill seek start short_name={} target_chapter={}",
            stream_state.short_name,
            target_chapter.saturating_add(1)
        );
        let mut last_seek_error: Option<SdProbeError<BUS::Error, CS::Error>> = None;
        for attempt in 1..=SD_REFILL_SEEK_ATTEMPTS {
            match probe_and_read_epub_text_chunk_at_chapter::<_, _, _, SD_TEXT_PATH_BYTES>(
                sd_spi,
                sd_cs,
                sd_delay,
                SD_BOOKS_DIR,
                stream_state.short_name.as_str(),
                target_chapter,
                &mut text_chunk,
            ) {
                Ok(text_probe) => {
                    debug!(
                        "sd: refill seek probe short_name={} attempt={}/{} status={:?} resource={} chapter={}/{} chapter_label={:?} start_offset={} bytes_read={} end={}",
                        stream_state.short_name,
                        attempt,
                        SD_REFILL_SEEK_ATTEMPTS,
                        text_probe.status,
                        text_probe.text_resource,
                        text_probe.chapter_index.saturating_add(1),
                        text_probe.chapter_total.max(1),
                        text_probe.chapter_label.as_str(),
                        text_probe.start_offset,
                        text_probe.bytes_read,
                        text_probe.end_of_resource
                    );
                    selected_probe = Some((text_probe, true));
                    last_seek_error = None;
                    break;
                }
                Err(err) => {
                    last_seek_error = Some(err);
                    if attempt < SD_REFILL_SEEK_ATTEMPTS {
                        info!(
                            "sd: refill seek retry short_name={} target_chapter={} attempt={}/{}",
                            stream_state.short_name,
                            target_chapter.saturating_add(1),
                            attempt,
                            SD_REFILL_SEEK_ATTEMPTS
                        );
                        sd_delay.delay_ms(SD_REFILL_SEEK_RETRY_DELAY_MS);
                    }
                }
            }
        }

        if selected_probe.is_none() {
            if let Some(err) = last_seek_error {
                match err {
                    SdProbeError::ChipSelect(_) => {
                        info!("sd: refill seek failed (chip-select pin)");
                    }
                    SdProbeError::Spi(_) => {
                        info!("sd: refill seek failed (spi transfer)");
                    }
                    SdProbeError::Card(card_err) => {
                        info!("sd: refill seek failed (card init): {:?}", card_err);
                    }
                    SdProbeError::Filesystem(fs_err) => {
                        info!("sd: refill seek failed (filesystem): {:?}", fs_err);
                    }
                }
            }
            let mut requeued = false;
            let _ = app.with_content_mut(|content| {
                if content.seek_chapter(target_chapter).ok() == Some(true) {
                    requeued = true;
                }
            });
            if requeued {
                info!(
                    "sd: refill seek deferred short_name={} target_chapter={} status=requeued",
                    stream_state.short_name,
                    target_chapter.saturating_add(1)
                );
                return;
            }

            let _ =
                app.with_content_mut(|content| content.mark_catalog_stream_exhausted(book_index));
            exhausted = true;
        }
    } else {
        for _ in 0..4 {
            debug!(
                "sd: refill attempt short_name={} path={} offset={} move_next={} end_of_resource={}",
                stream_state.short_name,
                current_resource,
                stream_state.next_offset,
                moving_to_next_resource,
                stream_state.end_of_resource
            );
            let refill_result = if moving_to_next_resource {
                probe_and_read_next_epub_text_chunk::<_, _, _, SD_TEXT_PATH_BYTES>(
                    sd_spi,
                    sd_cs,
                    sd_delay,
                    SD_BOOKS_DIR,
                    stream_state.short_name.as_str(),
                    current_resource.as_str(),
                    &mut text_chunk,
                )
            } else {
                probe_and_read_epub_text_chunk_from_resource::<_, _, _, SD_TEXT_PATH_BYTES>(
                    sd_spi,
                    sd_cs,
                    sd_delay,
                    SD_BOOKS_DIR,
                    stream_state.short_name.as_str(),
                    current_resource.as_str(),
                    stream_state.next_offset,
                    &mut text_chunk,
                )
            };

            match refill_result {
                Ok(text_probe) => {
                    debug!(
                        "sd: refill probe result short_name={} status={:?} resource={} chapter={}/{} chapter_label={:?} start_offset={} bytes_read={} end={}",
                        stream_state.short_name,
                        text_probe.status,
                        text_probe.text_resource,
                        text_probe.chapter_index.saturating_add(1),
                        text_probe.chapter_total.max(1),
                        text_probe.chapter_label.as_str(),
                        text_probe.start_offset,
                        text_probe.bytes_read,
                        text_probe.end_of_resource
                    );
                    if matches!(text_probe.status, SdEpubTextChunkStatus::ReadOk)
                        && text_probe.bytes_read == 0
                        && text_probe.end_of_resource
                    {
                        info!(
                            "sd: refill empty resource short_name={} resource={} moving_next=true",
                            stream_state.short_name, text_probe.text_resource
                        );
                        moving_to_next_resource = true;
                        current_resource.clear();
                        for ch in text_probe.text_resource.chars() {
                            if current_resource.push(ch).is_err() {
                                break;
                            }
                        }
                        continue;
                    }

                    selected_probe = Some((text_probe, moving_to_next_resource));
                    break;
                }
                Err(err) => {
                    match err {
                        SdProbeError::ChipSelect(_) => info!("sd: refill failed (chip-select pin)"),
                        SdProbeError::Spi(_) => info!("sd: refill failed (spi transfer)"),
                        SdProbeError::Card(card_err) => {
                            info!("sd: refill failed (card init): {:?}", card_err);
                        }
                        SdProbeError::Filesystem(fs_err) => {
                            info!("sd: refill failed (filesystem): {:?}", fs_err);
                        }
                    }
                    let _ = app.with_content_mut(|content| {
                        content.mark_catalog_stream_exhausted(book_index)
                    });
                    exhausted = true;
                    break;
                }
            }
        }
    }

    if exhausted {
        return;
    }

    let Some((text_probe, moved_flag)) = selected_probe else {
        let _ = app.with_content_mut(|content| content.mark_catalog_stream_exhausted(book_index));
        info!(
            "sd: refill stopped short_name={} status=no_next_resource_after_empty",
            stream_state.short_name
        );
        return;
    };

    if !matches!(text_probe.status, SdEpubTextChunkStatus::ReadOk) {
        stream_state.end_of_resource = true;
        let _ = app.with_content_mut(|content| content.mark_catalog_stream_exhausted(book_index));
        info!(
            "sd: refill stopped short_name={} status={:?}",
            stream_state.short_name, text_probe.status
        );
        return;
    }

    let apply_chunk = &text_chunk[..text_probe.bytes_read.min(text_chunk.len())];
    let mut previous_resource = HeaplessString::<SD_TEXT_PATH_BYTES>::new();
    for ch in stream_state.text_resource.chars() {
        if previous_resource.push(ch).is_err() {
            break;
        }
    }
    match app.with_content_mut(|content| {
        let applied = content.set_catalog_text_chunk_from_bytes(
            book_index,
            apply_chunk,
            text_probe.end_of_resource,
            text_probe.text_resource.as_str(),
        )?;
        let _ = content.set_catalog_stream_chapter_metadata(
            book_index,
            text_probe.chapter_index,
            text_probe.chapter_total,
            Some(text_probe.chapter_label.as_str()),
        );
        Ok::<_, SdCatalogError>(applied)
    }) {
        Ok(applied) => {
            if moved_flag {
                stream_state.next_offset = text_probe
                    .start_offset
                    .saturating_add(text_probe.bytes_read as u32);
            } else {
                stream_state.next_offset = stream_state
                    .next_offset
                    .saturating_add(text_probe.bytes_read as u32);
            }
            stream_state.end_of_resource = text_probe.end_of_resource;
            stream_state.text_resource.clear();
            for ch in text_probe.text_resource.chars() {
                if stream_state.text_resource.push(ch).is_err() {
                    break;
                }
            }
            stream_state.ready = !stream_state.text_resource.is_empty();
            debug!(
                "sd: refill apply short_name={} resource={} chapter={}/{} chapter_label={:?} start_offset={} bytes_read={} end={} applied_loaded={} applied_truncated={} next_offset={} next_ready={}",
                stream_state.short_name,
                stream_state.text_resource,
                text_probe.chapter_index.saturating_add(1),
                text_probe.chapter_total.max(1),
                text_probe.chapter_label.as_str(),
                text_probe.start_offset,
                text_probe.bytes_read,
                text_probe.end_of_resource,
                applied.loaded,
                applied.truncated,
                stream_state.next_offset,
                stream_state.ready
            );

            if moved_flag {
                debug!(
                    "sd: refill advanced resource short_name={} from={} to={} bytes_read={} end={}",
                    stream_state.short_name,
                    previous_resource,
                    stream_state.text_resource,
                    text_probe.bytes_read,
                    text_probe.end_of_resource
                );
            }

            if applied.truncated {
                debug!(
                    "sd: refill truncated short_name={} offset={} bytes_read={}",
                    stream_state.short_name, stream_state.next_offset, text_probe.bytes_read
                );
            }
        }
        Err(_) => {
            info!(
                "sd: refill apply failed (invalid catalog index={}) short_name={} resource={} chapter={}/{} bytes_read={}",
                book_index,
                stream_state.short_name,
                text_probe.text_resource,
                text_probe.chapter_index.saturating_add(1),
                text_probe.chapter_total.max(1),
                text_probe.bytes_read
            );
        }
    }
}
