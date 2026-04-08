#![cfg_attr(
    not(all(
        feature = "telemetry-memtrace",
        feature = "telemetry-verbose-diagnostics"
    )),
    allow(unused_imports, unused_variables)
)]

extern crate alloc;

use ::domain::{
    content::PackageState,
    device::{BootState, DeviceState},
    runtime::{BootstrapSnapshot, Effect, Event},
    sleep::{SleepModel, SleepState},
    storage::StorageRecoveryStatus,
    store::Store,
    sync::SyncStatus,
};
use ::services::storage::StorageError;
use ::services::{input::InputService, sleep::SleepService};
use alloc::boxed::Box;
use app_runtime::{AppRuntime, PreparedScreen, Screen, ScreenUpdate, TransitionPlan};
use core::mem::size_of;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use embassy_executor::Spawner;
use embassy_futures::select::{Either, Either5, select, select5};
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel, signal::Signal,
};
use embassy_time::{Duration, Instant, Ticker, Timer};
use embedded_hal::delay::DelayNs;
use esp_hal::{
    clock::CpuClock,
    delay::Delay,
    gpio::{AnyPin, Level, Output, OutputConfig, Pin, RtcPin},
    rtc_cntl::{Rtc, SocResetReason, reset_reason, wakeup_cause},
    spi::master::Spi,
    system::Cpu,
    time::Rate,
    timer::timg::TimerGroup,
};
use log::{info, warn};
use ls027b7dh01::FrameBuffer;

use crate::{
    backend,
    board::BoardConfig,
    content_storage,
    display::{HEARTBEAT_INTERVAL_MS, PlatformDisplay, diff_dirty_rows},
    input::PlatformInputService,
    internet,
    renderer::{self, AnimationPlayback},
    sleep::enter_deep_sleep_with_button,
    storage::PlatformStorageService,
    telemetry::{bool_flag, capture_heap},
};

const DISPLAY_SPI_HZ: u32 = 2_000_000;
const SD_SPI_INIT_HZ: u32 = 400_000;
const SD_SPI_PRODUCT_RUN_HZ: u32 = 8_000_000;
const SD_SPI_RUN_HZ_OVERRIDE_ENV: &str = "MOTIF_SD_SPI_RUN_HZ";
const INPUT_POLL_MS: u64 = 2;
const READER_TICK_MS: u64 = 20;
const RECLAIMED_INTERNAL_HEAP_BYTES: usize = 64 * 1024;
const PRIMARY_INTERNAL_HEAP_BYTES: usize = 96 * 1024;
// TimedEvent can carry whole manifest snapshots, so this queue must stay small.
const APP_EVENT_QUEUE_CAPACITY: usize = 8;
const PLATFORM_COMMAND_QUEUE_CAPACITY: usize = 4;
const DROP_LOG_SAMPLE_EVERY: u32 = 64;

static APP_EVENT_CH: Channel<CriticalSectionRawMutex, TimedEvent, APP_EVENT_QUEUE_CAPACITY> =
    Channel::new();
static PLATFORM_CMD_CH: Channel<
    CriticalSectionRawMutex,
    PlatformCommand,
    PLATFORM_COMMAND_QUEUE_CAPACITY,
> = Channel::new();
static SCREEN_SIGNAL: Signal<CriticalSectionRawMutex, ScreenUpdate> = Signal::new();
static PENDING_UI_TICK: AtomicBool = AtomicBool::new(false);
static PENDING_READER_TICK: AtomicBool = AtomicBool::new(false);
static DROPPED_UI_TICKS: AtomicU32 = AtomicU32::new(0);
static DROPPED_READER_TICKS: AtomicU32 = AtomicU32::new(0);

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct SdSpiClockConfig {
    init_hz: u32,
    run_hz: u32,
    source: &'static str,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct TimedEvent {
    event: Event,
    at_ms: u64,
}

#[derive(Debug, Clone, Eq, PartialEq)]
enum PlatformCommand {
    RequestDeepSleep,
    PersistBackendCredential(Box<crate::storage::BackendCredential>),
    PersistSettings(domain::settings::PersistedSettings),
}

#[embassy_executor::task]
async fn app_task(snapshot: BootstrapSnapshot) {
    let mut store = Box::new(Store::new());
    store.hydrate_from_bootstrap(snapshot);
    let mut app = Box::new(AppRuntime::new());
    let mut pending_event: Option<TimedEvent> = None;

    info!("settings loaded={:?}", store.settings);
    let mut last_update = Box::new(app.tick(&store));
    SCREEN_SIGNAL.signal(*last_update);

    loop {
        let timed_event = if let Some(event) = pending_event.take() {
            event
        } else {
            APP_EVENT_CH.receive().await
        };
        let timed_event = prioritize_non_tick_event(timed_event, &mut pending_event);
        release_tick_slot(&timed_event.event);
        let input_gesture = match &timed_event.event {
            Event::InputGestureReceived(gesture) => Some(*gesture),
            _ => None,
        };
        let mut effect = store
            .handle_event(timed_event.event, timed_event.at_ms)
            .unwrap_or(Effect::Noop);

        if let Some(gesture) = input_gesture {
            let command = app.handle_input_gesture(gesture);
            let command_effect = store.dispatch(command).unwrap_or(Effect::Noop);
            if !matches!(command_effect, Effect::Noop) {
                effect = command_effect;
            }
        }

        apply_effect(&mut store, effect, timed_event.at_ms).await;
        flush_pending_reading_progress(&mut store).await;

        let next_update = app.tick(&store);
        if next_update.screen != last_update.screen || next_update.prepared != last_update.prepared
        {
            SCREEN_SIGNAL.signal(next_update);
            *last_update = next_update;
        }
    }
}

fn default_sd_spi_clock_config() -> SdSpiClockConfig {
    SdSpiClockConfig {
        init_hz: SD_SPI_INIT_HZ,
        run_hz: SD_SPI_PRODUCT_RUN_HZ,
        source: "product_default",
    }
}

fn resolve_sd_spi_clock_config_from(
    override_raw: Option<&str>,
) -> (SdSpiClockConfig, Option<&str>) {
    match override_raw {
        Some(raw) => match raw.parse::<u32>().ok().filter(|hz| *hz > 0) {
            Some(run_hz) => (
                SdSpiClockConfig {
                    init_hz: SD_SPI_INIT_HZ,
                    run_hz,
                    source: "build_override",
                },
                None,
            ),
            None => (default_sd_spi_clock_config(), Some(raw)),
        },
        None => (default_sd_spi_clock_config(), None),
    }
}

