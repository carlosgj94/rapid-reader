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
use core::sync::atomic::{AtomicU32, Ordering};
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
use log::info;
use ls027b7dh01::FrameBuffer;

use crate::{
    backend,
    board::BoardConfig,
    content_storage,
    display::PlatformDisplay,
    input::PlatformInputService,
    internet,
    renderer::{self, AnimationPlayback},
    sleep::enter_deep_sleep_with_button,
    storage::PlatformStorageService,
};

const DISPLAY_SPI_HZ: u32 = 1_000_000;
const SD_SPI_HZ: u32 = 400_000;
const INPUT_POLL_MS: u64 = 2;
const READER_TICK_MS: u64 = 20;
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
static DROPPED_UI_TICKS: AtomicU32 = AtomicU32::new(0);
static DROPPED_READER_TICKS: AtomicU32 = AtomicU32::new(0);

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
    let mut store = Store::from_bootstrap(snapshot);
    let mut app = AppRuntime::new();

    info!("settings loaded={:?}", store.settings);
    let mut last_update = app.tick(&store);
    SCREEN_SIGNAL.signal(last_update);

    loop {
        let timed_event = APP_EVENT_CH.receive().await;
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

        let next_update = app.tick(&store);
        if next_update.screen != last_update.screen || next_update.prepared != last_update.prepared
        {
            SCREEN_SIGNAL.signal(next_update);
            last_update = next_update;
        }
    }
}

