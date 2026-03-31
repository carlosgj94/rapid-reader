#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum WifiStatus {
    #[default]
    Disabled,
    Idle,
    Unavailable,
}

pub trait WifiService {
    fn status(&self) -> WifiStatus;
}

#[derive(Debug, Default)]
pub struct NoopWifiService;

impl WifiService for NoopWifiService {
    fn status(&self) -> WifiStatus {
        WifiStatus::Disabled
    }
}
