#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use core::net::Ipv4Addr;

use embassy_executor::Spawner;
use embassy_net::{
    Stack,
    icmp::{PacketMetadata, ping::PingManager, ping::PingParams},
};
use embassy_time::{Duration as EmbassyDuration, Timer, WithTimeout};
use esp_hal::{
    clock::CpuClock,
    delay::Delay,
    gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull, RtcPin},
    rtc_cntl::{SocResetReason, reset_reason, wakeup_cause},
    spi::master::Spi,
    system::Cpu,
    time::{Duration as HalDuration, Instant, Rate},
    timer::timg::TimerGroup,
};
use esp_radio::wifi::{ClientConfig, ModeConfig, WifiController};
use heapless::{String as HeaplessString, Vec as HeaplessVec};
use log::{LevelFilter, info};
use ls027b7dh01::FrameBuffer;
use readily_core::{
    app::{ReaderApp, ReaderConfig, TickResult},
    content::sd_catalog::{SD_CATALOG_MAX_TITLES, SD_CATALOG_TITLE_BYTES, SdCatalogSource},
    render::Screen,
    settings::{ResumeState, SettingsStore},
};
use readily_hal_esp32s3::{
    input::rotary::{RotaryConfig, RotaryInput},
    network::{ConnectivityHandle, WifiConfig},
    platform::display::SharpDisplay,
    render::{FrameRenderer, rsvp::RsvpRenderer},
    storage::flash_settings::FlashSettingsStore,
};
use static_cell::StaticCell;

use loading::{LoadingCoordinator, LoadingEvent, LoadingMode};
use settings_sync::SettingsSyncState;

#[path = "main/book_db.rs"]
mod book_db;
#[path = "main/initial_catalog.rs"]
mod initial_catalog;
#[path = "main/loading.rs"]
mod loading;
#[path = "main/power.rs"]
mod power;
#[path = "main/sd_refill.rs"]
mod sd_refill;
#[path = "main/settings_sync.rs"]
mod settings_sync;

const DISPLAY_SPI_HZ: u32 = 1_000_000;
const SD_SPI_HZ_CANDIDATES: [u32; 4] = [1_000_000, 600_000, 300_000, 100_000];
const SD_PROBE_ATTEMPTS: u8 = 3;
const SD_PROBE_RETRY_DELAY_MS: u64 = 120;
const SD_BOOKS_DIR: &str = "BOOKS";
const SD_SCAN_MAX_EPUBS: usize = SD_CATALOG_MAX_TITLES;
const SD_SCAN_NAME_BYTES: usize = SD_CATALOG_TITLE_BYTES;
const SD_SCAN_MAX_CANDIDATES: usize = 48;
const SD_TEXT_CHUNK_BYTES: usize = 480;
const SD_TEXT_PATH_BYTES: usize = 192;
const SD_COVER_MEDIA_BYTES: usize = 32;
const SD_COVER_THUMB_WIDTH: u16 = 56;
const SD_COVER_THUMB_HEIGHT: u16 = 76;
const SD_COVER_THUMB_BYTES: usize =
    ((SD_COVER_THUMB_WIDTH as usize + 7) / 8) * SD_COVER_THUMB_HEIGHT as usize;
const SD_TEXT_PREVIEW_BYTES: usize = 96;
const TITLE: &str = "Readily";
const ORP_ANCHOR_PERCENT: usize = 42;
const COUNTDOWN_SECONDS: u8 = 3;
const ENCODER_DIRECTION_INVERTED: bool = false;
const SETTINGS_SAVE_DEBOUNCE_MS: u64 = 1_500;
const RESUME_SAVE_DEBOUNCE_MS: u64 = 4_000;
const RESUME_SAVE_MIN_SPACING_MS: u64 = 500;
const WIFI_RETRY_BACKOFF_MIN_SECS: u64 = 2;
const WIFI_RETRY_BACKOFF_MAX_SECS: u64 = 120;
const NETWORK_POLL_INTERVAL_MS: u64 = 500;
const PING_INTERVAL_SECS: u64 = 5;
const PING_IDLE_INTERVAL_SECS: u64 = 20;
const PING_TIMEOUT_MS: u64 = 1_200;
const DHCP_TIMEOUT_SECS: u64 = 15;
const SLEEP_INACTIVITY_TIMEOUT_MS: u64 = 60_000;
const SLEEP_NOTICE_MS: u64 = 120;