fn resolve_sd_spi_clock_config() -> SdSpiClockConfig {
    let (config, invalid_raw) =
        resolve_sd_spi_clock_config_from(option_env!("MOTIF_SD_SPI_RUN_HZ"));

    if let Some(raw) = invalid_raw {
        info!(
            "sd spi runtime override invalid env={} raw={} defaulting_to={}",
            SD_SPI_RUN_HZ_OVERRIDE_ENV, raw, SD_SPI_PRODUCT_RUN_HZ
        );
    } else if let Some(raw) = option_env!("MOTIF_SD_SPI_RUN_HZ")
        && config.source == "build_override"
    {
        info!(
            "sd spi runtime override accepted env={} raw={} run_hz={}",
            SD_SPI_RUN_HZ_OVERRIDE_ENV, raw, config.run_hz
        );
    }

    config
}

fn prioritize_non_tick_event(
    timed_event: TimedEvent,
    pending_event: &mut Option<TimedEvent>,
) -> TimedEvent {
    if !is_tick_event(&timed_event.event) {
        return timed_event;
    }

    let mut latest_tick = timed_event;
    while let Ok(next_event) = APP_EVENT_CH.try_receive() {
        if is_tick_event(&next_event.event) {
            latest_tick = next_event;
            continue;
        }

        *pending_event = Some(latest_tick);
        return next_event;
    }

    latest_tick
}

async fn apply_effect(store: &mut Store, effect: Effect, at_ms: u64) {
    match effect {
        Effect::EnterDeepSleep => {
            PLATFORM_CMD_CH
                .send(PlatformCommand::RequestDeepSleep)
                .await;
        }
        Effect::CollectionConfirmIgnored { collection, reason } => {
            warn!(
                "collection confirm ignored collection={:?} reason={}",
                collection,
                reason.label()
            );
        }
        Effect::OpenCachedContent(request) => {
            info!(
                "collection confirm open cached collection={:?} content_id={}",
                request.collection,
                request.content_id.as_str(),
            );
            match content_storage::open_cached_reader_package(request.content_id).await {
                Ok(opened) => {
                    let total_units = opened.total_units;
                    let paragraph_count = opened.paragraphs.len();
                    let window_units = opened.window.unit_count;
                    let resume_request = store.open_cached_content(
                        request.collection,
                        request.content_id,
                        request.remote_revision,
                        opened.title,
                        total_units,
                        opened.paragraphs,
                        opened.window,
                    );
                    if let Some(request) = resume_request {
                        load_reader_window_for_request(store, request).await;
                    }
                    info!(
                        "content storage opened cached package collection={:?} content_id={} total_units={} paragraph_count={} window_units={}",
                        request.collection,
                        request.content_id.as_str(),
                        total_units,
                        paragraph_count,
                        window_units,
                    );
                }
                Err(err) => {
                    info!(
                        "content storage open failed collection={:?} content_id={} err={:?}",
                        request.collection,
                        request.content_id.as_str(),
                        err,
                    );
                    let next_state = if matches!(err, StorageError::CorruptData) {
                        PackageState::Missing
                    } else {
                        PackageState::Failed
                    };
                    match content_storage::update_package_state(
                        request.collection,
                        request.remote_item_id,
                        next_state,
                    )
                    .await
                    {
                        Ok(snapshot) => {
                            let _ = store.handle_event(
                                Event::CollectionContentUpdated(
                                    request.collection,
                                    Box::new(snapshot),
                                ),
                                at_ms,
                            );
                        }
                        Err(_) => {
                            let _ = store.handle_event(
                                Event::ContentPackageStateChanged {
                                    collection: request.collection,
                                    remote_item_id: request.remote_item_id,
                                    package_state: next_state,
                                },
                                at_ms,
                            );
                        }
                    }
                    if matches!(err, StorageError::CorruptData)
                        && store.storage.sd_card_ready
                        && matches!(store.backend_sync.status, SyncStatus::Ready)
                    {
                        let _ = store.content_mut().update_package_state(
                            request.collection,
                            &request.remote_item_id,
                            PackageState::Fetching,
                        );
                        info!(
                            "content storage cached content corrupt, refetching collection={:?} content_id={}",
                            request.collection,
                            request.content_id.as_str(),
                        );
                        backend::request_prepare_content(request).await;
                    }
                }
            }
        }
        Effect::LoadReaderWindow(request) => {
            load_reader_window_for_request(store, request).await;
        }
        Effect::PrepareContent(request) => {
            info!(
                "collection confirm prepare content collection={:?} content_id={} remote_item_id={}",
                request.collection,
                request.content_id.as_str(),
                request.remote_item_id.as_str(),
            );
            backend::request_prepare_content(request).await;
        }
        Effect::LoadReaderPauseDetail(request) => {
            backend::request_reader_pause_detail(request).await;
        }
        Effect::ToggleReaderSaved(request) => {
            backend::request_reader_saved_toggle(request).await;
        }
        Effect::ToggleReaderSubscription(request) => {
            backend::request_reader_subscription_toggle(request).await;
        }
        Effect::LoadRecommendationSubtopics => {
            info!("recommendations load subtopics");
            backend::request_recommendation_subtopics().await;
        }
        Effect::LoadRecommendationTopic(request) => {
            info!(
                "recommendations load topic topic_slug={}",
                request.topic_slug.as_str()
            );
            backend::request_recommendation_topic(request).await;
        }
        Effect::RefreshCollection(collection) => {
            backend::request_collection_refresh(collection).await;
        }
        Effect::PersistSettings(settings) => {
            PLATFORM_CMD_CH
                .send(PlatformCommand::PersistSettings(settings))
                .await;
        }
        Effect::Noop => {}
    }
}

async fn load_reader_window_for_request(
    store: &mut Store,
    request: domain::reader::ReaderWindowLoadRequest,
) {
    match content_storage::load_reader_window(request.content_id, request.window_start_unit_index)
        .await
    {
        Ok(window) => {
            info!(
                "content storage loaded reader window content_id={} start_unit={} unit_count={}",
                request.content_id.as_str(),
                window.start_unit_index,
                window.unit_count,
            );
            store.load_reader_window(window);
        }
        Err(err) => {
            info!(
                "content storage reader window load failed content_id={} start_unit={} err={:?}",
                request.content_id.as_str(),
                request.window_start_unit_index,
                err,
            );
            store.reader.clear_pending_window_request();
        }
    }
}

