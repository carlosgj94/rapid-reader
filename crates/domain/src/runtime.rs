use crate::{
    device::DeviceState, input::InputGesture, settings::PersistedSettings, storage::StorageHealth,
};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum Command {
    #[default]
    Noop,
    Boot,
    RequestDeepSleep,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum Event {
    #[default]
    Noop,
    BootCompleted,
    InputGestureReceived(InputGesture),
    WokeFromDeepSleep,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum Effect {
    #[default]
    Noop,
    EnterDeepSleep,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct BootstrapSnapshot {
    pub device: DeviceState,
    pub boot_at_ms: u64,
    pub settings: Option<PersistedSettings>,
    pub storage: StorageHealth,
}

impl BootstrapSnapshot {
    pub const fn new(
        device: DeviceState,
        boot_at_ms: u64,
        settings: Option<PersistedSettings>,
        storage: StorageHealth,
    ) -> Self {
        Self {
            device,
            boot_at_ms,
            settings,
            storage,
        }
    }
}

impl Default for BootstrapSnapshot {
    fn default() -> Self {
        Self::new(DeviceState::new(), 0, None, StorageHealth::new())
    }
}
