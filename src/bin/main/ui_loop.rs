use core::fmt::Debug;

use embassy_time::Timer;
use embedded_hal::{delay::DelayNs, digital::OutputPin, spi::SpiBus};
use esp_hal::time::{Duration as HalDuration, Instant};
use heapless::Vec as HeaplessVec;
use log::info;
use ls027b7dh01::FrameBuffer;
use readily_core::{
    app::{ReaderApp, ReaderConfig, TickResult},
    content::sd_catalog::SdCatalogSource,
    input::InputProvider,
    render::Screen,
    settings::SettingsStore,
};
use readily_hal_esp32s3::{
    network::ConnectivityHandle,
    platform::display::SharpDisplay,
    render::{FrameRenderer, rsvp::RsvpRenderer},
    storage::flash_settings::FlashSettingsStore,
};

use super::{
    DISPLAY_SPI_HZ, SD_BOOKS_DIR, SD_COVER_THUMB_BYTES, SD_COVER_THUMB_HEIGHT,
    SD_COVER_THUMB_WIDTH, SD_SCAN_MAX_EPUBS, SD_SPI_HZ_CANDIDATES, SD_TEXT_CHUNK_BYTES,
    SD_TEXT_PREVIEW_BYTES, SdBookStreamState, TITLE, book_db, network_runtime, power,
    resume_sync::{ResumeFlushReason, ResumeSyncState},
    sd_refill,
    settings_sync::SettingsSyncState,
};

const SLEEP_INACTIVITY_TIMEOUT_MS: u64 = 60_000;
const SLEEP_NOTICE_MS: u64 = 120;

#[allow(
    clippy::large_stack_frames,
    reason = "The UI task owns the framebuffer and runtime state for the duration of the device loop."
)]
pub(super) async fn run<IN, DSPI, DISP, EMD, DCS, SDBUS, SDCS, DLY, F>(
    app: &mut ReaderApp<SdCatalogSource, IN>,
    renderer: &mut RsvpRenderer,
    display: &mut SharpDisplay<DSPI, DISP, EMD, DCS>,
    mut frame: &mut FrameBuffer,
    sd_stream_states: &mut HeaplessVec<SdBookStreamState, SD_SCAN_MAX_EPUBS>,
    display_delay: &mut DLY,
    sd_spi: &mut SDBUS,
    sd_cs: &mut SDCS,
    sd_delay: &mut DLY,
    sd_spi_speed_index: usize,
    try_set_sd_speed: &mut F,
    settings_store: &mut Option<FlashSettingsStore>,
    connectivity: &'static ConnectivityHandle,
    reader_config: ReaderConfig,
    display_fault_logged: bool,
) -> !
where
    IN: InputProvider,
    DSPI: SpiBus<u8>,
    DISP: OutputPin,
    EMD: OutputPin,
    DCS: OutputPin,
    SDBUS: SpiBus<u8>,
    SDCS: OutputPin,
    DLY: DelayNs,
    DSPI::Error: Debug,
    DISP::Error: Debug,
    EMD::Error: Debug,
    DCS::Error: Debug,
    SDBUS::Error: Debug,
    SDCS::Error: Debug,
    F: FnMut(&mut SDBUS, usize) -> bool,
{
    let mut settings_sync = SettingsSyncState::new(app.persisted_settings());
    let mut resume_sync = ResumeSyncState::new(app.export_resume_state(), app.sleep_eligible());
    let mut last_connectivity_revision = u32::MAX;
    let mut display_fault_logged = display_fault_logged;
    let mut display_first_flush_logged = false;

    let loop_start = Instant::now();
    let mut report_words = 0u64;
    let mut report_start = Instant::now();

    log_runtime_startup(reader_config, sd_spi_speed_index);

    loop {
        sd_refill::handle_pending_refill(
            app,
            sd_stream_states,
            sd_spi,
            sd_cs,
            sd_delay,
            sd_spi_speed_index,
            |spi, speed_index| try_set_sd_speed(spi, speed_index),
        );

        let now_ms = loop_start.elapsed().as_millis();
        let connectivity = connectivity.snapshot();
        let app_requests_render = app.tick(now_ms) == TickResult::RenderRequested;
        let current_resume = app.export_resume_state();
        if let Some((resume, reason)) =
            resume_sync.observe(current_resume, app.sleep_eligible(), now_ms)
        {
            if book_db::save_resume_to_db(resume, sd_stream_states, sd_spi, sd_cs, sd_delay) {
                resume_sync.mark_saved(resume, now_ms);
                info!(
                    "resume-save: flushed reason={} book={} chapter={} paragraph={} word={}",
                    reason.as_str(),
                    resume.selected_book.saturating_add(1),
                    resume.chapter_index.saturating_add(1),
                    resume.paragraph_in_chapter.saturating_add(1),
                    resume.word_index.max(1)
                );
            }
        }
        if let Some(resume) = resume_sync.debounced_due(now_ms)
            && resume_sync.can_flush_now(now_ms)
            && book_db::save_resume_to_db(resume, sd_stream_states, sd_spi, sd_cs, sd_delay)
        {
            resume_sync.mark_saved(resume, now_ms);
            info!(
                "resume-save: flushed reason={} book={} chapter={} paragraph={} word={}",
                ResumeFlushReason::Debounce.as_str(),
                resume.selected_book.saturating_add(1),
                resume.chapter_index.saturating_add(1),
                resume.paragraph_in_chapter.saturating_add(1),
                resume.word_index.max(1)
            );
        }
        let connectivity_changed = connectivity.revision != last_connectivity_revision;

        if app_requests_render || connectivity_changed {
            renderer.set_connectivity(connectivity);
            app.with_screen(now_ms, |screen| renderer.render(screen, &mut frame));
            if let Err(err) = display.flush_frame(frame, display_delay) {
                if !display_fault_logged {
                    esp_println::println!("display: flush failed");
                    info!("display flush failed: {:?}", err);
                    display_fault_logged = true;
                }
            } else if !display_first_flush_logged {
                esp_println::println!("display: first flush ok");
                display_first_flush_logged = true;
            }
            last_connectivity_revision = connectivity.revision;
        }

        settings_sync.track_current(app.persisted_settings(), now_ms);
        settings_sync.flush_if_due(settings_store.as_mut(), now_ms);

        if app.inactivity_sleep_due(now_ms, SLEEP_INACTIVITY_TIMEOUT_MS) {
            if let Some(resume) = app.export_resume_state() {
                if book_db::save_resume_to_db(resume, sd_stream_states, sd_spi, sd_cs, sd_delay) {
                    resume_sync.mark_saved(resume, now_ms);
                    info!(
                        "resume-save: flushed reason={} book={} chapter={} paragraph={} word={}",
                        ResumeFlushReason::Sleep.as_str(),
                        resume.selected_book.saturating_add(1),
                        resume.chapter_index.saturating_add(1),
                        resume.paragraph_in_chapter.saturating_add(1),
                        resume.word_index.max(1)
                    );
                }
            }
            let snapshot = app
                .persisted_settings()
                .with_wake_snapshot(app.export_wake_snapshot());
            if let Some(store) = settings_store.as_mut() {
                if store.save(&snapshot).is_ok() {
                    info!("sleep: persisted settings and wake snapshot");
                } else {
                    info!("sleep: failed to persist wake snapshot before deep sleep");
                }
            }
            info!(
                "sleep: entering deep sleep after {}ms inactivity",
                SLEEP_INACTIVITY_TIMEOUT_MS
            );
            renderer.render(
                Screen::Status {
                    title: TITLE,
                    wpm: snapshot.wpm,
                    line1: "SLEEPING...",
                    line2: "PRESS TO WAKE",
                    style: snapshot.style,
                    animation: None,
                },
                frame,
            );
            let _ = display.flush_frame(frame, display_delay);
            Timer::after_millis(SLEEP_NOTICE_MS).await;
            power::enter_deep_sleep(display, sd_spi, sd_cs);
        }

        report_words = report_words.saturating_add(app.drain_word_updates() as u64);

        let elapsed = report_start.elapsed();
        if elapsed >= HalDuration::from_secs(5) {
            let elapsed_ms = elapsed.as_millis().max(1);
            let wpm_x100 = report_words * 6_000_000 / elapsed_ms;

            info!(
                "effective_wpm={}.{:02} words={} elapsed_ms={}",
                wpm_x100 / 100,
                wpm_x100 % 100,
                report_words,
                elapsed_ms
            );

            report_words = 0;
            report_start = Instant::now();
        }

        Timer::after_millis(1).await;
    }
}