pub async fn run_minimal(spawner: Spawner) -> ! {
    let board = BoardConfig::new();
    let config = esp_hal::Config::default()
        .with_cpu_clock(CpuClock::max())
        .with_psram(esp_hal::psram::PsramConfig::default());
    let peripherals = esp_hal::init(config);

    let boot_reset_reason = reset_reason(Cpu::ProCpu);
    let boot_wakeup_cause = wakeup_cause();
    let woke_from_deep_sleep = boot_reset_reason == Some(SocResetReason::CoreDeepSleep);
    info!(
        "boot reset_reason={:?} wakeup_cause={:?} wake={}",
        boot_reset_reason, boot_wakeup_cause, woke_from_deep_sleep
    );

    let (psram_start, psram_mapped_bytes) = esp_hal::psram::psram_raw_parts(&peripherals.PSRAM);
    let psram_detected = psram_mapped_bytes > 0;

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: RECLAIMED_INTERNAL_HEAP_BYTES);
    // Ticket 07 phase 2: reclaim part of the SRAM we freed from oversized
    // embassy task pools by growing the primary internal allocator region.
    esp_alloc::heap_allocator!(size: PRIMARY_INTERNAL_HEAP_BYTES);
    if psram_detected {
        unsafe {
            esp_alloc::HEAP.add_region(esp_alloc::HeapRegion::new(
                psram_start,
                psram_mapped_bytes,
                esp_alloc::MemoryCapability::External.into(),
            ));
        }
    }
    let heap = capture_heap();
    if psram_detected {
        info!(
            "boot psram detected=true mapped_start=0x{:x} mapped_bytes={} external_regions={} external_heap_size={} external_heap_free={}",
            psram_start as usize,
            psram_mapped_bytes,
            heap.external_regions,
            heap.external_size,
            heap.external_free,
        );
    } else {
        info!(
            "boot psram detected=false mapped_bytes=0 external_regions=0 external_heap_size=0 external_heap_free=0"
        );
    }
    log_heap("after heap init");
    log_static_inventory();
    crate::memory_policy::log_policy_inventory();
    let sd_spi_clock = resolve_sd_spi_clock_config();
    backend::log_static_inventory();
    content_storage::log_static_inventory(
        sd_spi_clock.init_hz,
        sd_spi_clock.run_hz,
        sd_spi_clock.source,
    );
    crate::transfer_tuning::log_runtime_config();
    crate::memtrace!(
        "boot_state",
        "component" = "bootstrap",
        "at_ms" = Instant::now().as_millis(),
        "action" = "heap_initialized",
        "woke_from_deep_sleep" = bool_flag(woke_from_deep_sleep),
        "psram_detected" = bool_flag(psram_detected),
        "psram_mapped_start" = psram_start as usize,
        "psram_mapped_bytes" = psram_mapped_bytes,
        "psram_heap_region_added" = bool_flag(psram_detected),
    );

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    let mut rtc = Rtc::new(peripherals.LPWR);
    let mut storage = PlatformStorageService::mount(peripherals.FLASH);
    let persisted_settings = match storage.read_persisted_settings_sync() {
        Ok(settings) => settings,
        Err(err) => {
            info!("settings hydrate failed: {:?}", err);
            None
        }
    };
    let backend_credential = match storage.read_backend_credential_sync() {
        Ok(credential) => credential,
        Err(err) => {
            info!("backend credential hydrate failed: {:?}", err);
            None
        }
    };
    let sd_spi_config = esp_hal::spi::master::Config::default()
        .with_frequency(Rate::from_hz(sd_spi_clock.init_hz))
        .with_mode(esp_hal::spi::Mode::_0);
    let sd_spi = Spi::new(peripherals.SPI3, sd_spi_config)
        .unwrap()
        .with_sck(peripherals.GPIO4)
        .with_mosi(peripherals.GPIO40)
        .with_miso(peripherals.GPIO41);
    let sd_cs = Output::new(peripherals.GPIO8, Level::High, OutputConfig::default());
    let mut content_mount =
        content_storage::mount(sd_spi, sd_cs, sd_spi_clock.run_hz, sd_spi_clock.source);
    let mut storage_health = storage.health_snapshot().with_sd_card(
        content_mount.sd_card_ready,
        content_mount.sd_total_bytes,
        content_mount.sd_free_bytes,
    );
    if matches!(
        content_mount.last_recovery,
        StorageRecoveryStatus::Recovered
    ) {
        storage_health.last_recovery = StorageRecoveryStatus::Recovered;
    }
    info!("storage health={:?}", storage_health);
    log_heap("after content mount");
    crate::memtrace!(
        "boot_state",
        "component" = "bootstrap",
        "at_ms" = Instant::now().as_millis(),
        "action" = "content_mount",
        "sd_card_ready" = bool_flag(content_mount.sd_card_ready),
        "sd_total_bytes" = content_mount.sd_total_bytes,
        "sd_free_bytes" = content_mount.sd_free_bytes,
        "sd_spi_init_hz" = sd_spi_clock.init_hz,
        "sd_spi_run_hz" = content_mount.sd_run_hz,
        "sd_spi_source" = content_mount.sd_run_hz_source,
        "sd_spi_speed_switch_ok" = bool_flag(content_mount.sd_speed_switch_ok),
        "last_recovery" = storage_health.last_recovery as u8,
    );

    let boot_ms = Instant::now().as_millis();
    let boot_state = if woke_from_deep_sleep {
        BootState::DeepSleepWake
    } else {
        BootState::ColdBoot
    };
    let bootstrap_content =
        content_storage::bootstrap_content_state(content_mount.storage.as_deref_mut());
    let bootstrap_reading_progress =
        content_storage::bootstrap_reading_progress_state(content_mount.storage.as_deref_mut());
    let bootstrap_recommendation_subtopics =
        content_storage::bootstrap_recommendation_subtopics_state(
            content_mount.storage.as_deref_mut(),
        );
    let snapshot = BootstrapSnapshot::new(
        DeviceState {
            pairing: backend::initial_pairing_state(backend_credential),
            boot: boot_state,
        },
        boot_ms,
        bootstrap_content,
        bootstrap_reading_progress,
        bootstrap_recommendation_subtopics,
        persisted_settings,
        storage_health,
        internet::initial_network_state(),
    );

    spawner.spawn(app_task(snapshot)).unwrap();
    content_storage::install(spawner, content_mount.storage);
    let network_stack = internet::install(spawner, peripherals.WIFI);
    backend::install(
        spawner,
        network_stack,
        backend_credential,
        peripherals.RNG,
        peripherals.ADC1,
    );

    let mut input = PlatformInputService::new(
        peripherals.GPIO10.degrade(),
        peripherals.GPIO11.degrade(),
        peripherals.GPIO12.degrade(),
        woke_from_deep_sleep,
    );
    let mut sleep = crate::sleep::PlatformSleepService::new();
    sleep.hydrate_from_boot(woke_from_deep_sleep, boot_ms);
    if let Some(settings) = persisted_settings {
        sleep.configure_inactivity_timeout(settings.inactivity_timeout_ms);
    }

    publish_event(Event::BootCompleted, boot_ms);
    if woke_from_deep_sleep {
        publish_event(Event::WokeFromDeepSleep, boot_ms);
    }

    let disp_pin = peripherals.GPIO2;
    disp_pin.rtcio_pad_hold(false);
    let emd_pin = peripherals.GPIO9;
    emd_pin.rtcio_pad_hold(false);
    let disp = Output::new(disp_pin, Level::Low, OutputConfig::default());
    let emd = Output::new(emd_pin, Level::Low, OutputConfig::default());
    let cs = Output::new(peripherals.GPIO15, Level::Low, OutputConfig::default());

    let spi_config = esp_hal::spi::master::Config::default()
        .with_frequency(Rate::from_hz(DISPLAY_SPI_HZ))
        .with_mode(esp_hal::spi::Mode::_1);
    let spi = Spi::new(peripherals.SPI2, spi_config)
        .unwrap()
        .with_sck(peripherals.GPIO13)
        .with_mosi(peripherals.GPIO14);

    let mut delay = Delay::new();
    let mut display = PlatformDisplay::new(spi, disp, emd, cs);

    if let Err(err) = display.initialize(&mut delay) {
        info!("display initialize failed: {:?}", err);
    }
    if let Err(err) = display.clear_all(&mut delay) {
        info!("display clear failed: {:?}", err);
    }

    log_gpio_contract(&board, sd_spi_clock);

    let mut committed_frame = FrameBuffer::new();
    let mut working_frame = FrameBuffer::new();
    let mut committed_update: Option<ScreenUpdate> = None;
    let mut animation: Option<AnimationPlayback> = None;
    let mut next_animation_deadline: Option<Instant> = None;
    let mut next_heartbeat_deadline = Instant::now() + Duration::from_millis(HEARTBEAT_INTERVAL_MS);

    let mut input_tick = Ticker::every(Duration::from_millis(INPUT_POLL_MS));
    let mut ui_tick = Ticker::every(Duration::from_millis(renderer::UI_TICK_MS));
    let mut reader_tick = Ticker::every(Duration::from_millis(READER_TICK_MS));
    let event_loop = crate::memory_policy::try_external_pinned_box(async move {
        loop {
            let suppress_sleep = current_prepared_screen(animation, committed_update)
                .is_some_and(|screen| prepared_screen_suppresses_sleep(&screen));
            let sleep_deadline = next_sleep_deadline(sleep.model(), suppress_sleep);
            let display_deadline =
                next_display_deadline(next_animation_deadline, next_heartbeat_deadline);

            match select5(
                input_tick.next(),
                select(ui_tick.next(), reader_tick.next()),
                Timer::at(sleep_deadline),
                PLATFORM_CMD_CH.receive(),
                select(Timer::at(display_deadline), SCREEN_SIGNAL.wait()),
            )
            .await
            {
                Either5::First(_) => {
                    let now_ms = Instant::now().as_millis();
                    input.sample(now_ms);

                    let dropped = input.take_dropped_gesture_count();
                    if dropped > 0 {
                        info!("input dropped_gestures={}", dropped);
                    }

                    while let Some(gesture) = input.pop_gesture() {
                        info!("input gesture={:?}", gesture);
                        sleep.note_activity(now_ms);
                        publish_event(Event::InputGestureReceived(gesture), now_ms);
                    }
                }
                Either5::Second(tick_kind) => {
                    let now_ms = Instant::now().as_millis();

                    match tick_kind {
                        Either::First(_) => {
                            if current_prepared_screen(animation, committed_update)
                                .is_some_and(|screen| prepared_screen_drives_ui_ticks(&screen))
                            {
                                publish_event(Event::UiTick(now_ms), now_ms);
                            }
                        }
                        Either::Second(_) => {
                            if reader_ticks_are_active(animation, committed_update) {
                                publish_event(Event::ReaderTick(now_ms), now_ms);
                            }
                        }
                    }
                }
                Either5::Third(_) => {
                    enter_low_power_sleep(
                        &board,
                        &mut display,
                        &mut delay,
                        &mut input,
                        &mut sleep,
                        &mut rtc,
                    );
                }
                Either5::Fourth(command) => match command {
                    PlatformCommand::RequestDeepSleep => {
                        enter_low_power_sleep(
                            &board,
                            &mut display,
                            &mut delay,
                            &mut input,
                            &mut sleep,
                            &mut rtc,
                        );
                    }
                    PlatformCommand::PersistBackendCredential(credential) => {
                        if let Err(err) = storage.write_backend_credential_sync(&credential) {
                            info!("persist backend credential failed: {:?}", err);
                        }
                    }
                    PlatformCommand::PersistSettings(settings) => {
                        sleep.configure_inactivity_timeout(settings.inactivity_timeout_ms);
                        if let Err(err) = storage.write_persisted_settings_sync(&settings) {
                            info!("persist settings failed: {:?}", err);
                        }
                    }
                },
                Either5::Fifth(display_event) => match display_event {
                    Either::First(_) => {
                        let now = Instant::now();

                        if next_animation_deadline.is_some_and(|deadline| deadline <= now) {
                            if let Some(active_animation) = animation {
                                let next_frame = active_animation.advance();
                                present_transition_frame(
                                    &mut display,
                                    &mut committed_frame,
                                    &mut working_frame,
                                    &mut delay,
                                    &next_frame,
                                );
                                next_heartbeat_deadline = schedule_heartbeat_deadline();

                                if next_frame.is_complete() {
                                    committed_update = Some(ScreenUpdate {
                                        screen: next_frame.screen,
                                        prepared: next_frame.target_screen(),
                                        transition: TransitionPlan::none(),
                                    });
                                    animation = None;
                                    next_animation_deadline = None;
                                } else {
                                    animation = Some(next_frame);
                                    next_animation_deadline =
                                        Some(schedule_animation_deadline(next_frame.plan.frame_ms));
                                }
                            } else {
                                next_animation_deadline = None;
                            }
                        } else if now >= next_heartbeat_deadline {
                            if let Err(err) = display.heartbeat(&mut delay) {
                                info!("display heartbeat failed: {:?}", err);
                                let _ = display.disable_output();
                            } else {
                                next_heartbeat_deadline = schedule_heartbeat_deadline();
                            }
                        }
                    }
                    Either::Second(update) => {
                        let previous_screen = animation
                            .map(|active| active.screen)
                            .or(committed_update.map(|committed| committed.screen));
                        let previous_prepared = animation
                            .map(|active| active.target_screen())
                            .or(committed_update.map(|committed| committed.prepared));

                        if previous_screen != Some(update.screen) {
                            info!("app screen={:?}", update.screen);
                        }

                        if update.screen == Screen::Reader
                            && previous_screen != Some(Screen::Reader)
                        {
                            let reset = input.reset_after_reader_open();
                            if reset.cleared_gestures > 0
                                || reset.cleared_dropped_gestures > 0
                                || reset.button_was_pressed
                            {
                                info!(
                                    "input reset for reader cleared_gestures={} cleared_dropped={} button_pressed={}",
                                    reset.cleared_gestures,
                                    reset.cleared_dropped_gestures,
                                    reset.button_was_pressed,
                                );
                            }
                        }

                        if let Some(previous) = previous_prepared {
                            if update.transition == TransitionPlan::none() {
                                animation = None;
                                next_animation_deadline = None;
                                committed_update = Some(update);
                                present_prepared_screen(
                                    &mut display,
                                    &mut committed_frame,
                                    &mut working_frame,
                                    &mut delay,
                                    &update.prepared,
                                );
                                next_heartbeat_deadline = schedule_heartbeat_deadline();
                            } else {
                                let next_animation = AnimationPlayback::new(previous, update);
                                present_transition_frame(
                                    &mut display,
                                    &mut committed_frame,
                                    &mut working_frame,
                                    &mut delay,
                                    &next_animation,
                                );
                                next_heartbeat_deadline = schedule_heartbeat_deadline();

                                if next_animation.is_complete() {
                                    committed_update = Some(ScreenUpdate {
                                        screen: next_animation.screen,
                                        prepared: next_animation.target_screen(),
                                        transition: TransitionPlan::none(),
                                    });
                                    animation = None;
                                    next_animation_deadline = None;
                                } else {
                                    animation = Some(next_animation);
                                    next_animation_deadline = Some(schedule_animation_deadline(
                                        next_animation.plan.frame_ms,
                                    ));
                                }
                            }
                        } else {
                            animation = None;
                            next_animation_deadline = None;
                            committed_update = Some(update);
                            present_prepared_screen(
                                &mut display,
                                &mut committed_frame,
                                &mut working_frame,
                                &mut delay,
                                &update.prepared,
                            );
                            next_heartbeat_deadline = schedule_heartbeat_deadline();
                        }
                    }
                },
            }
        }
    });
    let event_loop = match event_loop {
        Ok(event_loop) => event_loop,
        Err(_) => panic!("bootstrap event loop alloc failed"),
    };
    event_loop.await
}