const WIFI_SSID: &str = env!(
    "READILY_WIFI_SSID",
    "Set READILY_WIFI_SSID in your environment before building/flashing."
);
const WIFI_PASSWORD: &str = env!(
    "READILY_WIFI_PASSWORD",
    "Set READILY_WIFI_PASSWORD in your environment before building/flashing."
);
const WIFI_CONFIG: WifiConfig = WifiConfig::new(WIFI_SSID, WIFI_PASSWORD);
const PING_TARGET: Ipv4Addr = Ipv4Addr::new(1, 1, 1, 1);

static CONNECTIVITY: ConnectivityHandle = ConnectivityHandle::new();
static NET_RESOURCES: StaticCell<embassy_net::StackResources<4>> = StaticCell::new();

#[derive(Debug, Clone)]
struct SdBookStreamState {
    short_name: HeaplessString<SD_SCAN_NAME_BYTES>,
    text_resource: HeaplessString<SD_TEXT_PATH_BYTES>,
    next_offset: u32,
    end_of_resource: bool,
    ready: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResumeFlushReason {
    Debounce,
    PauseOrNavigation,
    Sleep,
}

impl ResumeFlushReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::Debounce => "debounce",
            Self::PauseOrNavigation => "pause_or_navigation",
            Self::Sleep => "sleep",
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ResumeSyncState {
    last_saved: Option<ResumeState>,
    last_seen: Option<ResumeState>,
    dirty_since_ms: Option<u64>,
    last_flush_ms: Option<u64>,
    prev_sleep_eligible: bool,
}

impl ResumeSyncState {
    fn new(initial_resume: Option<ResumeState>, initial_sleep_eligible: bool) -> Self {
        Self {
            last_saved: initial_resume,
            last_seen: initial_resume,
            dirty_since_ms: None,
            last_flush_ms: None,
            prev_sleep_eligible: initial_sleep_eligible,
        }
    }

    fn observe(
        &mut self,
        current: Option<ResumeState>,
        sleep_eligible_now: bool,
        now_ms: u64,
    ) -> Option<(ResumeState, ResumeFlushReason)> {
        let mut force = None;

        if !self.prev_sleep_eligible
            && sleep_eligible_now
            && let Some(resume) = current
        {
            force = Some((resume, ResumeFlushReason::PauseOrNavigation));
        }
        self.prev_sleep_eligible = sleep_eligible_now;

        match (self.last_seen, current) {
            (Some(previous), Some(now)) => {
                if sleep_eligible_now
                    && previous.selected_book == now.selected_book
                    && !same_resume_paragraph(previous, now)
                    && force.is_none()
                {
                    force = Some((now, ResumeFlushReason::PauseOrNavigation));
                }
                if !sleep_eligible_now && !same_resume_paragraph(previous, now) {
                    self.dirty_since_ms = Some(now_ms);
                }
                self.last_seen = Some(now);
            }
            (None, Some(now)) => {
                if !sleep_eligible_now
                    && self
                        .last_saved
                        .is_none_or(|saved| !same_resume_paragraph(saved, now))
                {
                    self.dirty_since_ms = Some(now_ms);
                }
                self.last_seen = Some(now);
            }
            (_, None) => {
                self.last_seen = None;
                self.dirty_since_ms = None;
            }
        }

        force
    }

    fn debounced_due(&self, now_ms: u64) -> Option<ResumeState> {
        let dirty_since = self.dirty_since_ms?;
        let resume = self.last_seen?;
        if now_ms.saturating_sub(dirty_since) < RESUME_SAVE_DEBOUNCE_MS {
            return None;
        }
        if self
            .last_saved
            .is_some_and(|saved| same_resume_paragraph(saved, resume))
        {
            return None;
        }
        Some(resume)
    }

    fn can_flush_now(&self, now_ms: u64) -> bool {
        self.last_flush_ms
            .is_none_or(|last| now_ms.saturating_sub(last) >= RESUME_SAVE_MIN_SPACING_MS)
    }

