pub const PROVISIONING_PROTOCOL_VERSION: u16 = 1;
pub const WIFI_SSID_MAX_LEN: usize = 32;
pub const WIFI_PASSPHRASE_MAX_LEN: usize = 64;
pub const PAIRING_TOKEN_MAX_LEN: usize = 128;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum ProvisioningState {
    #[default]
    Disabled,
    Unprovisioned,
    Advertising,
    SessionAuthenticating,
    ScanningWifi,
    AwaitingBundle,
    ApplyingBundle,
    ConnectingWifi,
    ValidatingBackend,
    Provisioned,
    FailedRetryable,
    FailedTerminal,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum ProvisioningFailure {
    #[default]
    None,
    Cancelled,
    InvalidBundle,
    TransportUnavailable,
    ApplyFailed,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ProvisioningSession {
    pub active: bool,
    pub protocol_version: u16,
    pub state: ProvisioningState,
}

impl ProvisioningSession {
    pub const fn new(active: bool, state: ProvisioningState) -> Self {
        Self {
            active,
            protocol_version: PROVISIONING_PROTOCOL_VERSION,
            state,
        }
    }
}

impl Default for ProvisioningSession {
    fn default() -> Self {
        Self::new(false, ProvisioningState::Disabled)
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct WifiScanResult {
    pub ssid: [u8; WIFI_SSID_MAX_LEN],
    pub ssid_len: u8,
    pub rssi_dbm: i16,
    pub secure: bool,
}

impl WifiScanResult {
    pub const fn empty() -> Self {
        Self {
            ssid: [0; WIFI_SSID_MAX_LEN],
            ssid_len: 0,
            rssi_dbm: 0,
            secure: false,
        }
    }
}

impl Default for WifiScanResult {
    fn default() -> Self {
        Self::empty()
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ProvisioningBundle {
    pub wifi_ssid: [u8; WIFI_SSID_MAX_LEN],
    pub wifi_ssid_len: u8,
    pub wifi_passphrase: [u8; WIFI_PASSPHRASE_MAX_LEN],
    pub wifi_passphrase_len: u8,
    pub pairing_token: [u8; PAIRING_TOKEN_MAX_LEN],
    pub pairing_token_len: u8,
}

impl ProvisioningBundle {
    pub const fn empty() -> Self {
        Self {
            wifi_ssid: [0; WIFI_SSID_MAX_LEN],
            wifi_ssid_len: 0,
            wifi_passphrase: [0; WIFI_PASSPHRASE_MAX_LEN],
            wifi_passphrase_len: 0,
            pairing_token: [0; PAIRING_TOKEN_MAX_LEN],
            pairing_token_len: 0,
        }
    }
}

impl Default for ProvisioningBundle {
    fn default() -> Self {
        Self::empty()
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ProvisioningStatus {
    pub state: ProvisioningState,
    pub last_failure: ProvisioningFailure,
    pub discovered_networks: u8,
    pub claimed: bool,
}

impl ProvisioningStatus {
    pub const fn new(state: ProvisioningState) -> Self {
        Self {
            state,
            last_failure: ProvisioningFailure::None,
            discovered_networks: 0,
            claimed: false,
        }
    }
}

impl Default for ProvisioningStatus {
    fn default() -> Self {
        Self::new(ProvisioningState::Disabled)
    }
}
