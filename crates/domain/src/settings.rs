use crate::sleep::DEFAULT_INACTIVITY_TIMEOUT_MS;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PersistedSettings {
    pub inactivity_timeout_ms: u64,
}

impl PersistedSettings {
    pub const fn new(inactivity_timeout_ms: u64) -> Self {
        Self {
            inactivity_timeout_ms,
        }
    }
}

impl Default for PersistedSettings {
    fn default() -> Self {
        Self::new(DEFAULT_INACTIVITY_TIMEOUT_MS)
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct SettingsState {
    pub inactivity_timeout_ms: u64,
}

impl SettingsState {
    pub const fn new(inactivity_timeout_ms: u64) -> Self {
        Self {
            inactivity_timeout_ms,
        }
    }

    pub const fn from_persisted(settings: PersistedSettings) -> Self {
        Self::new(settings.inactivity_timeout_ms)
    }

    pub const fn to_persisted(self) -> PersistedSettings {
        PersistedSettings::new(self.inactivity_timeout_ms)
    }
}

impl Default for SettingsState {
    fn default() -> Self {
        Self::new(DEFAULT_INACTIVITY_TIMEOUT_MS)
    }
}
