#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use embassy_executor::Spawner;
use embassy_time::Timer;
use esp_hal::{
    Blocking,
    clock::CpuClock,
    delay::Delay,
    gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull, RtcPin},
    rtc_cntl::{SocResetReason, reset_reason, wakeup_cause},
    spi::master::Spi,
    system::Cpu,
    time::{Instant, Rate},
    timer::timg::TimerGroup,
};
use esp_radio::wifi::{ClientConfig, ModeConfig};
use heapless::{String as HeaplessString, Vec as HeaplessVec};
use log::{LevelFilter, info};
use ls027b7dh01::FrameBuffer;
use readily_core::{
    app::{ReaderApp, ReaderConfig},
    content::sd_catalog::{SD_CATALOG_MAX_TITLES, SD_CATALOG_TITLE_BYTES, SdCatalogSource},
    settings::SettingsStore,
};
use readily_hal_esp32s3::{
    input::rotary::{RotaryConfig, RotaryInput},
    network::{ConnectivityHandle, WifiConfig},
    platform::display::SharpDisplay,
    render::rsvp::RsvpRenderer,
    storage::flash_settings::FlashSettingsStore,
};
use static_cell::StaticCell;

use loading::{LoadingCoordinator, LoadingEvent, LoadingMode, render_loading_event};
use resume_sync::apply_resume_chapter_hint;

#[path = "main/book_db.rs"]
mod book_db;
#[path = "main/initial_catalog.rs"]
mod initial_catalog;
#[path = "main/loading.rs"]
mod loading;
#[path = "main/network_runtime.rs"]
mod network_runtime;
#[path = "main/power.rs"]
mod power;
#[path = "main/resume_sync.rs"]
mod resume_sync;
#[path = "main/sd_refill.rs"]
mod sd_refill;
#[path = "main/settings_sync.rs"]
mod settings_sync;
#[path = "main/ui_loop.rs"]
mod ui_loop;

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

const WIFI_SSID: &str = env!(
    "READILY_WIFI_SSID",
    "Set READILY_WIFI_SSID in your environment before building/flashing."
);
const WIFI_PASSWORD: &str = env!(
    "READILY_WIFI_PASSWORD",
    "Set READILY_WIFI_PASSWORD in your environment before building/flashing."
);
const WIFI_CONFIG: WifiConfig = WifiConfig::new(WIFI_SSID, WIFI_PASSWORD);

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

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    esp_println::println!("panic: {}", info);
    loop {
        core::hint::spin_loop();
    }
}

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

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
    let mut try_set_sd_speed = |spi: &mut Spi<'_, Blocking>, speed_index| {
        let speed_hz = SD_SPI_HZ_CANDIDATES[speed_index];
        let speed_config = esp_hal::spi::master::Config::default()
            .with_frequency(Rate::from_hz(speed_hz))
            .with_mode(esp_hal::spi::Mode::_0);
        spi.apply_config(&speed_config).is_ok()
    };

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
        render_loading_event(
            &mut loading,
            LoadingEvent::Begin,
            &mut renderer,
            &mut frame,
            &mut display,
            &mut delay,
            &mut display_fault_logged,
            now_ms,
        );
        let books_total = sd_stream_states.len().clamp(0, u16::MAX as usize) as u16;
        let now_ms = loading_start.elapsed().as_millis();
        render_loading_event(
            &mut loading,
            LoadingEvent::ScanResult {
                books_dir_found: true,
                books_total,
            },
            &mut renderer,
            &mut frame,
            &mut display,
            &mut delay,
            &mut display_fault_logged,
            now_ms,
        );
        let now_ms = loading_start.elapsed().as_millis();
        render_loading_event(
            &mut loading,
            LoadingEvent::Finished,
            &mut renderer,
            &mut frame,
            &mut display,
            &mut delay,
            &mut display_fault_logged,
            now_ms,
        );
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
            &mut try_set_sd_speed,
            |event, renderer| {
                let now_ms = loading_start.elapsed().as_millis();
                render_loading_event(
                    &mut loading,
                    event,
                    renderer,
                    &mut frame,
                    &mut display,
                    &mut delay,
                    &mut display_fault_logged,
                    now_ms,
                );
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
                apply_resume_chapter_hint(&mut app, resume);
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
            apply_resume_chapter_hint(&mut app, resume);
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
        apply_resume_chapter_hint(&mut app, resume);
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

    CONNECTIVITY.mark_connecting();

    let net_future = net_runner.run();
    let wifi_future =
        network_runtime::wifi_connection_loop(&mut wifi_controller, stack, &CONNECTIVITY);
    let ping_future = network_runtime::ping_loop(stack, &CONNECTIVITY);
    let ui_future = ui_loop::run(
        &mut app,
        &mut renderer,
        &mut display,
        &mut frame,
        &mut sd_stream_states,
        &mut delay,
        &mut sd_spi,
        &mut sd_cs,
        &mut sd_delay,
        sd_spi_speed_index,
        &mut try_set_sd_speed,
        &mut settings_store,
        &CONNECTIVITY,
        reader_config,
        display_fault_logged,
    );

    let _ = embassy_futures::join::join4(net_future, wifi_future, ping_future, ui_future).await;
    unreachable!()
}
