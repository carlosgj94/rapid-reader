#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use esp_hal::{
    clock::CpuClock,
    delay::Delay,
    gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull},
    main,
    spi::master::Spi,
    time::{Duration, Instant, Rate},
};
use log::info;
use ls027b7dh01::FrameBuffer;
use readily_core::{
    app::{ReaderApp, ReaderConfig, TickResult},
    content::sd_stub::FakeSdCatalogSource,
    settings::SettingsStore,
};
use readily_hal_esp32s3::{
    input::rotary::{RotaryConfig, RotaryInput},
    platform::display::SharpDisplay,
    render::{FrameRenderer, rsvp::RsvpRenderer},
    storage::flash_settings::FlashSettingsStore,
};

const DISPLAY_SPI_HZ: u32 = 1_000_000;
const TITLE: &str = "Readily";
const ORP_ANCHOR_PERCENT: usize = 42;
const COUNTDOWN_SECONDS: u8 = 3;
const ENCODER_DIRECTION_INVERTED: bool = false;
const SETTINGS_SAVE_DEBOUNCE_MS: u64 = 1_500;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[main]
fn main() -> ! {
    esp_println::logger::init_logger_from_env();

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // Wiring used by this demo:
    // CLK=GPIO4, DI=GPIO5, CS=GPIO6, DISP=GPIO7, EMD=GPIO9
    let disp = Output::new(peripherals.GPIO7, Level::Low, OutputConfig::default());
    let emd = Output::new(peripherals.GPIO9, Level::Low, OutputConfig::default());
    let cs = Output::new(peripherals.GPIO6, Level::Low, OutputConfig::default());

    let spi_config = esp_hal::spi::master::Config::default()
        .with_frequency(Rate::from_hz(DISPLAY_SPI_HZ))
        // LS027B7DH01 uses CPOL=0, CPHA=1.
        .with_mode(esp_hal::spi::Mode::_1);

    let spi = Spi::new(peripherals.SPI2, spi_config)
        .unwrap()
        .with_sck(peripherals.GPIO4)
        .with_mosi(peripherals.GPIO5);

    let mut delay = Delay::new();

    let mut display = SharpDisplay::new(spi, disp, emd, cs);
    let _ = display.initialize(&mut delay);
    let _ = display.clear_all(&mut delay);

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

    let content = FakeSdCatalogSource::new();
    let renderer = RsvpRenderer::new(ORP_ANCHOR_PERCENT);

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

    if let Some(store) = settings_store.as_mut() {
        match store.load() {
            Ok(Some(saved)) => {
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

    let mut renderer = renderer;
    let mut frame = FrameBuffer::new();
    let mut last_saved_settings = app.persisted_settings();
    let mut pending_save: Option<(readily_core::settings::PersistedSettings, u64)> = None;

    let loop_start = Instant::now();
    let mut report_words = 0u64;
    let mut report_start = Instant::now();

    info!(
        "Reader started: target_wpm={} dot_pause_ms={} comma_pause_ms={} spi_hz={}",
        reader_config.wpm, reader_config.dot_pause_ms, reader_config.comma_pause_ms, DISPLAY_SPI_HZ
    );
    info!("Encoder pins: CLK=GPIO10 DT=GPIO11 SW=GPIO12");

    loop {
        let now_ms = loop_start.elapsed().as_millis();

        if app.tick(now_ms) == TickResult::RenderRequested {
            app.with_screen(now_ms, |screen| renderer.render(screen, &mut frame));
            let _ = display.flush_frame(&frame, &mut delay);
        }

        let current_settings = app.persisted_settings();
        if current_settings != last_saved_settings {
            match pending_save.as_mut() {
                Some((pending, changed_at_ms)) => {
                    if *pending != current_settings {
                        *pending = current_settings;
                        *changed_at_ms = now_ms;
                    }
                }
                None => {
                    pending_save = Some((current_settings, now_ms));
                }
            }
        }

        if let Some((candidate, changed_at_ms)) = pending_save {
            if now_ms.saturating_sub(changed_at_ms) >= SETTINGS_SAVE_DEBOUNCE_MS {
                if let Some(store) = settings_store.as_mut() {
                    if store.save(&candidate).is_ok() {
                        last_saved_settings = candidate;
                    }
                } else {
                    last_saved_settings = candidate;
                }

                pending_save = None;
            }
        }

        report_words = report_words.saturating_add(app.drain_word_updates() as u64);

        let elapsed = report_start.elapsed();
        if elapsed >= Duration::from_secs(5) {
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

        delay.delay_millis(1);
    }
}
