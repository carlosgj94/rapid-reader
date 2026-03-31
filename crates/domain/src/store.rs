use crate::{
    device::{BootState, DeviceState},
    input::InputState,
    reader::ReaderSession,
    runtime::{BootstrapSnapshot, Command, Effect, Event},
    settings::SettingsState,
    sleep::{SleepModel, WakeReason},
    storage::StorageHealth,
};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum DispatchError {
    #[default]
    UnsupportedCommand,
}

pub type DispatchResult = Result<Effect, DispatchError>;

#[derive(Debug, Default)]
pub struct Store {
    pub device: DeviceState,
    pub input: InputState,
    pub reader: ReaderSession,
    pub settings: SettingsState,
    pub sleep: SleepModel,
    pub storage: StorageHealth,
}

impl Store {
    pub const fn new() -> Self {
        Self::from_bootstrap(BootstrapSnapshot::new(
            DeviceState::new(),
            0,
            None,
            StorageHealth::new(),
        ))
    }

    pub const fn from_bootstrap(snapshot: BootstrapSnapshot) -> Self {
        let settings = match snapshot.settings {
            Some(settings) => SettingsState::from_persisted(settings),
            None => SettingsState::new(crate::sleep::DEFAULT_INACTIVITY_TIMEOUT_MS),
        };
        let wake_reason = match snapshot.device.boot {
            BootState::DeepSleepWake => WakeReason::ExternalButton,
            BootState::ColdBoot => WakeReason::ColdBoot,
        };

        Self {
            device: snapshot.device,
            input: InputState::new(),
            reader: ReaderSession::new(),
            settings,
            sleep: SleepModel {
                config: crate::sleep::SleepConfig::new(settings.inactivity_timeout_ms),
                state: crate::sleep::SleepState::Awake,
                last_activity_ms: snapshot.boot_at_ms,
                last_wake_reason: wake_reason,
            },
            storage: snapshot.storage,
        }
    }

    pub fn dispatch(&mut self, command: Command) -> DispatchResult {
        match command {
            Command::RequestDeepSleep => {
                self.sleep.request_sleep();
                Ok(Effect::EnterDeepSleep)
            }
            _ => Ok(Effect::Noop),
        }
    }

    pub fn handle_event(&mut self, event: Event, now_ms: u64) -> DispatchResult {
        match event {
            Event::InputGestureReceived(gesture) => {
                self.input.record_gesture(gesture);
                self.sleep.note_activity(now_ms);
            }
            Event::WokeFromDeepSleep => {
                self.device.boot = BootState::DeepSleepWake;
                self.sleep.mark_woke(WakeReason::ExternalButton, now_ms);
            }
            Event::BootCompleted => {}
            Event::Noop => {}
        }

        Ok(Effect::Noop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        device::{BootState, DeviceState},
        settings::PersistedSettings,
        storage::{StorageHealth, StorageRecoveryStatus},
    };

    #[test]
    fn deep_sleep_bootstrap_hydrates_sleep_and_storage() {
        let snapshot = BootstrapSnapshot::new(
            DeviceState::with_boot(BootState::DeepSleepWake),
            42,
            Some(PersistedSettings::new(45_000)),
            StorageHealth::available(100, 200, StorageRecoveryStatus::Recovered),
        );

        let store = Store::from_bootstrap(snapshot);

        assert_eq!(store.device.boot, BootState::DeepSleepWake);
        assert_eq!(store.settings.inactivity_timeout_ms, 45_000);
        assert_eq!(store.sleep.config.inactivity_timeout_ms, 45_000);
        assert_eq!(store.sleep.last_activity_ms, 42);
        assert_eq!(store.sleep.last_wake_reason, WakeReason::ExternalButton);
        assert_eq!(store.storage.state_free_bytes, 100);
        assert_eq!(store.storage.outbox_free_bytes, 200);
    }

    #[test]
    fn missing_persisted_settings_fall_back_to_default_timeout() {
        let snapshot = BootstrapSnapshot::new(
            DeviceState::with_boot(BootState::ColdBoot),
            7,
            None,
            StorageHealth::new(),
        );

        let store = Store::from_bootstrap(snapshot);

        assert_eq!(
            store.settings.inactivity_timeout_ms,
            crate::sleep::DEFAULT_INACTIVITY_TIMEOUT_MS
        );
        assert_eq!(
            store.sleep.config.inactivity_timeout_ms,
            crate::sleep::DEFAULT_INACTIVITY_TIMEOUT_MS
        );
        assert_eq!(store.sleep.last_wake_reason, WakeReason::ColdBoot);
    }
}