    fn mark_saved(&mut self, saved: ResumeState, now_ms: u64) {
        self.last_saved = Some(saved);
        self.last_flush_ms = Some(now_ms);
        if self
            .last_seen
            .is_some_and(|seen| same_resume_paragraph(seen, saved))
        {
            self.dirty_since_ms = None;
        }
    }
}

fn same_resume_paragraph(a: ResumeState, b: ResumeState) -> bool {
    a.selected_book == b.selected_book
        && a.chapter_index == b.chapter_index
        && a.paragraph_in_chapter == b.paragraph_in_chapter
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

fn wifi_retry_backoff_secs(consecutive_failures: u32) -> u64 {
    // 2, 4, 8, 16, 32, 64, 120, 120, ...
    let shift = consecutive_failures.min(6);
    WIFI_RETRY_BACKOFF_MIN_SECS
        .saturating_mul(1u64 << shift)
        .min(WIFI_RETRY_BACKOFF_MAX_SECS)
}

async fn wait_before_wifi_retry(consecutive_failures: &mut u32) {
    let delay_secs = wifi_retry_backoff_secs(*consecutive_failures);
    *consecutive_failures = consecutive_failures.saturating_add(1);
    info!(
        "wifi retrying in {}s (consecutive_failures={})",
        delay_secs, *consecutive_failures
    );
    Timer::after_secs(delay_secs).await;
}

async fn wifi_connection_loop(
    wifi_controller: &mut WifiController<'_>,
    stack: Stack<'_>,
    connectivity: &'static ConnectivityHandle,
) -> ! {
    let mut consecutive_failures = 0u32;

    loop {
        connectivity.mark_connecting();

        if !wifi_controller.is_started().unwrap_or(false) {
            if let Err(err) = wifi_controller.start_async().await {
                info!("wifi start failed: {:?}", err);
                connectivity.mark_disconnected();
                wait_before_wifi_retry(&mut consecutive_failures).await;
                continue;
            }
        }

        if let Err(err) = wifi_controller.connect_async().await {
            info!("wifi connect failed: {:?}", err);
            connectivity.mark_disconnected();
            let _ = wifi_controller.disconnect_async().await;
            wait_before_wifi_retry(&mut consecutive_failures).await;
            continue;
        }

        match stack
            .wait_config_up()
            .with_timeout(EmbassyDuration::from_secs(DHCP_TIMEOUT_SECS))
            .await
        {
            Ok(()) => {
                connectivity.update_link_ip(stack.is_link_up(), stack.config_v4().is_some());
                info!("wifi connected and dhcp ready");
            }
            Err(_) => {
                info!("dhcp timeout; forcing reconnect");
                connectivity.update_link_ip(stack.is_link_up(), false);
                let _ = wifi_controller.disconnect_async().await;
                wait_before_wifi_retry(&mut consecutive_failures).await;
                continue;
            }
        }

        consecutive_failures = 0;

        loop {
            let link_up = stack.is_link_up();
            let has_ipv4 = stack.config_v4().is_some();
            let is_connected = matches!(wifi_controller.is_connected(), Ok(true));

            connectivity.update_link_ip(link_up, has_ipv4);

            if !(link_up && has_ipv4 && is_connected) {
                info!(
                    "wifi state lost (link_up={} has_ipv4={} connected={}); reconnecting",
                    link_up, has_ipv4, is_connected
                );
                break;
            }

            Timer::after_millis(NETWORK_POLL_INTERVAL_MS).await;
        }

        connectivity.mark_disconnected();
        let _ = wifi_controller.disconnect_async().await;
        wait_before_wifi_retry(&mut consecutive_failures).await;
    }
}

async fn ping_loop(stack: Stack<'_>, connectivity: &'static ConnectivityHandle) -> ! {
    let mut rx_buffer = [0u8; 256];
    let mut tx_buffer = [0u8; 256];
    let mut rx_meta = [PacketMetadata::EMPTY; 1];
    let mut tx_meta = [PacketMetadata::EMPTY; 1];

    let mut ping_manager = PingManager::new(
        stack,
        &mut rx_meta,
        &mut rx_buffer,
        &mut tx_meta,
        &mut tx_buffer,
    );
    let mut ping_params = PingParams::new(PING_TARGET);
    ping_params
        .set_payload(b"readily")
        .set_count(1)
        .set_timeout(EmbassyDuration::from_millis(PING_TIMEOUT_MS))
        .set_rate_limit(EmbassyDuration::from_secs(1));

    loop {
        let link_up = stack.is_link_up();
        let has_ipv4 = stack.config_v4().is_some();
        connectivity.update_link_ip(link_up, has_ipv4);

        if link_up && has_ipv4 {
            match ping_manager.ping(&ping_params).await {
                Ok(_) => connectivity.update_ping(true),
                Err(err) => {
                    info!("ping {} failed: {:?}", PING_TARGET, err);
                    connectivity.update_ping(false);
                }
            }
        } else {
            connectivity.update_ping(false);
        }

        let interval_secs = if link_up && has_ipv4 {
            PING_INTERVAL_SECS
        } else {
            PING_IDLE_INTERVAL_SECS
        };
        Timer::after_secs(interval_secs).await;
    }
}

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    esp_println::logger::init_logger(LevelFilter::Info);
    esp_println::println!("boot: readily starting");

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);
    let boot_reset_reason = reset_reason(Cpu::ProCpu);
    let boot_wakeup_cause = wakeup_cause();
    let woke_from_deep_sleep = boot_reset_reason == Some(SocResetReason::CoreDeepSleep);
    info!(
        "boot reset_reason={:?} wakeup_cause={:?}",
        boot_reset_reason, boot_wakeup_cause
    );