pub(crate) fn publish_event(event: Event, at_ms: u64) {
    let is_ui_tick = matches!(event, Event::UiTick(_));
    let is_reader_tick = matches!(event, Event::ReaderTick(_));

    if !reserve_tick_slot(&event) {
        return;
    }

    match APP_EVENT_CH.try_send(TimedEvent { event, at_ms }) {
        Ok(()) => {
            if !is_ui_tick && !is_reader_tick {
                flush_tick_drop_logs();
            }
        }
        Err(embassy_sync::channel::TrySendError::Full(timed_event)) => {
            release_tick_slot(&timed_event.event);
            if is_ui_tick {
                record_tick_drop(&DROPPED_UI_TICKS, "ui");
            } else if is_reader_tick {
                record_tick_drop(&DROPPED_READER_TICKS, "reader");
            } else {
                flush_tick_drop_logs();
                info!("app event dropped: {:?}", timed_event.event);
            }
        }
    }
}

const fn is_tick_event(event: &Event) -> bool {
    matches!(event, Event::UiTick(_) | Event::ReaderTick(_))
}

pub(crate) async fn persist_backend_credential(credential: crate::storage::BackendCredential) {
    PLATFORM_CMD_CH
        .send(PlatformCommand::PersistBackendCredential(Box::new(
            credential,
        )))
        .await;
}

