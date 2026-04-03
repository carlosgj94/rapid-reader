use domain::network::NetworkStatus;

pub trait WifiService {
    fn status(&self) -> NetworkStatus;
    fn start(&mut self) -> NetworkStatus;
}

#[derive(Debug, Default)]
pub struct NoopWifiService;

impl WifiService for NoopWifiService {
    fn status(&self) -> NetworkStatus {
        NetworkStatus::Disabled
    }

    fn start(&mut self) -> NetworkStatus {
        NetworkStatus::Disabled
    }
}
