extern crate alloc;

use alloc::boxed::Box;
use core::net::{IpAddr, Ipv4Addr};
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use embassy_executor::Spawner;
use embassy_net::{Runner, Stack, StackResources, dns::DnsSocket, tcp::TcpSocket};
use embassy_time::{Duration, Instant, Timer};
use embedded_nal_async::{AddrType, Dns as _};
use esp_hal::{peripherals::WIFI, rng::Rng};
use esp_radio::wifi::{
    ClientConfig, Config as WifiDriverConfig, CountryInfo, ModeConfig, PowerSaveMode,
    WifiController, WifiDevice, WifiEvent, WifiStaState,
};
use log::{info, warn};

use domain::{
    network::{NetworkState, NetworkStatus},
    provisioning::{WIFI_PASSPHRASE_MAX_LEN, WIFI_SSID_MAX_LEN},
    runtime::Event,
};

use crate::{
    backend::{BACKEND_HOST, BACKEND_PORT},
    bootstrap::publish_event,
};

const STATUS_POLL_MS: u64 = 500;
const RECONNECT_BACKOFF_MS: u64 = 5_000;
const NETWORK_STACK_SOCKET_CAPACITY: usize = 4;
const WIFI_COUNTRY_CODE: [u8; 2] = *b"ES";
const WIFI_POWER_SAVE_MODE: PowerSaveMode = PowerSaveMode::None;