    // esp-radio requires an allocator.
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 65536);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    // Wiring used by this demo:
    // CLK=GPIO13, DI=GPIO14, CS=GPIO15, DISP=GPIO2, EMD=GPIO9
    let disp_pin = peripherals.GPIO2;
    // Release any prior deep-sleep pad hold before driving DISP high again.
    disp_pin.rtcio_pad_hold(false);
    let disp = Output::new(disp_pin, Level::Low, OutputConfig::default());
    let emd = Output::new(peripherals.GPIO9, Level::Low, OutputConfig::default());
    let cs = Output::new(peripherals.GPIO15, Level::Low, OutputConfig::default());

    let spi_config = esp_hal::spi::master::Config::default()
        .with_frequency(Rate::from_hz(DISPLAY_SPI_HZ))
        // LS027B7DH01 uses CPOL=0, CPHA=1.
        .with_mode(esp_hal::spi::Mode::_1);

    let spi = Spi::new(peripherals.SPI2, spi_config)
        .unwrap()
        .with_sck(peripherals.GPIO13)
        .with_mosi(peripherals.GPIO14);

    let mut delay = Delay::new();

    let mut display = SharpDisplay::new(spi, disp, emd, cs);
    let mut display_fault_logged = false;
    esp_println::println!("display: init begin (CLK=13 DI=14 CS=15 DISP=2 EMD=9)");
    if let Err(err) = display.initialize(&mut delay) {
        esp_println::println!("display: initialize failed");
        info!("display initialize failed: {:?}", err);
        display_fault_logged = true;
    } else {
        esp_println::println!("display: initialize ok");
    }
    if let Err(err) = display.clear_all(&mut delay) {
        esp_println::println!("display: clear failed");
        info!("display clear failed: {:?}", err);
        display_fault_logged = true;
    } else {
        esp_println::println!("display: clear ok");
    }

    // Early bring-up proof: push a full-black frame before app setup.
    let mut boot_test_frame = FrameBuffer::new();
    boot_test_frame.clear(true);
    if let Err(err) = display.flush_frame(&boot_test_frame, &mut delay) {
        esp_println::println!("display: boot test flush failed");
        info!("display boot test flush failed: {:?}", err);
        display_fault_logged = true;
    } else {
        esp_println::println!("display: boot test frame flushed (full black)");
    }

    // Rotary encoder wiring used by this demo:
    // CLK=GPIO10, DT=GPIO11, SW=GPIO12
    let input_cfg = InputConfig::default().with_pull(Pull::Up);
    let encoder_clk = Input::new(peripherals.GPIO10, input_cfg);
    let encoder_dt = Input::new(peripherals.GPIO11, input_cfg);
    let encoder_sw = Input::new(peripherals.GPIO12, input_cfg);

    let input = RotaryInput::new(
        encoder_clk,
        encoder_dt,
        encoder_sw,
        RotaryConfig::default()
            .with_direction_inverted(ENCODER_DIRECTION_INVERTED)
            .with_button_debounce_polls(4),
    )
    .unwrap();

    // SD SPI wiring (phase-1 bring-up):
    // CS=GPIO8, SCK=GPIO4, MOSI=GPIO40, MISO=GPIO41
    let mut sd_cs = Output::new(peripherals.GPIO8, Level::High, OutputConfig::default());
    let sd_spi_config = esp_hal::spi::master::Config::default()
        .with_frequency(Rate::from_hz(SD_SPI_HZ_CANDIDATES[0]))
        // SD cards in SPI mode use CPOL=0, CPHA=0.
        .with_mode(esp_hal::spi::Mode::_0);
    let mut sd_spi = Spi::new(peripherals.SPI3, sd_spi_config)
        .unwrap()
        .with_sck(peripherals.GPIO4)
        .with_mosi(peripherals.GPIO40)
        .with_miso(peripherals.GPIO41);
    let mut sd_delay = Delay::new();
    let mut sd_spi_speed_index = 0usize;

    let mut renderer = RsvpRenderer::new(ORP_ANCHOR_PERCENT);
    let loading_mode = if woke_from_deep_sleep {
        LoadingMode::WakeFromDeepSleep
    } else {
        LoadingMode::ColdBoot
    };
    let mut loading = LoadingCoordinator::new(loading_mode);
    let loading_start = Instant::now();
    let mut frame = FrameBuffer::new();

    let mut content = SdCatalogSource::new();
    let mut sd_stream_states: HeaplessVec<SdBookStreamState, SD_SCAN_MAX_EPUBS> =
        HeaplessVec::new();
    let mut catalog_loaded_from_db = false;
    match book_db::try_load_catalog_from_db(
        &mut content,
        &mut sd_stream_states,
        &mut sd_spi,
        &mut sd_cs,
        &mut sd_delay,
    ) {
        Ok(loaded) => {
            catalog_loaded_from_db = loaded;
        }
        Err(err) => {
            info!("sd-db: manifest load failed: {:?}", err);
        }
    }

    if catalog_loaded_from_db {
        let now_ms = loading_start.elapsed().as_millis();
        if loading.on_event(now_ms, LoadingEvent::Begin) {
            renderer.render_loading(loading.view(now_ms), &mut frame);
            if let Err(err) = display.flush_frame(&frame, &mut delay)
                && !display_fault_logged
            {
                esp_println::println!("display: loading flush failed");
                info!("display loading flush failed: {:?}", err);
                display_fault_logged = true;
            }
        }
        let books_total = sd_stream_states.len().clamp(0, u16::MAX as usize) as u16;
        let now_ms = loading_start.elapsed().as_millis();
        if loading.on_event(
            now_ms,
            LoadingEvent::ScanResult {
                books_dir_found: true,
                books_total,
            },
        ) {
            renderer.render_loading(loading.view(now_ms), &mut frame);
            if let Err(err) = display.flush_frame(&frame, &mut delay)
                && !display_fault_logged
            {
                esp_println::println!("display: loading flush failed");
                info!("display loading flush failed: {:?}", err);
                display_fault_logged = true;
            }
        }
        let now_ms = loading_start.elapsed().as_millis();
        if loading.on_event(now_ms, LoadingEvent::Finished) {
            renderer.render_loading(loading.view(now_ms), &mut frame);
            if let Err(err) = display.flush_frame(&frame, &mut delay)
                && !display_fault_logged
            {
                esp_println::println!("display: loading flush failed");
                info!("display loading flush failed: {:?}", err);
                display_fault_logged = true;
            }
        }
        info!(
            "sd-db: boot catalog source=manifest books={} spi_hz={}",
            books_total, SD_SPI_HZ_CANDIDATES[sd_spi_speed_index]
        );
    } else {
        sd_spi_speed_index = initial_catalog::preload_initial_catalog(
            &mut content,
            &mut renderer,
            &mut sd_stream_states,
            &mut sd_spi,
            &mut sd_cs,
            &mut sd_delay,
            sd_spi_speed_index,
            |spi, speed_index| {
                let speed_hz = SD_SPI_HZ_CANDIDATES[speed_index];
                let speed_config = esp_hal::spi::master::Config::default()
                    .with_frequency(Rate::from_hz(speed_hz))
                    .with_mode(esp_hal::spi::Mode::_0);
                spi.apply_config(&speed_config).is_ok()
            },
            |event, renderer| {
                let now_ms = loading_start.elapsed().as_millis();
                if loading.on_event(now_ms, event) {
                    renderer.render_loading(loading.view(now_ms), &mut frame);
                    if let Err(err) = display.flush_frame(&frame, &mut delay)
                        && !display_fault_logged
                    {
                        esp_println::println!("display: loading flush failed");
                        info!("display loading flush failed: {:?}", err);
                        display_fault_logged = true;
                    }
                }
            },
        )
        .await;
        book_db::build_book_db_from_runtime(
            &content,
            &sd_stream_states,
            &mut sd_spi,
            &mut sd_cs,
            &mut sd_delay,
        );
    }

    let reader_config = ReaderConfig {
        wpm: 230,
        min_wpm: 80,
        max_wpm: 600,
        dot_pause_ms: 240,
        comma_pause_ms: 240,
    };

    let mut app = ReaderApp::new(content, input, reader_config, TITLE, COUNTDOWN_SECONDS);

    let mut settings_store = match FlashSettingsStore::new() {
        Ok(store) => Some(store),
        Err(_) => {
            info!("settings storage unavailable; defaults will be volatile");
            None
        }
    };
    let mut restore_wake_snapshot = None;

    if let Some(store) = settings_store.as_mut() {
        match store.load() {
            Ok(Some(saved)) => {
                restore_wake_snapshot = saved.wake_snapshot;
                app.apply_persisted_settings(saved);
                info!("settings restored from flash");
            }
            Ok(None) => {
                info!("no saved settings in flash");
            }
            Err(_) => {
                info!("failed to read saved settings; using defaults");
            }
        }
    }

    let db_resume =
        book_db::load_resume_from_db(&sd_stream_states, &mut sd_spi, &mut sd_cs, &mut sd_delay);

    if woke_from_deep_sleep {
        let mut restored = false;

        if let Some(mut snapshot) = restore_wake_snapshot {
            if let Some(resume) = db_resume {
                snapshot.resume = resume;
                info!(
                    "wake resume merged from sd-db selected_book={} chapter={} paragraph={} word={}",
                    resume.selected_book.saturating_add(1),
                    resume.chapter_index.saturating_add(1),
                    resume.paragraph_in_chapter.saturating_add(1),
                    resume.word_index.max(1)
                );
            }
            if app.import_wake_snapshot(snapshot, 0) {
                restored = true;
                info!(
                    "wake snapshot restored context={:?} selected_book={} chapter={} paragraph={} word={}",
                    snapshot.ui_context,
                    snapshot.resume.selected_book.saturating_add(1),
                    snapshot.resume.chapter_index.saturating_add(1),
                    snapshot.resume.paragraph_in_chapter.saturating_add(1),
                    snapshot.resume.word_index.max(1)
                );
            } else {
                info!("wake snapshot restore failed; using default app state");
            }
        } else if let Some(resume) = db_resume {
            if app.import_resume_state(resume, 0) {
                restored = true;
                info!(
                    "wake resume restored from sd-db selected_book={} chapter={} paragraph={} word={}",
                    resume.selected_book.saturating_add(1),
                    resume.chapter_index.saturating_add(1),
                    resume.paragraph_in_chapter.saturating_add(1),
                    resume.word_index.max(1)
                );
            } else {
                info!("wake resume from sd-db failed; using default app state");
            }
        }

        if !restored {
            info!("woke from deep sleep without restorable snapshot/progress");
        }
    } else if let Some(resume) = db_resume {
        if app.import_resume_state(resume, 0) {
            info!(
                "boot resume restored from sd-db selected_book={} chapter={} paragraph={} word={}",
                resume.selected_book.saturating_add(1),
                resume.chapter_index.saturating_add(1),
                resume.paragraph_in_chapter.saturating_add(1),
                resume.word_index.max(1)
            );
        } else {
            info!("boot resume from sd-db failed; using default app state");
        }
    }

    let radio = match esp_radio::init() {
        Ok(radio) => radio,
        Err(err) => {
            info!("esp-radio init failed: {:?}", err);
            loop {
                Timer::after_secs(1).await;
            }
        }
    };

    let (mut wifi_controller, interfaces) =
        match esp_radio::wifi::new(&radio, peripherals.WIFI, esp_radio::wifi::Config::default()) {
            Ok(parts) => parts,
            Err(err) => {
                info!("wifi peripheral init failed: {:?}", err);
                loop {
                    Timer::after_secs(1).await;
                }
            }
        };

    let client_config = ClientConfig::default()
        .with_ssid(WIFI_CONFIG.ssid.into())
        .with_password(WIFI_CONFIG.password.into());
    let wifi_mode = ModeConfig::Client(client_config);
    if let Err(err) = wifi_controller.set_config(&wifi_mode) {
        info!("wifi mode config failed: {:?}", err);
        loop {
            Timer::after_secs(1).await;
        }
    }

    let stack_config = embassy_net::Config::dhcpv4(Default::default());
    let (stack, mut net_runner) = embassy_net::new(
        interfaces.sta,
        stack_config,
        NET_RESOURCES.init(embassy_net::StackResources::<4>::new()),
        0x5A17_2B34_D099_EE11,
    );

    let mut settings_sync = SettingsSyncState::new(app.persisted_settings());
    let mut resume_sync = ResumeSyncState::new(app.export_resume_state(), app.sleep_eligible());
    let mut last_connectivity_revision = u32::MAX;
    let mut display_first_flush_logged = false;

    let loop_start = Instant::now();
    let mut report_words = 0u64;
    let mut report_start = Instant::now();

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
        PING_TARGET
    );

    CONNECTIVITY.mark_connecting();

    let net_future = net_runner.run();
    let wifi_future = wifi_connection_loop(&mut wifi_controller, stack, &CONNECTIVITY);
    let ping_future = ping_loop(stack, &CONNECTIVITY);
    let ui_future = async {
        loop {
            sd_refill::handle_pending_refill(
                &mut app,
                &mut sd_stream_states,
                &mut sd_spi,
                &mut sd_cs,
                &mut sd_delay,
                sd_spi_speed_index,
                |spi, speed_index| {
                    let speed_hz = SD_SPI_HZ_CANDIDATES[speed_index];
                    let speed_config = esp_hal::spi::master::Config::default()
                        .with_frequency(Rate::from_hz(speed_hz))
                        .with_mode(esp_hal::spi::Mode::_0);
                    spi.apply_config(&speed_config).is_ok()
                },
            );

            let now_ms = loop_start.elapsed().as_millis();
            let connectivity = CONNECTIVITY.snapshot();
            let app_requests_render = app.tick(now_ms) == TickResult::RenderRequested;
            let current_resume = app.export_resume_state();
            if let Some((resume, reason)) =
                resume_sync.observe(current_resume, app.sleep_eligible(), now_ms)
            {
                if book_db::save_resume_to_db(
                    resume,
                    &sd_stream_states,
                    &mut sd_spi,
                    &mut sd_cs,
                    &mut sd_delay,
                ) {
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
                && book_db::save_resume_to_db(
                    resume,
                    &sd_stream_states,
                    &mut sd_spi,
                    &mut sd_cs,
                    &mut sd_delay,
                )
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
                if let Err(err) = display.flush_frame(&frame, &mut delay) {
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
                    if book_db::save_resume_to_db(
                        resume,
                        &sd_stream_states,
                        &mut sd_spi,
                        &mut sd_cs,
                        &mut sd_delay,
                    ) {
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
                    &mut frame,
                );
                let _ = display.flush_frame(&frame, &mut delay);
                Timer::after_millis(SLEEP_NOTICE_MS).await;
                power::enter_deep_sleep(&mut display, &mut sd_spi, &mut sd_cs);
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
    };

    let _ = embassy_futures::join::join4(net_future, wifi_future, ping_future, ui_future).await;
    unreachable!()
}
