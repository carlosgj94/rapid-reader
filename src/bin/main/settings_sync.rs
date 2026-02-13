use readily_core::settings::{PersistedSettings, SettingsStore};
use readily_hal_esp32s3::storage::flash_settings::FlashSettingsStore;

use super::SETTINGS_SAVE_DEBOUNCE_MS;

pub(super) struct SettingsSyncState {
    last_saved: PersistedSettings,
    pending: Option<(PersistedSettings, u64)>,
}

impl SettingsSyncState {
    pub(super) fn new(initial: PersistedSettings) -> Self {
        Self {
            last_saved: initial,
            pending: None,
        }
    }

    pub(super) fn track_current(&mut self, current: PersistedSettings, now_ms: u64) {
        if current == self.last_saved {
            return;
        }

        match self.pending.as_mut() {
            Some((pending, changed_at_ms)) => {
                if *pending != current {
                    *pending = current;
                    *changed_at_ms = now_ms;
                }
            }
            None => {
                self.pending = Some((current, now_ms));
            }
        }
    }

    pub(super) fn flush_if_due(&mut self, store: Option<&mut FlashSettingsStore>, now_ms: u64) {
        let Some((candidate, changed_at_ms)) = self.pending else {
            return;
        };

        if now_ms.saturating_sub(changed_at_ms) < SETTINGS_SAVE_DEBOUNCE_MS {
            return;
        }

        match store {
            Some(store) => {
                if store.save(&candidate).is_ok() {
                    self.last_saved = candidate;
                    self.pending = None;
                } else {
                    // Keep pending changes and retry later if flash is temporarily unavailable.
                    self.pending = Some((candidate, now_ms));
                }
            }
            None => {
                self.last_saved = candidate;
                self.pending = None;
            }
        }
    }
}
