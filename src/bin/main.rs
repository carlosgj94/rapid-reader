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
    gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull},
    spi::master::Spi,
    time::{Duration as HalDuration, Instant, Rate},
    timer::timg::TimerGroup,
};
use esp_radio::wifi::{ClientConfig, ModeConfig, WifiController};
use heapless::{String as HeaplessString, Vec as HeaplessVec};
use log::{debug, info};
use ls027b7dh01::FrameBuffer;
use readily_core::{
    app::{ReaderApp, ReaderConfig, TickResult},
    content::sd_stub::{FakeSdCatalogSource, SD_CATALOG_MAX_TITLES, SD_CATALOG_TITLE_BYTES},
    settings::SettingsStore,
};
use readily_hal_esp32s3::{
    input::rotary::{RotaryConfig, RotaryInput},
    network::{ConnectivityHandle, WifiConfig},
    platform::display::SharpDisplay,
    render::{FrameRenderer, rsvp::RsvpRenderer},
    storage::{
        flash_settings::FlashSettingsStore,
        sd_spi::{
            SdEpubCoverStatus, SdEpubTextChunkResult, SdEpubTextChunkStatus, SdProbeError,
            probe_and_read_epub_cover_thumbnail, probe_and_read_epub_text_chunk,
            probe_and_read_epub_text_chunk_at_chapter,
            probe_and_read_epub_text_chunk_from_resource, probe_and_read_next_epub_text_chunk,
            probe_and_scan_epubs,
        },
    },
};
use static_cell::StaticCell;

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
const WIFI_RETRY_BACKOFF_MIN_SECS: u64 = 2;
const WIFI_RETRY_BACKOFF_MAX_SECS: u64 = 120;
const NETWORK_POLL_INTERVAL_MS: u64 = 500;
const PING_INTERVAL_SECS: u64 = 5;
const PING_IDLE_INTERVAL_SECS: u64 = 20;
const PING_TIMEOUT_MS: u64 = 1_200;
const DHCP_TIMEOUT_SECS: u64 = 15;

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
    esp_println::logger::init_logger_from_env();
    esp_println::println!("boot: readily starting");

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // esp-radio requires an allocator.
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 65536);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    // Wiring used by this demo:
    // CLK=GPIO13, DI=GPIO14, CS=GPIO15, DISP=GPIO7, EMD=GPIO9
    let disp = Output::new(peripherals.GPIO7, Level::Low, OutputConfig::default());
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
    esp_println::println!("display: init begin (CLK=13 DI=14 CS=15 DISP=7 EMD=9)");
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
    let mut content = FakeSdCatalogSource::new();
    let mut sd_stream_states: HeaplessVec<SdBookStreamState, SD_SCAN_MAX_EPUBS> =
        HeaplessVec::new();
    let mut initial_scan_result = None;
    'boot_speed_scan: for speed_index in sd_spi_speed_index..SD_SPI_HZ_CANDIDATES.len() {
        let speed_hz = SD_SPI_HZ_CANDIDATES[speed_index];
        let speed_config = esp_hal::spi::master::Config::default()
            .with_frequency(Rate::from_hz(speed_hz))
            .with_mode(esp_hal::spi::Mode::_0);

        if let Err(err) = sd_spi.apply_config(&speed_config) {
            info!(
                "sd: initial catalog spi config failed (spi_hz={}): {:?}",
                speed_hz, err
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
            >(&mut sd_spi, &mut sd_cs, &mut sd_delay, SD_BOOKS_DIR)
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
                            &mut sd_spi,
                            &mut sd_cs,
                            &mut sd_delay,
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
                                    stream_state.next_offset = text_probe.bytes_read as u32;
                                    stream_state.end_of_resource = text_probe.end_of_resource;
                                    stream_state.ready = !stream_state.text_resource.is_empty();
                                    match content.set_catalog_text_chunk_from_bytes(
                                        index as u16,
                                        &text_chunk[..text_probe.bytes_read],
                                        text_probe.end_of_resource,
                                        text_probe.text_resource.as_str(),
                                    ) {
                                        Ok(applied) => {
                                            let _ = content.set_catalog_stream_chapter_hint(
                                                index as u16,
                                                text_probe.chapter_index,
                                                text_probe.chapter_total,
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
                                                "sd: initial text chunk short_name={} resource={} chapter={}/{} compression={} bytes_read={} end={} applied_loaded={} applied_truncated={} preview={:?}",
                                                epub.short_name,
                                                text_probe.text_resource,
                                                text_probe.chapter_index.saturating_add(1),
                                                text_probe.chapter_total.max(1),
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
                            &mut sd_spi,
                            &mut sd_cs,
                            &mut sd_delay,
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

    let mut frame = FrameBuffer::new();
    let mut last_saved_settings = app.persisted_settings();
    let mut pending_save: Option<(readily_core::settings::PersistedSettings, u64)> = None;
    let mut last_connectivity_revision = u32::MAX;
    let mut display_first_flush_logged = false;

    let loop_start = Instant::now();
    let mut report_words = 0u64;
    let mut report_start = Instant::now();

    info!(
        "Reader started: target_wpm={} dot_pause_ms={} comma_pause_ms={} spi_hz={}",
        reader_config.wpm, reader_config.dot_pause_ms, reader_config.comma_pause_ms, DISPLAY_SPI_HZ
    );
    info!("Display pins: CLK=GPIO13 DI=GPIO14 CS=GPIO15 DISP=GPIO7 EMD=GPIO9");
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
            if let Some(refill_request) =
                app.with_content_mut(|content| content.take_chunk_refill_request())
            {
                let book_index = refill_request.book_index;
                let seek_target_chapter = refill_request.target_chapter;
                debug!(
                    "sd: refill dispatch requested book_index={} seek_target_chapter={:?} known_stream_states={}",
                    book_index,
                    seek_target_chapter.map(|chapter| chapter.saturating_add(1)),
                    sd_stream_states.len()
                );
                if let Some(stream_state) = sd_stream_states.get_mut(book_index as usize) {
                    debug!(
                        "sd: refill dispatch state short_name={} path={} offset={} end_of_resource={} ready={}",
                        stream_state.short_name,
                        stream_state.text_resource,
                        stream_state.next_offset,
                        stream_state.end_of_resource,
                        stream_state.ready
                    );
                    if stream_state.ready || seek_target_chapter.is_some() {
                        let speed_hz = SD_SPI_HZ_CANDIDATES[sd_spi_speed_index];
                        let speed_config = esp_hal::spi::master::Config::default()
                            .with_frequency(Rate::from_hz(speed_hz))
                            .with_mode(esp_hal::spi::Mode::_0);

                        if sd_spi.apply_config(&speed_config).is_ok() {
                            let mut text_chunk = [0u8; SD_TEXT_CHUNK_BYTES];
                            let mut moving_to_next_resource = stream_state.end_of_resource;
                            let mut current_resource = HeaplessString::<SD_TEXT_PATH_BYTES>::new();
                            for ch in stream_state.text_resource.chars() {
                                if current_resource.push(ch).is_err() {
                                    break;
                                }
                            }

                            let mut selected_probe: Option<(
                                SdEpubTextChunkResult<SD_TEXT_PATH_BYTES>,
                                bool,
                            )> = None;
                            let mut exhausted = false;
                            if let Some(target_chapter) = seek_target_chapter {
                                debug!(
                                    "sd: refill seek start short_name={} target_chapter={}",
                                    stream_state.short_name,
                                    target_chapter.saturating_add(1)
                                );
                                match probe_and_read_epub_text_chunk_at_chapter::<
                                    _,
                                    _,
                                    _,
                                    SD_TEXT_PATH_BYTES,
                                >(
                                    &mut sd_spi,
                                    &mut sd_cs,
                                    &mut sd_delay,
                                    SD_BOOKS_DIR,
                                    stream_state.short_name.as_str(),
                                    target_chapter,
                                    &mut text_chunk,
                                ) {
                                    Ok(text_probe) => {
                                        debug!(
                                            "sd: refill seek probe short_name={} status={:?} resource={} chapter={}/{} bytes_read={} end={}",
                                            stream_state.short_name,
                                            text_probe.status,
                                            text_probe.text_resource,
                                            text_probe.chapter_index.saturating_add(1),
                                            text_probe.chapter_total.max(1),
                                            text_probe.bytes_read,
                                            text_probe.end_of_resource
                                        );
                                        selected_probe = Some((text_probe, true));
                                    }
                                    Err(err) => {
                                        match err {
                                            SdProbeError::ChipSelect(_) => {
                                                info!("sd: refill seek failed (chip-select pin)");
                                            }
                                            SdProbeError::Spi(_) => {
                                                info!("sd: refill seek failed (spi transfer)");
                                            }
                                            SdProbeError::Card(card_err) => {
                                                info!(
                                                    "sd: refill seek failed (card init): {:?}",
                                                    card_err
                                                );
                                            }
                                            SdProbeError::Filesystem(fs_err) => {
                                                info!(
                                                    "sd: refill seek failed (filesystem): {:?}",
                                                    fs_err
                                                );
                                            }
                                        }
                                        let _ = app.with_content_mut(|content| {
                                            content.mark_catalog_stream_exhausted(book_index)
                                        });
                                        exhausted = true;
                                    }
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
                                        probe_and_read_next_epub_text_chunk::<
                                            _,
                                            _,
                                            _,
                                            SD_TEXT_PATH_BYTES,
                                        >(
                                            &mut sd_spi,
                                            &mut sd_cs,
                                            &mut sd_delay,
                                            SD_BOOKS_DIR,
                                            stream_state.short_name.as_str(),
                                            current_resource.as_str(),
                                            &mut text_chunk,
                                        )
                                    } else {
                                        probe_and_read_epub_text_chunk_from_resource::<
                                            _,
                                            _,
                                            _,
                                            SD_TEXT_PATH_BYTES,
                                        >(
                                            &mut sd_spi,
                                            &mut sd_cs,
                                            &mut sd_delay,
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
                                                "sd: refill probe result short_name={} status={:?} resource={} chapter={}/{} bytes_read={} end={}",
                                                stream_state.short_name,
                                                text_probe.status,
                                                text_probe.text_resource,
                                                text_probe.chapter_index.saturating_add(1),
                                                text_probe.chapter_total.max(1),
                                                text_probe.bytes_read,
                                                text_probe.end_of_resource
                                            );
                                            if matches!(
                                                text_probe.status,
                                                SdEpubTextChunkStatus::ReadOk
                                            ) && text_probe.bytes_read == 0
                                                && text_probe.end_of_resource
                                            {
                                                info!(
                                                    "sd: refill empty resource short_name={} resource={} moving_next=true",
                                                    stream_state.short_name,
                                                    text_probe.text_resource
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

                                            selected_probe =
                                                Some((text_probe, moving_to_next_resource));
                                            break;
                                        }
                                        Err(err) => {
                                            match err {
                                                SdProbeError::ChipSelect(_) => {
                                                    info!("sd: refill failed (chip-select pin)");
                                                }
                                                SdProbeError::Spi(_) => {
                                                    info!("sd: refill failed (spi transfer)");
                                                }
                                                SdProbeError::Card(card_err) => {
                                                    info!(
                                                        "sd: refill failed (card init): {:?}",
                                                        card_err
                                                    );
                                                }
                                                SdProbeError::Filesystem(fs_err) => {
                                                    info!(
                                                        "sd: refill failed (filesystem): {:?}",
                                                        fs_err
                                                    );
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
                                // already marked exhausted
                            } else if let Some((text_probe, moved_flag)) = selected_probe {
                                if matches!(text_probe.status, SdEpubTextChunkStatus::ReadOk) {
                                    let apply_chunk =
                                        &text_chunk[..text_probe.bytes_read.min(text_chunk.len())];
                                    let mut previous_resource =
                                        HeaplessString::<SD_TEXT_PATH_BYTES>::new();
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
                                        let _ = content.set_catalog_stream_chapter_hint(
                                            book_index,
                                            text_probe.chapter_index,
                                            text_probe.chapter_total,
                                        );
                                        Ok::<_, readily_core::content::sd_stub::SdStubError>(
                                            applied,
                                        )
                                    }) {
                                        Ok(applied) => {
                                            if moved_flag {
                                                stream_state.next_offset =
                                                    text_probe.bytes_read as u32;
                                            } else {
                                                stream_state.next_offset = stream_state
                                                    .next_offset
                                                    .saturating_add(text_probe.bytes_read as u32);
                                            }
                                            stream_state.end_of_resource =
                                                text_probe.end_of_resource;
                                            stream_state.text_resource.clear();
                                            for ch in text_probe.text_resource.chars() {
                                                if stream_state.text_resource.push(ch).is_err() {
                                                    break;
                                                }
                                            }
                                            stream_state.ready =
                                                !stream_state.text_resource.is_empty();
                                            debug!(
                                                "sd: refill apply short_name={} resource={} chapter={}/{} bytes_read={} end={} applied_loaded={} applied_truncated={} next_offset={} next_ready={}",
                                                stream_state.short_name,
                                                stream_state.text_resource,
                                                text_probe.chapter_index.saturating_add(1),
                                                text_probe.chapter_total.max(1),
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
                                                    stream_state.short_name,
                                                    stream_state.next_offset,
                                                    text_probe.bytes_read
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
                                } else {
                                    stream_state.end_of_resource = true;
                                    let _ = app.with_content_mut(|content| {
                                        content.mark_catalog_stream_exhausted(book_index)
                                    });
                                    info!(
                                        "sd: refill stopped short_name={} status={:?}",
                                        stream_state.short_name, text_probe.status
                                    );
                                }
                            } else {
                                let _ = app.with_content_mut(|content| {
                                    content.mark_catalog_stream_exhausted(book_index)
                                });
                                info!(
                                    "sd: refill stopped short_name={} status=no_next_resource_after_empty",
                                    stream_state.short_name
                                );
                            }
                        } else {
                            info!("sd: refill failed (spi config)");
                            let _ = app.with_content_mut(|content| {
                                content.mark_catalog_stream_exhausted(book_index)
                            });
                        }
                    } else {
                        info!(
                            "sd: refill dispatch marking exhausted book_index={} short_name={} reason=stream_state_not_ready path={} offset={} end_of_resource={}",
                            book_index,
                            stream_state.short_name,
                            stream_state.text_resource,
                            stream_state.next_offset,
                            stream_state.end_of_resource
                        );
                        let _ = app.with_content_mut(|content| {
                            content.mark_catalog_stream_exhausted(book_index)
                        });
                    }
                } else {
                    info!(
                        "sd: refill dispatch marking exhausted book_index={} reason=stream_state_missing",
                        book_index
                    );
                    let _ = app.with_content_mut(|content| {
                        content.mark_catalog_stream_exhausted(book_index)
                    });
                }
            }

            let now_ms = loop_start.elapsed().as_millis();
            let connectivity = CONNECTIVITY.snapshot();
            let app_requests_render = app.tick(now_ms) == TickResult::RenderRequested;
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