#[allow(
    clippy::large_stack_frames,
    reason = "Startup logging stays in one helper to keep the runtime entrypoint flat."
)]
fn log_runtime_startup(reader_config: ReaderConfig, sd_spi_speed_index: usize) {
    info!(
        "Reader started: target_wpm={} dot_pause_ms={} comma_pause_ms={} spi_hz={}",
        reader_config.wpm, reader_config.dot_pause_ms, reader_config.comma_pause_ms, DISPLAY_SPI_HZ
    );
    info!("Display pins: CLK=GPIO13 DI=GPIO14 CS=GPIO15 DISP=GPIO2 EMD=GPIO9");
    info!("Encoder pins: CLK=GPIO10 DT=GPIO11 SW=GPIO12");
    info!("SD pins: CS=GPIO8 SCK=GPIO4 MOSI=GPIO40 MISO=GPIO41");
    info!(
        "SD stream refill enabled: books_dir={} start_spi_hz={} chunk_bytes={}",
        SD_BOOKS_DIR, SD_SPI_HZ_CANDIDATES[sd_spi_speed_index], SD_TEXT_CHUNK_BYTES
    );
    info!(
        "SD text-chunk probe enabled: chunk_bytes={} preview_bytes={}",
        SD_TEXT_CHUNK_BYTES, SD_TEXT_PREVIEW_BYTES
    );
    info!(
        "SD cover probe enabled: thumb={}x{} bytes={}",
        SD_COVER_THUMB_WIDTH, SD_COVER_THUMB_HEIGHT, SD_COVER_THUMB_BYTES
    );
    info!("Content source uses initial SD scan for titles and first text chunks");
    info!(
        "Wi-Fi bootstrap configured from env; ping_target={}",
        network_runtime::PING_TARGET
    );
}