async fn flush_pending_reading_progress(store: &mut Store) {
    while let Some(entry) = store.take_pending_reading_progress_write() {
        if let Err(err) = content_storage::queue_reading_progress_write(entry).await {
            info!(
                "content storage reading progress persist failed content_id={} remote_revision={} paragraph_index={} total_paragraphs={} err={:?}",
                entry.content_id.as_str(),
                entry.remote_revision,
                entry.paragraph_index,
                entry.total_paragraphs,
                err,
            );
            break;
        }
    }
}

fn current_prepared_screen(
    animation: Option<AnimationPlayback>,
    committed_update: Option<ScreenUpdate>,
) -> Option<PreparedScreen> {
    animation
        .map(|active| active.target_screen())
        .or(committed_update.map(|update| update.prepared))
}

fn prepared_screen_suppresses_sleep(screen: &PreparedScreen) -> bool {
    prepared_screen_shows_startup_splash(screen)
        || prepared_screen_drives_reader_ticks(screen)
        || prepared_screen_shows_reader_loading(screen)
        || prepared_screen_shows_collection_fetch(screen)
        || prepared_screen_shows_dashboard_sync(screen)
}

fn prepared_screen_shows_startup_splash(screen: &PreparedScreen) -> bool {
    matches!(screen, PreparedScreen::StartupSplash(_))
}

fn prepared_screen_drives_reader_ticks(screen: &PreparedScreen) -> bool {
    matches!(screen, PreparedScreen::Reader(shell) if shell.modal.is_none())
}

fn prepared_screen_shows_reader_loading(screen: &PreparedScreen) -> bool {
    matches!(
        screen,
        PreparedScreen::Reader(shell)
            if matches!(shell.modal, Some(app_runtime::components::ReaderModal::Loading(_)))
    )
}

fn reader_ticks_are_active(
    animation: Option<AnimationPlayback>,
    committed_update: Option<ScreenUpdate>,
) -> bool {
    if animation.is_some() {
        return false;
    }

    committed_update.is_some_and(|update| prepared_screen_drives_reader_ticks(&update.prepared))
}

fn prepared_screen_shows_collection_fetch(screen: &PreparedScreen) -> bool {
    match screen {
        PreparedScreen::Collection(shell) => shell.rows.iter().any(|row| row.is_fetching),
        _ => false,
    }
}

fn prepared_screen_shows_dashboard_sync(screen: &PreparedScreen) -> bool {
    matches!(screen, PreparedScreen::Dashboard(shell) if shell.sync_indicator.is_some())
}

fn prepared_screen_drives_ui_ticks(screen: &PreparedScreen) -> bool {
    match screen {
        PreparedScreen::StartupSplash(_) => true,
        PreparedScreen::Dashboard(_) => true,
        PreparedScreen::Reader(shell) => matches!(
            shell.modal,
            Some(app_runtime::components::ReaderModal::Loading(_))
        ),
        PreparedScreen::Settings(shell) => {
            matches!(shell.mode, domain::ui::SettingsMode::RefreshLoading)
        }
        _ => false,
    }
}

fn reserve_tick_slot(event: &Event) -> bool {
    match event {
        Event::UiTick(_) => PENDING_UI_TICK
            .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok(),
        Event::ReaderTick(_) => PENDING_READER_TICK
            .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok(),
        _ => true,
    }
}

fn release_tick_slot(event: &Event) {
    match event {
        Event::UiTick(_) => PENDING_UI_TICK.store(false, Ordering::Relaxed),
        Event::ReaderTick(_) => PENDING_READER_TICK.store(false, Ordering::Relaxed),
        _ => {}
    }
}

fn record_tick_drop(counter: &AtomicU32, label: &str) {
    let dropped = counter.fetch_add(1, Ordering::Relaxed) + 1;
    if dropped == 1 || dropped.is_multiple_of(DROP_LOG_SAMPLE_EVERY) {
        info!("app {} ticks dropped={} (aggregated)", label, dropped);
    }
}