async fn apply_effect(store: &mut Store, effect: Effect, at_ms: u64) {
    match effect {
        Effect::EnterDeepSleep => {
            PLATFORM_CMD_CH
                .send(PlatformCommand::RequestDeepSleep)
                .await;
        }
        Effect::CollectionConfirmIgnored { collection, reason } => {
            info!(
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
                    store.open_cached_content(
                        request.collection,
                        request.content_id,
                        opened.title,
                        total_units,
                        opened.paragraphs,
                        opened.window,
                    );
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
            match content_storage::load_reader_window(
                request.content_id,
                request.window_start_unit_index,
            )
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
        Effect::PrepareContent(request) => {
            info!(
                "collection confirm prepare content collection={:?} content_id={} remote_item_id={}",
                request.collection,
                request.content_id.as_str(),
                request.remote_item_id.as_str(),
            );
            backend::request_prepare_content(request).await;
        }
        Effect::PersistSettings(settings) => {
            PLATFORM_CMD_CH
                .send(PlatformCommand::PersistSettings(settings))
                .await;
        }
        Effect::Noop => {}
    }
}

pub async fn run_minimal(spawner: Spawner) -> ! {
    let board = BoardConfig::new();
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    let boot_reset_reason = reset_reason(Cpu::ProCpu);
    let boot_wakeup_cause = wakeup_cause();
    let woke_from_deep_sleep = boot_reset_reason == Some(SocResetReason::CoreDeepSleep);
    info!(
        "boot reset_reason={:?} wakeup_cause={:?} wake={}",
        boot_reset_reason, boot_wakeup_cause, woke_from_deep_sleep
    );

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 64 * 1024);
    log_heap("after heap init");

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
        .with_frequency(Rate::from_hz(SD_SPI_HZ))
        .with_mode(esp_hal::spi::Mode::_0);
    let sd_spi = Spi::new(peripherals.SPI3, sd_spi_config)
        .unwrap()
        .with_sck(peripherals.GPIO4)
        .with_mosi(peripherals.GPIO40)
        .with_miso(peripherals.GPIO41);
    let sd_cs = Output::new(peripherals.GPIO8, Level::High, OutputConfig::default());
    let content_mount = content_storage::mount(sd_spi, sd_cs);
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

    let boot_ms = Instant::now().as_millis();
    let boot_state = if woke_from_deep_sleep {
        BootState::DeepSleepWake
    } else {
        BootState::ColdBoot
    };
    let snapshot = BootstrapSnapshot::new(
        DeviceState {
            pairing: backend::initial_pairing_state(backend_credential),
            boot: boot_state,
        },
        boot_ms,
        None,
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

    log_gpio_contract(&board);

    let mut frame = FrameBuffer::new();
    let mut committed_update: Option<ScreenUpdate> = None;
    let mut animation: Option<AnimationPlayback> = None;
    let mut maintenance_ticks = 0u8;

    let mut input_tick = Ticker::every(Duration::from_millis(INPUT_POLL_MS));
    let mut ui_tick = Ticker::every(Duration::from_millis(renderer::UI_TICK_MS));
    let mut reader_tick = Ticker::every(Duration::from_millis(READER_TICK_MS));

    loop {
        let suppress_sleep = current_prepared_screen(animation, committed_update)
            .is_some_and(|screen| prepared_screen_suppresses_sleep(&screen));
        let sleep_deadline = next_sleep_deadline(sleep.model(), suppress_sleep);

        match select5(
            input_tick.next(),
            select(ui_tick.next(), reader_tick.next()),
            Timer::at(sleep_deadline),
            PLATFORM_CMD_CH.receive(),
            SCREEN_SIGNAL.wait(),
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
                        publish_event(Event::UiTick(now_ms), now_ms);

                        if let Some(active_animation) = animation {
                            let next_frame = active_animation.advance();
                            flush_transition_frame(
                                &mut display,
                                &mut frame,
                                &mut delay,
                                &next_frame,
                            );
                            maintenance_ticks = 0;

                            if next_frame.is_complete() {
                                committed_update = Some(ScreenUpdate {
                                    screen: next_frame.screen,
                                    prepared: next_frame.target_screen(),
                                    transition: TransitionPlan::none(),
                                });
                                animation = None;
                            } else {
                                animation = Some(next_frame);
                            }
                        } else if let Some(update) = committed_update {
                            maintenance_ticks = maintenance_ticks.saturating_add(1);
                            if maintenance_ticks >= renderer::MAINTENANCE_REFRESH_TICKS {
                                flush_prepared_screen(
                                    &mut display,
                                    &mut frame,
                                    &mut delay,
                                    &update.prepared,
                                );
                                maintenance_ticks = 0;
                            }
                        }
                    }
                    Either::Second(_) => {
                        if current_prepared_screen(animation, committed_update)
                            .is_some_and(|screen| prepared_screen_drives_reader_ticks(&screen))
                        {
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
            Either5::Fifth(update) => {
                let previous_screen = animation
                    .map(|active| active.screen)
                    .or(committed_update.map(|committed| committed.screen));
                let previous_prepared = animation
                    .map(|active| active.target_screen())
                    .or(committed_update.map(|committed| committed.prepared));

                if previous_screen != Some(update.screen) {
                    info!("app screen={:?}", update.screen);
                }

                if update.screen == Screen::Reader && previous_screen != Some(Screen::Reader) {
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
                        committed_update = Some(update);
                        flush_prepared_screen(
                            &mut display,
                            &mut frame,
                            &mut delay,
                            &update.prepared,
                        );
                    } else {
                        let next_animation = AnimationPlayback::new(previous, update);
                        flush_transition_frame(
                            &mut display,
                            &mut frame,
                            &mut delay,
                            &next_animation,
                        );
                        maintenance_ticks = 0;

                        if next_animation.is_complete() {
                            committed_update = Some(ScreenUpdate {
                                screen: next_animation.screen,
                                prepared: next_animation.target_screen(),
                                transition: TransitionPlan::none(),
                            });
                            animation = None;
                        } else {
                            animation = Some(next_animation);
                        }
                    }
                } else {
                    committed_update = Some(update);
                    flush_prepared_screen(&mut display, &mut frame, &mut delay, &update.prepared);
                    maintenance_ticks = 0;
                }
            }
        }
    }
}

pub(crate) fn publish_event(event: Event, at_ms: u64) {
    let is_ui_tick = matches!(event, Event::UiTick(_));
    let is_reader_tick = matches!(event, Event::ReaderTick(_));

    match APP_EVENT_CH.try_send(TimedEvent { event, at_ms }) {
        Ok(()) => {
            if !is_ui_tick && !is_reader_tick {
                flush_tick_drop_logs();
            }
        }
        Err(embassy_sync::channel::TrySendError::Full(timed_event)) => {
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

pub(crate) async fn persist_backend_credential(credential: crate::storage::BackendCredential) {
    PLATFORM_CMD_CH
        .send(PlatformCommand::PersistBackendCredential(Box::new(
            credential,
        )))
        .await;
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
    prepared_screen_drives_reader_ticks(screen)
}

fn prepared_screen_drives_reader_ticks(screen: &PreparedScreen) -> bool {
    matches!(screen, PreparedScreen::Reader(shell) if shell.pause_modal.is_none())
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

fn flush_prepared_screen<SPI, DISP, EMD, CS, D>(
    display: &mut PlatformDisplay<SPI, DISP, EMD, CS>,
    frame: &mut FrameBuffer,
    delay: &mut D,
    screen: &PreparedScreen,
) where
    SPI: embedded_hal::spi::SpiBus<u8>,
    DISP: embedded_hal::digital::OutputPin,
    EMD: embedded_hal::digital::OutputPin,
    CS: embedded_hal::digital::OutputPin,
    D: DelayNs,
{
    renderer::draw_prepared_screen(frame, screen);
    if let Err(err) = display.flush_frame(frame, delay) {
        info!("display flush failed: {:?}", err);
        let _ = display.disable_output();
    }
}

fn flush_transition_frame<SPI, DISP, EMD, CS, D>(
    display: &mut PlatformDisplay<SPI, DISP, EMD, CS>,
    frame: &mut FrameBuffer,
    delay: &mut D,
    animation: &AnimationPlayback,
) where
    SPI: embedded_hal::spi::SpiBus<u8>,
    DISP: embedded_hal::digital::OutputPin,
    EMD: embedded_hal::digital::OutputPin,
    CS: embedded_hal::digital::OutputPin,
    D: DelayNs,
{
    renderer::draw_transition_frame(frame, animation);
    if let Err(err) = display.flush_frame(frame, delay) {
        info!("display flush failed: {:?}", err);
        let _ = display.disable_output();
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

fn log_gpio_contract(board: &BoardConfig) {
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
        "encoder gpio clk={} dt={} sw={}",
        board.encoder_clk_gpio, board.encoder_dt_gpio, board.encoder_sw_gpio
    );
    info!(
        "sleep wake gpio={} inactivity_ms=30000",
        board.sleep_wake_gpio
    );
}

fn log_heap(label: &str) {
    let stats = esp_alloc::HEAP.stats();
    info!(
        "heap label={} size={} used={} free={}",
        label,
        stats.size,
        stats.current_usage,
        stats.size.saturating_sub(stats.current_usage),
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

    #[test]
    fn requested_sleep_uses_immediate_deadline() {
        let mut model = SleepModel::new(SleepConfig::new(30_000));
        model.request_sleep();

        assert!(next_sleep_deadline(&model, false) <= Instant::now());
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
        let screen = PreparedScreen::Reader(app_runtime::components::ReaderShell {
            appearance: domain::settings::AppearanceMode::Light,
            stage: app_runtime::components::RsvpStage {
                title: "TEST",
                wpm: 260,
                left_word: domain::text::InlineText::new(),
                right_word: domain::text::InlineText::new(),
                preview: domain::text::InlineText::new(),
                font: domain::formatter::StageFont::Large,
                progress_width: 0,
            },
            badge: None,
            pause_modal: None,
        });

        assert!(prepared_screen_suppresses_sleep(&screen));
    }

    #[test]
    fn paused_reader_does_not_suppress_sleep() {
        let screen = PreparedScreen::Reader(app_runtime::components::ReaderShell {
            appearance: domain::settings::AppearanceMode::Light,
            stage: app_runtime::components::RsvpStage {
                title: "TEST",
                wpm: 260,
                left_word: domain::text::InlineText::new(),
                right_word: domain::text::InlineText::new(),
                preview: domain::text::InlineText::new(),
                font: domain::formatter::StageFont::Large,
                progress_width: 0,
            },
            badge: None,
            pause_modal: Some(app_runtime::components::PauseModal {
                title: "PAUSED",
                rows: [
                    app_runtime::components::PauseModalRow {
                        label: "A",
                        action: "A",
                    },
                    app_runtime::components::PauseModalRow {
                        label: "B",
                        action: "B",
                    },
                    app_runtime::components::PauseModalRow {
                        label: "C",
                        action: "C",
                    },
                ],
            }),
        });

        assert!(!prepared_screen_suppresses_sleep(&screen));
    }
}
