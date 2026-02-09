//! Persisted user settings abstraction.

use crate::render::VisualStyle;

/// User-tunable settings that should survive reboot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PersistedSettings {
    pub wpm: u16,
    pub style: VisualStyle,
}

impl PersistedSettings {
    pub const fn new(wpm: u16, style: VisualStyle) -> Self {
        Self { wpm, style }
    }
}

/// Abstract settings persistence backend.
pub trait SettingsStore {
    type Error;

    fn load(&mut self) -> Result<Option<PersistedSettings>, Self::Error>;
    fn save(&mut self, settings: &PersistedSettings) -> Result<(), Self::Error>;
}