fn flush_tick_drop_logs() {
    let ui = DROPPED_UI_TICKS.swap(0, Ordering::Relaxed);
    let reader = DROPPED_READER_TICKS.swap(0, Ordering::Relaxed);

    if ui > 0 || reader > 0 {
        info!("app tick drops ui={} reader={}", ui, reader);
    }
}

fn next_sleep_deadline(model: &SleepModel, suppress_inactivity_sleep: bool) -> Instant {
    if matches!(model.state, SleepState::SleepRequested) {
        return Instant::now();
    }

    if suppress_inactivity_sleep {
        return Instant::from_millis(u64::MAX);
    }

    Instant::from_millis(
        model
            .last_activity_ms
            .saturating_add(model.config.inactivity_timeout_ms),
    )
}

fn next_display_deadline(
    next_animation_deadline: Option<Instant>,
    next_heartbeat_deadline: Instant,
) -> Instant {
    next_animation_deadline
        .map(|deadline| deadline.min(next_heartbeat_deadline))
        .unwrap_or(next_heartbeat_deadline)
}

fn schedule_animation_deadline(frame_ms: u16) -> Instant {
    Instant::now() + Duration::from_millis(frame_ms.max(1) as u64)
}

fn schedule_heartbeat_deadline() -> Instant {
    Instant::now() + Duration::from_millis(HEARTBEAT_INTERVAL_MS)
}

fn present_prepared_screen<SPI, DISP, EMD, CS, D>(
    display: &mut PlatformDisplay<SPI, DISP, EMD, CS>,
    committed: &mut FrameBuffer,
    working: &mut FrameBuffer,
    delay: &mut D,
    screen: &PreparedScreen,
) where
    SPI: embedded_hal::spi::SpiBus<u8>,
    DISP: embedded_hal::digital::OutputPin,
    EMD: embedded_hal::digital::OutputPin,
    CS: embedded_hal::digital::OutputPin,
    D: DelayNs,
{
    renderer::draw_prepared_screen(working, screen);
    let dirty_rows = diff_dirty_rows(committed, working);
    present_frame(display, committed, working, &dirty_rows, delay);
}

fn present_transition_frame<SPI, DISP, EMD, CS, D>(
    display: &mut PlatformDisplay<SPI, DISP, EMD, CS>,
    committed: &mut FrameBuffer,
    working: &mut FrameBuffer,
    delay: &mut D,
    animation: &AnimationPlayback,
) where
    SPI: embedded_hal::spi::SpiBus<u8>,
    DISP: embedded_hal::digital::OutputPin,
    EMD: embedded_hal::digital::OutputPin,
    CS: embedded_hal::digital::OutputPin,
    D: DelayNs,
{
    renderer::draw_transition_frame(working, animation);
    let dirty_rows = diff_dirty_rows(committed, working);
    present_frame(display, committed, working, &dirty_rows, delay);
}

fn present_frame<SPI, DISP, EMD, CS, D>(
    display: &mut PlatformDisplay<SPI, DISP, EMD, CS>,
    committed: &mut FrameBuffer,
    working: &FrameBuffer,
    dirty_rows: &ls027b7dh01::DirtyRows,
    delay: &mut D,
) where
    SPI: embedded_hal::spi::SpiBus<u8>,
    DISP: embedded_hal::digital::OutputPin,
    EMD: embedded_hal::digital::OutputPin,
    CS: embedded_hal::digital::OutputPin,
    D: DelayNs,
{
    match display.present(committed, working, dirty_rows, delay) {
        Ok(_stats) => {}
        Err(err) => {
            info!("display flush failed: {:?}", err);
            let _ = display.disable_output();
        }
    }
}

fn enter_low_power_sleep<SPI, DISP, EMD, CS, D>(
    board: &BoardConfig,
    display: &mut PlatformDisplay<SPI, DISP, EMD, CS>,
    delay: &mut D,
    input: &mut PlatformInputService<'_>,
    sleep: &mut crate::sleep::PlatformSleepService,
    rtc: &mut Rtc<'_>,
) -> !
where
    SPI: embedded_hal::spi::SpiBus<u8>,
    DISP: embedded_hal::digital::OutputPin,
    EMD: embedded_hal::digital::OutputPin,
    CS: embedded_hal::digital::OutputPin,
    D: DelayNs,
{
    let now_ms = Instant::now().as_millis();
    info!(
        "sleep due now_ms={} last_activity_ms={} wake_reason={:?}",
        now_ms,
        sleep.model().last_activity_ms,
        sleep.model().last_wake_reason
    );
    if let Err(err) = display.enter_low_power(delay) {
        info!("display low-power transition failed: {:?}", err);
    }
    hold_display_sleep_pins(board);
    let wake_button = input.take_wake_button();
    enter_deep_sleep_with_button(sleep, rtc, wake_button);
}

fn log_gpio_contract(board: &BoardConfig, sd_spi_clock: SdSpiClockConfig) {
    info!(
        "display gpio clk={} di={} cs={} disp={} emd={}",
        board.display_clk_gpio,
        board.display_di_gpio,
        board.display_cs_gpio,
        board.display_disp_gpio,
        board.display_emd_gpio
    );
    info!(
        "sd gpio cs={} sck={} mosi={} miso={}",
        board.sd_cs_gpio, board.sd_sck_gpio, board.sd_mosi_gpio, board.sd_miso_gpio
    );
    info!(
        "sd spi hz init={} run={} source={}",
        sd_spi_clock.init_hz, sd_spi_clock.run_hz, sd_spi_clock.source
    );
    info!(
        "encoder gpio clk={} dt={} sw={}",
        board.encoder_clk_gpio, board.encoder_dt_gpio, board.encoder_sw_gpio
    );
    info!(
        "sleep wake gpio={} inactivity_ms=30000",
        board.sleep_wake_gpio
    );
}

fn log_static_inventory() {
    crate::memtrace!(
        "static_inventory",
        "component" = "bootstrap",
        "at_ms" = Instant::now().as_millis(),
        "main_loop_future_storage" = "external_pinned_box",
        "timed_event_bytes" = size_of::<TimedEvent>(),
        "platform_command_bytes" = size_of::<PlatformCommand>(),
        "bootstrap_snapshot_bytes" = size_of::<BootstrapSnapshot>(),
        "screen_update_bytes" = size_of::<ScreenUpdate>(),
        "prepared_screen_bytes" = size_of::<PreparedScreen>(),
        "animation_playback_bytes" = size_of::<AnimationPlayback>(),
        "framebuffer_bytes" = size_of::<FrameBuffer>(),
        "app_event_queue_capacity" = APP_EVENT_QUEUE_CAPACITY,
        "app_event_queue_resident_bytes" = APP_EVENT_QUEUE_CAPACITY * size_of::<TimedEvent>(),
        "platform_command_queue_capacity" = PLATFORM_COMMAND_QUEUE_CAPACITY,
        "platform_command_queue_resident_bytes" =
            PLATFORM_COMMAND_QUEUE_CAPACITY * size_of::<PlatformCommand>(),
    );
}

