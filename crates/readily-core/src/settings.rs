//! Persisted user settings abstraction.

use crate::render::VisualStyle;

/// Reading location used to resume after deep sleep.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResumeState {
    pub selected_book: u16,
    pub chapter_index: u16,
    pub paragraph_in_chapter: u16,
    pub word_index: u16,
}

/// UI context to restore after deep sleep.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SleepUiContext {
    ReadingPaused,
    Library {
        cursor: u16,
    },
    Settings {
        cursor: u8,
        editing: bool,
    },
    NavigateChapter {
        chapter_cursor: u16,
    },
    NavigateParagraph {
        chapter_index: u16,
        paragraph_in_chapter: u16,
    },
}

/// Snapshot persisted before sleep to restore app context on wake.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WakeSnapshot {
    pub ui_context: SleepUiContext,
    pub resume: ResumeState,
}

/// User-tunable settings that should survive reboot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PersistedSettings {
    pub wpm: u16,
    pub style: VisualStyle,
    pub wake_snapshot: Option<WakeSnapshot>,
}

impl PersistedSettings {
    pub const fn new(wpm: u16, style: VisualStyle) -> Self {
        Self {
            wpm,
            style,
            wake_snapshot: None,
        }
    }

    pub const fn with_wake_snapshot(mut self, wake_snapshot: Option<WakeSnapshot>) -> Self {
        self.wake_snapshot = wake_snapshot;
        self
    }
}

/// Abstract settings persistence backend.
pub trait SettingsStore {
    type Error;

    fn load(&mut self) -> Result<Option<PersistedSettings>, Self::Error>;
    fn save(&mut self, settings: &PersistedSettings) -> Result<(), Self::Error>;
}
