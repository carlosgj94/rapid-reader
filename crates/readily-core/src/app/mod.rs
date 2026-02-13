//! Application state machine for library, settings, and RSVP reading.

use core::str;

use log::debug;

use crate::{
    content::{
        NavigationCatalog, ParagraphNavigator, SelectableWordSource, TextCatalog, WordSource,
    },
    input::{InputEvent, InputProvider},
    render::{
        AnimationKind, AnimationSpec, FontFamily, FontSize, MenuItemKind, MenuItemView, Screen,
        SettingRowView, SettingValue, VisualStyle,
    },
    settings::PersistedSettings,
    text_policy::{
        chapter_number_label, preview_compact, preview_limited, section_secondary_label,
    },
};

const WPM_STEP: u16 = 10;
const WORD_BUFFER_BYTES: usize = 96;
const MAX_LIBRARY_ITEMS: usize = 12;
const EXIT_DOUBLE_PRESS_MS: u64 = 450;

const ANIM_MENU_MS: u16 = 180;
const ANIM_SCREEN_MS: u16 = 220;
const ANIM_NAV_MS: u16 = 160;
const ANIM_NAV_ROTATE_MS: u16 = 120;
const NAV_LABEL_BYTES: usize = 52;
const NAV_PREVIEW_BYTES: usize = 240;
const PAUSE_ANIM_FRAME_MS: u64 = 180;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TickResult {
    NoRender,
    RenderRequested,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReaderConfig {
    pub wpm: u16,
    pub min_wpm: u16,
    pub max_wpm: u16,
    pub dot_pause_ms: u16,
    pub comma_pause_ms: u16,
}

impl Default for ReaderConfig {
    fn default() -> Self {
        Self {
            wpm: 230,
            min_wpm: 80,
            max_wpm: 600,
            dot_pause_ms: 240,
            comma_pause_ms: 240,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UiState {
    Library {
        cursor: u16,
    },
    Settings {
        cursor: u8,
        editing: bool,
    },
    Countdown {
        selected_book: u16,
        remaining: u8,
        next_step_ms: u64,
    },
    Reading {
        selected_book: u16,
        paused: bool,
        next_word_ms: u64,
    },
    NavigateChapter {
        selected_book: u16,
        chapter_cursor: u16,
    },
    NavigateChapterLoading {
        selected_book: u16,
        chapter_index: u16,
    },
    NavigateParagraph {
        selected_book: u16,
        chapter_index: u16,
        paragraph_cursor: u16,
    },
    Status {
        line1: &'static str,
        line2: &'static str,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SettingsRow {
    Font,
    Size,
    Invert,
    Wpm,
    Back,
}

impl SettingsRow {
    const COUNT: u8 = 5;

    fn from_index(index: u8) -> Self {
        match index {
            0 => Self::Font,
            1 => Self::Size,
            2 => Self::Invert,
            3 => Self::Wpm,
            _ => Self::Back,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AdvanceWordResult {
    Advanced,
    AwaitingRefill,
    EndOfText,
}

pub struct ReaderApp<WS, IN>
where
    WS: WordSource + TextCatalog + SelectableWordSource + ParagraphNavigator + NavigationCatalog,
    IN: InputProvider,
{
    content: WS,
    input: IN,
    config: ReaderConfig,
    app_title: &'static str,
    countdown_seconds: u8,
    style: VisualStyle,
    ui: UiState,
    pending_redraw: bool,
    transition: Option<AnimationSpec>,
    word_buffer: WordBuffer<WORD_BUFFER_BYTES>,
    paragraph_word_index: u16,
    paragraph_word_total: u16,
    last_ends_sentence: bool,
    last_ends_clause: bool,
    words_since_drain: u32,
    last_reading_press_ms: Option<u64>,
    paused_since_ms: Option<u64>,
    last_pause_anim_slot: Option<u32>,
}

include!("view.rs");
include!("input.rs");
include!("runtime.rs");
include!("navigation.rs");
include!("word_buffer.rs");
