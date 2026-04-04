extern crate alloc;

use alloc::boxed::Box;
use core::net::IpAddr;
use core::sync::atomic::{AtomicBool, Ordering};

use embassy_executor::Spawner;
use embassy_net::{Runner, Stack, StackResources, dns::DnsSocket, tcp::TcpSocket};
use embassy_time::{Duration, Instant, Timer};
use embedded_nal_async::{AddrType, Dns as _};
use esp_hal::{peripherals::WIFI, rng::Rng};
use esp_radio::wifi::{
    ClientConfig, ModeConfig, WifiController, WifiDevice, WifiEvent, WifiStaState,
};
use log::info;

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

static PROBE_SUSPENDED: AtomicBool = AtomicBool::new(false);

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
            info!("internet init failed: {:?}", err);
            publish_status(NetworkStatus::Offline);
            return None;
        }
    };

    let (wifi_controller, interfaces) =
        match esp_radio::wifi::new(controller, wifi, Default::default()) {
            Ok(parts) => parts,
            Err(err) => {
                info!("internet wifi setup failed: {:?}", err);
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
        info!("internet failed to spawn connection task");
        publish_status(NetworkStatus::Offline);
        return None;
    }

    if spawner.spawn(net_task(runner)).is_err() {
        info!("internet failed to spawn network runner task");
        publish_status(NetworkStatus::Offline);
        return None;
    }

    if spawner.spawn(probe_task(stack)).is_err() {
        info!("internet failed to spawn probe task");
        publish_status(NetworkStatus::Offline);
        return None;
    }

    publish_status(NetworkStatus::Connecting);
    Some(stack)
}

pub(crate) fn set_probe_suspended(suspended: bool) {
    PROBE_SUSPENDED.store(suspended, Ordering::Relaxed);
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
                info!("internet wifi config failed: {:?}", err);
                publish_status(NetworkStatus::Offline);
                Timer::after(Duration::from_millis(RECONNECT_BACKOFF_MS)).await;
                continue;
            }

            info!("internet starting wifi");
            if let Err(err) = controller.start_async().await {
                info!("internet wifi start failed: {:?}", err);
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
                info!("internet wifi connect failed: {:?}", err);
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

    loop {
        if !stack.is_link_up() {
            probe_ready = false;
            Timer::after(Duration::from_millis(STATUS_POLL_MS)).await;
            continue;
        }

        let Some(config) = stack.config_v4() else {
            Timer::after(Duration::from_millis(STATUS_POLL_MS)).await;
            continue;
        };

        if PROBE_SUSPENDED.load(Ordering::Relaxed) {
            Timer::after(Duration::from_millis(STATUS_POLL_MS)).await;
            continue;
        }

        if probe_ready {
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
                publish_status(NetworkStatus::Online);
                probe_ready = true;
            }
            Err(err) => {
                info!("internet probe failed: {:?}", err);
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
    let dns = DnsSocket::new(stack);

    socket.set_timeout(Some(Duration::from_secs(10)));
    let remote = dns
        .get_host_by_name(BACKEND_HOST, AddrType::IPv4)
        .await
        .map_err(|_| ProbeError::Dns)?;
    let remote = match remote {
        IpAddr::V4(addr) => addr,
        IpAddr::V6(_) => return Err(ProbeError::Dns),
    };
    socket
        .connect((remote, BACKEND_PORT))
        .await
        .map_err(|_| ProbeError::Connect)?;
    socket.abort();
    Ok(())
}

fn publish_status(status: NetworkStatus) {
    publish_event(
        Event::NetworkStatusChanged(status),
        Instant::now().as_millis(),
    );
}
