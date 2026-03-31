#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum PairingState {
    #[default]
    Unpaired,
    Paired,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum BootState {
    #[default]
    ColdBoot,
    DeepSleepWake,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub struct DeviceState {
    pub pairing: PairingState,
    pub boot: BootState,
}

impl DeviceState {
    pub const fn new() -> Self {
        Self {
            pairing: PairingState::Unpaired,
            boot: BootState::ColdBoot,
        }
    }

    pub const fn with_boot(boot: BootState) -> Self {
        Self {
            pairing: PairingState::Unpaired,
            boot,
        }
    }
}