static PROBE_SUSPENDED: AtomicBool = AtomicBool::new(false);
static BACKEND_PATH_READY: AtomicBool = AtomicBool::new(false);
static WIFI_EVENT_LOGGING_INSTALLED: AtomicBool = AtomicBool::new(false);
static NETWORK_SESSION_EPOCH: AtomicU32 = AtomicU32::new(0);
static BACKEND_ENDPOINT_CACHE_VALID: AtomicBool = AtomicBool::new(false);
static BACKEND_ENDPOINT_CACHE_IP: AtomicU32 = AtomicU32::new(0);
static BACKEND_ENDPOINT_CACHE_SESSION_EPOCH: AtomicU32 = AtomicU32::new(0);
static BACKEND_ENDPOINT_CACHE_SET_AT_MS: AtomicU32 = AtomicU32::new(0);

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) struct CachedBackendEndpoint {
    pub addr: Ipv4Addr,
    pub session_epoch: u32,
    pub age_ms: u32,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct WifiCredentials {
    pub ssid: &'static str,
    pub passphrase: &'static str,
}

impl WifiCredentials {
    fn from_env() -> Option<Self> {
        let ssid = option_env!("MOTIF_WIFI_SSID").map(str::trim)?;
        let passphrase = option_env!("MOTIF_WIFI_PASS")
            .map(str::trim)
            .unwrap_or_default();

        if ssid.is_empty()
            || ssid.len() > WIFI_SSID_MAX_LEN
            || passphrase.len() > WIFI_PASSPHRASE_MAX_LEN
        {
            return None;
        }

        Some(Self { ssid, passphrase })
    }
}

pub fn initial_network_state() -> NetworkState {
    if WifiCredentials::from_env().is_some() {
        NetworkState::offline()
    } else {
        NetworkState::disabled()
    }
}

pub fn install(spawner: Spawner, wifi: WIFI<'static>) -> Option<Stack<'static>> {
    let Some(credentials) = WifiCredentials::from_env() else {
        info!("internet disabled: MOTIF_WIFI_SSID/MOTIF_WIFI_PASS not configured");
        return None;
    };

    let controller = match esp_radio::init() {
        Ok(controller) => Box::leak(Box::new(controller)),
        Err(err) => {
            warn!("internet init failed: {:?}", err);
            publish_status(NetworkStatus::Offline);
            return None;
        }
    };

    install_wifi_event_logging();

    let wifi_driver_config = WifiDriverConfig::default()
        .with_power_save_mode(WIFI_POWER_SAVE_MODE)
        .with_country_code(CountryInfo::from(WIFI_COUNTRY_CODE));
    info!(
        "internet wifi driver config power_save_mode={:?} country_code={}{} source=product_default config={:?}",
        WIFI_POWER_SAVE_MODE,
        WIFI_COUNTRY_CODE[0] as char,
        WIFI_COUNTRY_CODE[1] as char,
        wifi_driver_config
    );

    let (wifi_controller, interfaces) =
        match esp_radio::wifi::new(controller, wifi, wifi_driver_config) {
            Ok(parts) => parts,
            Err(err) => {
                warn!("internet wifi setup failed: {:?}", err);
                publish_status(NetworkStatus::Offline);
                return None;
            }
        };

    let seed = {
        let rng = Rng::new();
        (rng.random() as u64) << 32 | rng.random() as u64
    };
    // Embassy reserves internal sockets for DNS and DHCP. Leave extra room for the
    // startup probe plus backend TCP/TLS requests so boot-time connectivity checks
    // don't exhaust the shared socket set.
    let resources = Box::leak(Box::new(
        StackResources::<NETWORK_STACK_SOCKET_CAPACITY>::new(),
    ));
    let (stack, runner) = embassy_net::new(
        interfaces.sta,
        embassy_net::Config::dhcpv4(Default::default()),
        resources,
        seed,
    );

    if spawner
        .spawn(connection_task(wifi_controller, credentials))
        .is_err()
    {
        warn!("internet failed to spawn connection task");
        publish_status(NetworkStatus::Offline);
        return None;
    }

    if spawner.spawn(net_task(runner)).is_err() {
        warn!("internet failed to spawn network runner task");
        publish_status(NetworkStatus::Offline);
        return None;
    }

    if spawner.spawn(probe_task(stack)).is_err() {
        warn!("internet failed to spawn probe task");
        publish_status(NetworkStatus::Offline);
        return None;
    }

    publish_status(NetworkStatus::Connecting);
    Some(stack)
}

fn install_wifi_event_logging() {
    use esp_radio::wifi::event::{EventExt, StaConnected, StaDisconnected};

    if WIFI_EVENT_LOGGING_INSTALLED.swap(true, Ordering::Relaxed) {
        return;
    }

    StaConnected::update_handler(|event| {
        info!(
            "internet wifi event=StaConnected ssid_len={} channel={} authmode={} aid={}",
            event.ssid_len(),
            event.channel(),
            event.authmode(),
            event.aid()
        );
    });

    StaDisconnected::update_handler(|event| {
        info!(
            "internet wifi event=StaDisconnected ssid_len={} reason={} rssi={}",
            event.ssid_len(),
            event.reason(),
            event.rssi()
        );
    });
}

pub(crate) fn set_probe_suspended(suspended: bool) {
    PROBE_SUSPENDED.store(suspended, Ordering::Relaxed);
}

pub(crate) fn backend_path_ready() -> bool {
    BACKEND_PATH_READY.load(Ordering::Relaxed)
}

pub(crate) fn cached_backend_endpoint() -> Option<CachedBackendEndpoint> {
    if !BACKEND_ENDPOINT_CACHE_VALID.load(Ordering::Relaxed) {
        return None;
    }

    let current_epoch = NETWORK_SESSION_EPOCH.load(Ordering::Relaxed);
    if current_epoch == 0 {
        return None;
    }

    let cached_epoch = BACKEND_ENDPOINT_CACHE_SESSION_EPOCH.load(Ordering::Relaxed);
    if cached_epoch != current_epoch {
        return None;
    }

    let addr_bits = BACKEND_ENDPOINT_CACHE_IP.load(Ordering::Relaxed);
    let addr = Ipv4Addr::from(addr_bits.to_be_bytes());
    let stored_at_ms = BACKEND_ENDPOINT_CACHE_SET_AT_MS.load(Ordering::Relaxed);
    Some(CachedBackendEndpoint {
        addr,
        session_epoch: cached_epoch,
        age_ms: now_ms_u32().wrapping_sub(stored_at_ms),
    })
}

pub(crate) fn record_backend_endpoint(addr: Ipv4Addr, source: &'static str) {
    let session_epoch = NETWORK_SESSION_EPOCH.load(Ordering::Relaxed);
    if session_epoch == 0 {
        return;
    }

    let addr_bits = u32::from_be_bytes(addr.octets());
    let was_valid = BACKEND_ENDPOINT_CACHE_VALID.load(Ordering::Relaxed);
    let previous_addr = BACKEND_ENDPOINT_CACHE_IP.load(Ordering::Relaxed);
    let previous_epoch = BACKEND_ENDPOINT_CACHE_SESSION_EPOCH.load(Ordering::Relaxed);

    BACKEND_ENDPOINT_CACHE_IP.store(addr_bits, Ordering::Relaxed);
    BACKEND_ENDPOINT_CACHE_SESSION_EPOCH.store(session_epoch, Ordering::Relaxed);
    BACKEND_ENDPOINT_CACHE_SET_AT_MS.store(now_ms_u32(), Ordering::Relaxed);
    BACKEND_ENDPOINT_CACHE_VALID.store(true, Ordering::Relaxed);

    if !was_valid || previous_addr != addr_bits || previous_epoch != session_epoch {
        info!(
            "internet backend endpoint cached ip={} session_epoch={} source={}",
            addr, session_epoch, source
        );
    }
}

pub(crate) fn mark_backend_path_ready(source: &'static str) {
    if !BACKEND_PATH_READY.swap(true, Ordering::Relaxed) {
        info!("internet backend path ready source={}", source);
    }
}

pub(crate) fn invalidate_backend_path(reason: &'static str) {
    if BACKEND_PATH_READY.swap(false, Ordering::Relaxed) {
        info!("internet backend path invalidated reason={}", reason);
    }
}

fn clear_cached_backend_endpoint(reason: &'static str) {
    if BACKEND_ENDPOINT_CACHE_VALID.swap(false, Ordering::Relaxed) {
        let addr = Ipv4Addr::from(
            BACKEND_ENDPOINT_CACHE_IP
                .load(Ordering::Relaxed)
                .to_be_bytes(),
        );
        let session_epoch = BACKEND_ENDPOINT_CACHE_SESSION_EPOCH.load(Ordering::Relaxed);
        info!(
            "internet backend endpoint cache cleared ip={} session_epoch={} reason={}",
            addr, session_epoch, reason
        );
    }
}

fn advance_network_session_epoch(address: embassy_net::Ipv4Cidr, reason: &'static str) {
    clear_cached_backend_endpoint("network_session_changed");
    let epoch = NETWORK_SESSION_EPOCH
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1);
    invalidate_backend_path("network_session_changed");
    info!(
        "internet network session start epoch={} ip={:?} reason={}",
        epoch, address, reason
    );
}

#[embassy_executor::task]
async fn connection_task(mut controller: WifiController<'static>, credentials: WifiCredentials) {
    info!(
        "internet connection task starting ssid_len={} pass_len={}",
        credentials.ssid.len(),
        credentials.passphrase.len()
    );
    info!("internet wifi capabilities={:?}", controller.capabilities());

    loop {
        if matches!(esp_radio::wifi::sta_state(), WifiStaState::Connected) {
            controller.wait_for_event(WifiEvent::StaDisconnected).await;
            info!("internet wifi disconnected");
            clear_cached_backend_endpoint("wifi_disconnected");
            invalidate_backend_path("wifi_disconnected");
            publish_status(NetworkStatus::Offline);
            Timer::after(Duration::from_millis(RECONNECT_BACKOFF_MS)).await;
        }

        if !matches!(controller.is_started(), Ok(true)) {
            let client_config = ModeConfig::Client(
                ClientConfig::default()
                    .with_ssid(credentials.ssid.into())
                    .with_password(credentials.passphrase.into()),
            );

            if let Err(err) = controller.set_config(&client_config) {
                warn!("internet wifi config failed: {:?}", err);
                invalidate_backend_path("wifi_config_failed");
                publish_status(NetworkStatus::Offline);
                Timer::after(Duration::from_millis(RECONNECT_BACKOFF_MS)).await;
                continue;
            }

            info!("internet starting wifi");
            if let Err(err) = controller.start_async().await {
                warn!("internet wifi start failed: {:?}", err);
                invalidate_backend_path("wifi_start_failed");
                publish_status(NetworkStatus::Offline);
                Timer::after(Duration::from_millis(RECONNECT_BACKOFF_MS)).await;
                continue;
            }
        }

        publish_status(NetworkStatus::Connecting);
        info!("internet connecting to wifi");

        match controller.connect_async().await {
            Ok(_) => info!("internet wifi associated"),
            Err(err) => {
                warn!("internet wifi connect failed: {:?}", err);
                invalidate_backend_path("wifi_connect_failed");
                publish_status(NetworkStatus::Offline);
                Timer::after(Duration::from_millis(RECONNECT_BACKOFF_MS)).await;
            }
        }
    }
}

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, WifiDevice<'static>>) {
    runner.run().await;
}

