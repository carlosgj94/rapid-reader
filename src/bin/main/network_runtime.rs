use core::net::Ipv4Addr;

use embassy_net::{
    Stack,
    icmp::{PacketMetadata, ping::PingManager, ping::PingParams},
};
use embassy_time::{Duration as EmbassyDuration, Timer, WithTimeout};
use esp_radio::wifi::WifiController;
use log::info;
use readily_hal_esp32s3::network::ConnectivityHandle;

const WIFI_RETRY_BACKOFF_MIN_SECS: u64 = 2;
const WIFI_RETRY_BACKOFF_MAX_SECS: u64 = 120;
const NETWORK_POLL_INTERVAL_MS: u64 = 500;
const PING_INTERVAL_SECS: u64 = 5;
const PING_IDLE_INTERVAL_SECS: u64 = 20;
const PING_TIMEOUT_MS: u64 = 1_200;
const DHCP_TIMEOUT_SECS: u64 = 15;

pub(super) const PING_TARGET: Ipv4Addr = Ipv4Addr::new(1, 1, 1, 1);

fn wifi_retry_backoff_secs(consecutive_failures: u32) -> u64 {
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

#[allow(
    clippy::large_stack_frames,
    reason = "Long-lived embedded async tasks hold their connection state inside one future state."
)]
pub(super) async fn wifi_connection_loop(
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

#[allow(
    clippy::large_stack_frames,
    reason = "Long-lived embedded async tasks hold their network buffers inside one future state."
)]
pub(super) async fn ping_loop(stack: Stack<'_>, connectivity: &'static ConnectivityHandle) -> ! {
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