fn log_heap(label: &str) {
    let stats = capture_heap();
    info!(
        "heap label={} size={} used={} free={} internal_size={} internal_used={} internal_free={} internal_peak_used={} internal_min_free={} external_size={} external_used={} external_free={} external_peak_used={} external_min_free={}",
        label,
        stats.size,
        stats.used,
        stats.free,
        stats.internal_size,
        stats.internal_used,
        stats.internal_free,
        stats.internal_peak_used,
        stats.internal_min_free,
        stats.external_size,
        stats.external_used,
        stats.external_free,
        stats.external_peak_used,
        stats.external_min_free,
    );
    info!(
        "heap regions label={} region0_kind={} region0_used={} region0_free={} region0_peak_used={} region0_min_free={} region1_kind={} region1_used={} region1_free={} region1_peak_used={} region1_min_free={} region2_kind={} region2_used={} region2_free={} region2_peak_used={} region2_min_free={}",
        label,
        stats.regions[0].kind,
        stats.regions[0].used,
        stats.regions[0].free,
        stats.regions[0].peak_used,
        stats.regions[0].min_free,
        stats.regions[1].kind,
        stats.regions[1].used,
        stats.regions[1].free,
        stats.regions[1].peak_used,
        stats.regions[1].min_free,
        stats.regions[2].kind,
        stats.regions[2].used,
        stats.regions[2].free,
        stats.regions[2].peak_used,
        stats.regions[2].min_free,
    );
}