#[embassy_executor::task]
async fn probe_task(stack: Stack<'static>) {
    let mut probe_ready = false;
    let mut session_address = None;

    loop {
        if !stack.is_link_up() {
            if session_address.take().is_some() {
                clear_cached_backend_endpoint("link_down");
            }
            probe_ready = false;
            invalidate_backend_path("link_down");
            Timer::after(Duration::from_millis(STATUS_POLL_MS)).await;
            continue;
        }

        let Some(config) = stack.config_v4() else {
            if session_address.take().is_some() {
                clear_cached_backend_endpoint("missing_ip");
            }
            invalidate_backend_path("missing_ip");
            Timer::after(Duration::from_millis(STATUS_POLL_MS)).await;
            continue;
        };

        if session_address != Some(config.address) {
            advance_network_session_epoch(config.address, "ip_acquired");
            session_address = Some(config.address);
            probe_ready = false;
        }

        if PROBE_SUSPENDED.load(Ordering::Relaxed) {
            Timer::after(Duration::from_millis(STATUS_POLL_MS)).await;
            continue;
        }

        if probe_ready && backend_path_ready() {
            Timer::after(Duration::from_millis(STATUS_POLL_MS)).await;
            continue;
        }

        info!("internet got ip {:?}", config.address);

        match perform_probe(stack).await {
            Ok(()) => {
                info!(
                    "internet probe succeeded host={} port={}",
                    BACKEND_HOST, BACKEND_PORT
                );
                mark_backend_path_ready("probe");
                publish_status(NetworkStatus::Online);
                probe_ready = true;
            }
            Err(err) => {
                warn!("internet probe failed: {:?}", err);
                invalidate_backend_path("probe_failed");
                publish_status(NetworkStatus::ProbeFailed);
                Timer::after(Duration::from_millis(STATUS_POLL_MS)).await;
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum ProbeError {
    Dns,
    Connect,
}

async fn perform_probe(stack: Stack<'static>) -> Result<(), ProbeError> {
    let mut rx_buffer = [0u8; 1024];
    let mut tx_buffer = [0u8; 512];
    let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);

    socket.set_timeout(Some(Duration::from_secs(10)));
    let remote = resolve_backend_ip_for_probe(stack).await?;
    socket
        .connect((remote, BACKEND_PORT))
        .await
        .map_err(|_| ProbeError::Connect)?;
    record_backend_endpoint(remote, "probe_connect_ok");
    socket.abort();
    Ok(())
}

async fn resolve_backend_ip_for_probe(stack: Stack<'static>) -> Result<Ipv4Addr, ProbeError> {
    let dns = DnsSocket::new(stack);
    match dns.get_host_by_name(BACKEND_HOST, AddrType::IPv4).await {
        Ok(IpAddr::V4(addr)) => Ok(addr),
        Ok(IpAddr::V6(_)) => cached_backend_endpoint().map(|cached| {
            info!(
                "internet probe dns fallback host={} cached_ip={} cache_age_ms={} session_epoch={} reason=invalid_family",
                BACKEND_HOST, cached.addr, cached.age_ms, cached.session_epoch
            );
            cached.addr
        }).ok_or_else(|| {
            info!(
                "internet probe dns fallback miss host={} reason=invalid_family",
                BACKEND_HOST
            );
            ProbeError::Dns
        }),
        Err(_) => cached_backend_endpoint().map(|cached| {
            info!(
                "internet probe dns fallback host={} cached_ip={} cache_age_ms={} session_epoch={} reason=dns_failed",
                BACKEND_HOST, cached.addr, cached.age_ms, cached.session_epoch
            );
            cached.addr
        }).ok_or_else(|| {
            info!(
                "internet probe dns fallback miss host={} reason=dns_failed",
                BACKEND_HOST
            );
            ProbeError::Dns
        }),
    }
}

fn publish_status(status: NetworkStatus) {
    publish_event(
        Event::NetworkStatusChanged(status),
        Instant::now().as_millis(),
    );
}

fn now_ms_u32() -> u32 {
    Instant::now().as_millis() as u32
}