fn hold_display_sleep_pins(board: &BoardConfig) {
    unsafe {
        let disp_pin = AnyPin::steal(board.display_disp_gpio);
        disp_pin.rtcio_pad_hold(true);

        let emd_pin = AnyPin::steal(board.display_emd_gpio);
        emd_pin.rtcio_pad_hold(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::sleep::{SleepConfig, WakeReason};

    fn reader_shell(
        modal: Option<app_runtime::components::ReaderModal>,
    ) -> app_runtime::components::ReaderShell {
        app_runtime::components::ReaderShell {
            appearance: domain::settings::AppearanceMode::Light,
            stage: app_runtime::components::RsvpStage {
                title: domain::text::InlineText::from_slice("TEST"),
                wpm: 260,
                left_word: domain::text::InlineText::new(),
                right_word: domain::text::InlineText::new(),
                preview: domain::text::InlineText::new(),
                font: domain::formatter::StageFont::Large,
                progress_width: 0,
            },
            badge: None,
            modal,
        }
    }

    fn dashboard_shell(
        sync_indicator: Option<app_runtime::components::SyncIndicator>,
    ) -> app_runtime::components::DashboardShell {
        app_runtime::components::DashboardShell {
            appearance: domain::settings::AppearanceMode::Light,
            status: app_runtime::components::StatusCluster {
                battery_percent: 82,
                wifi_online: true,
            },
            sync_indicator,
            rail: app_runtime::components::VerticalRail { text: "HOME" },
            items: [
                app_runtime::components::DashboardItem {
                    label: "INBOX",
                    live_dot: false,
                    selected: false,
                },
                app_runtime::components::DashboardItem {
                    label: "SAVED",
                    live_dot: true,
                    selected: true,
                },
                app_runtime::components::DashboardItem {
                    label: "FOR YOU",
                    live_dot: false,
                    selected: false,
                },
            ],
            band: app_runtime::components::SelectionBand { y: 106, height: 68 },
        }
    }

    fn startup_splash_shell() -> app_runtime::components::StartupSplashShell {
        app_runtime::components::StartupSplashShell {
            appearance: domain::settings::AppearanceMode::Light,
            progress_width: 120,
            stripe_phase: 3,
            skip_hint: "long press to skip sync",
        }
    }

    #[test]
    fn requested_sleep_uses_immediate_deadline() {
        let mut model = SleepModel::new(SleepConfig::new(30_000));
        model.request_sleep();

        assert!(next_sleep_deadline(&model, false) <= Instant::now());
    }

    #[test]
    fn sd_spi_clock_defaults_to_product_value() {
        let (config, invalid_raw) = resolve_sd_spi_clock_config_from(None);

        assert_eq!(
            config,
            SdSpiClockConfig {
                init_hz: SD_SPI_INIT_HZ,
                run_hz: SD_SPI_PRODUCT_RUN_HZ,
                source: "product_default",
            }
        );
        assert_eq!(invalid_raw, None);
    }

    #[test]
    fn sd_spi_clock_accepts_valid_override() {
        let (config, invalid_raw) = resolve_sd_spi_clock_config_from(Some("12000000"));

        assert_eq!(
            config,
            SdSpiClockConfig {
                init_hz: SD_SPI_INIT_HZ,
                run_hz: 12_000_000,
                source: "build_override",
            }
        );
        assert_eq!(invalid_raw, None);
    }

    #[test]
    fn sd_spi_clock_rejects_invalid_override() {
        let (config, invalid_raw) = resolve_sd_spi_clock_config_from(Some("oops"));

        assert_eq!(config, default_sd_spi_clock_config());
        assert_eq!(invalid_raw, Some("oops"));
    }

    #[test]
    fn inactivity_deadline_tracks_last_activity() {
        let mut model = SleepModel::new(SleepConfig::new(30_000));
        model.mark_woke(WakeReason::ColdBoot, 1_000);

        assert_eq!(
            next_sleep_deadline(&model, false),
            Instant::from_millis(31_000)
        );
    }

    #[test]
    fn active_reader_suppresses_inactivity_deadline() {
        let mut model = SleepModel::new(SleepConfig::new(30_000));
        model.mark_woke(WakeReason::ColdBoot, 1_000);

        assert_eq!(
            next_sleep_deadline(&model, true),
            Instant::from_millis(u64::MAX)
        );
    }

    #[test]
    fn prepared_reader_without_modal_suppresses_sleep() {
        let screen = PreparedScreen::Reader(reader_shell(None));

        assert!(prepared_screen_suppresses_sleep(&screen));
    }

    #[test]
    fn startup_splash_suppresses_sleep() {
        let screen = PreparedScreen::StartupSplash(startup_splash_shell());

        assert!(prepared_screen_suppresses_sleep(&screen));
    }

    #[test]
    fn startup_splash_drives_ui_ticks() {
        let screen = PreparedScreen::StartupSplash(startup_splash_shell());

        assert!(prepared_screen_drives_ui_ticks(&screen));
    }

    #[test]
    fn paused_reader_does_not_suppress_sleep() {
        let screen =
            PreparedScreen::Reader(reader_shell(Some(app_runtime::components::PauseModal {
                title: "PAUSED",
                rows: [
                    app_runtime::components::PauseModalRow {
                        label: "A",
                        action: "A",
                        selected: true,
                        enabled: true,
                    },
                    app_runtime::components::PauseModalRow {
                        label: "B",
                        action: "B",
                        selected: false,
                        enabled: true,
                    },
                    app_runtime::components::PauseModalRow {
                        label: "C",
                        action: "C",
                        selected: false,
                        enabled: true,
                    },
                    app_runtime::components::PauseModalRow {
                        label: "D",
                        action: "D",
                        selected: false,
                        enabled: true,
                    },
                ],
            })));

        assert!(!prepared_screen_suppresses_sleep(&screen));
    }

    #[test]
    fn committed_reader_without_animation_drives_reader_ticks() {
        let committed = ScreenUpdate {
            screen: Screen::Reader,
            prepared: PreparedScreen::Reader(reader_shell(None)),
            transition: TransitionPlan::none(),
        };

        assert!(reader_ticks_are_active(None, Some(committed)));
    }

    #[test]
    fn reader_enter_animation_blocks_reader_ticks_until_commit() {
        let committed = ScreenUpdate {
            screen: Screen::Saved,
            prepared: PreparedScreen::Collection(app_runtime::components::ContentListShell {
                appearance: domain::settings::AppearanceMode::Light,
                status: app_runtime::components::StatusCluster {
                    battery_percent: 82,
                    wifi_online: true,
                },
                rail: app_runtime::components::VerticalRail { text: "SAVED" },
                large_rail: true,
                recommendations_bar: None,
                rows: [
                    app_runtime::components::ContentRow {
                        meta: "A",
                        title: "A",
                        progress_badge: None,
                        is_fetching: false,
                        selected: false,
                    },
                    app_runtime::components::ContentRow {
                        meta: "B",
                        title: "B",
                        progress_badge: None,
                        is_fetching: false,
                        selected: true,
                    },
                    app_runtime::components::ContentRow {
                        meta: "C",
                        title: "C",
                        progress_badge: None,
                        is_fetching: false,
                        selected: false,
                    },
                ],
                band: app_runtime::components::SelectionBand { y: 106, height: 68 },
                help: app_runtime::components::HelpHint { text: "BACK" },
            }),
            transition: TransitionPlan::none(),
        };
        let animation = AnimationPlayback::new(
            committed.prepared,
            ScreenUpdate {
                screen: Screen::Reader,
                prepared: PreparedScreen::Reader(reader_shell(None)),
                transition: TransitionPlan::new(
                    app_runtime::AnimationDescriptor::ReaderEnter,
                    3,
                    50,
                ),
            },
        );

        assert!(!reader_ticks_are_active(Some(animation), Some(committed)));
    }

    #[test]
    fn modal_hide_animation_blocks_reader_ticks_until_commit() {
        let paused = reader_shell(Some(app_runtime::components::PauseModal {
            title: "PAUSED",
            rows: [
                app_runtime::components::PauseModalRow {
                    label: "A",
                    action: "A",
                    selected: true,
                    enabled: true,
                },
                app_runtime::components::PauseModalRow {
                    label: "B",
                    action: "B",
                    selected: false,
                    enabled: true,
                },
                app_runtime::components::PauseModalRow {
                    label: "C",
                    action: "C",
                    selected: false,
                    enabled: true,
                },
                app_runtime::components::PauseModalRow {
                    label: "D",
                    action: "D",
                    selected: false,
                    enabled: true,
                },
            ],
        }));
        let committed = ScreenUpdate {
            screen: Screen::Reader,
            prepared: PreparedScreen::Reader(paused),
            transition: TransitionPlan::none(),
        };
        let animation = AnimationPlayback::new(
            committed.prepared,
            ScreenUpdate {
                screen: Screen::Reader,
                prepared: PreparedScreen::Reader(reader_shell(None)),
                transition: TransitionPlan::new(app_runtime::AnimationDescriptor::ModalHide, 3, 55),
            },
        );

        assert!(!reader_ticks_are_active(Some(animation), Some(committed)));
    }

    #[test]
    fn dashboard_with_sync_indicator_suppresses_sleep() {
        let screen = PreparedScreen::Dashboard(dashboard_shell(Some(
            app_runtime::components::SyncIndicator {
                label: "syncing...",
                spinner_phase: 2,
            },
        )));

        assert!(prepared_screen_suppresses_sleep(&screen));
    }

    #[test]
    fn dashboard_without_sync_indicator_does_not_suppress_sleep() {
        let screen = PreparedScreen::Dashboard(dashboard_shell(None));

        assert!(!prepared_screen_suppresses_sleep(&screen));
    }

    #[test]
    fn collection_with_fetching_row_suppresses_sleep() {
        let screen = PreparedScreen::Collection(app_runtime::components::ContentListShell {
            appearance: domain::settings::AppearanceMode::Light,
            status: app_runtime::components::StatusCluster {
                battery_percent: 82,
                wifi_online: true,
            },
            rail: app_runtime::components::VerticalRail { text: "SAVED" },
            large_rail: true,
            recommendations_bar: None,
            rows: [
                app_runtime::components::ContentRow {
                    meta: "SOURCE",
                    title: "Previous",
                    progress_badge: None,
                    is_fetching: false,
                    selected: false,
                },
                app_runtime::components::ContentRow {
                    meta: "SOURCE",
                    title: "Fetching",
                    progress_badge: None,
                    is_fetching: true,
                    selected: true,
                },
                app_runtime::components::ContentRow {
                    meta: "SOURCE",
                    title: "Next",
                    progress_badge: None,
                    is_fetching: false,
                    selected: false,
                },
            ],
            band: app_runtime::components::SelectionBand { y: 106, height: 68 },
            help: app_runtime::components::HelpHint {
                text: "long press_",
            },
        });

        assert!(prepared_screen_suppresses_sleep(&screen));
    }

    #[test]
    fn collection_without_fetching_row_does_not_suppress_sleep() {
        let screen = PreparedScreen::Collection(app_runtime::components::ContentListShell {
            appearance: domain::settings::AppearanceMode::Light,
            status: app_runtime::components::StatusCluster {
                battery_percent: 82,
                wifi_online: true,
            },
            rail: app_runtime::components::VerticalRail { text: "SAVED" },
            large_rail: true,
            recommendations_bar: None,
            rows: [
                app_runtime::components::ContentRow {
                    meta: "SOURCE",
                    title: "Previous",
                    progress_badge: None,
                    is_fetching: false,
                    selected: false,
                },
                app_runtime::components::ContentRow {
                    meta: "SOURCE",
                    title: "Current",
                    progress_badge: None,
                    is_fetching: false,
                    selected: true,
                },
                app_runtime::components::ContentRow {
                    meta: "SOURCE",
                    title: "Next",
                    progress_badge: None,
                    is_fetching: false,
                    selected: false,
                },
            ],
            band: app_runtime::components::SelectionBand { y: 106, height: 68 },
            help: app_runtime::components::HelpHint {
                text: "long press_",
            },
        });

        assert!(!prepared_screen_suppresses_sleep(&screen));
    }
}
