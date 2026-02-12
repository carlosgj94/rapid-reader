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

impl<WS, IN> ReaderApp<WS, IN>
where
    WS: WordSource + TextCatalog + SelectableWordSource + ParagraphNavigator + NavigationCatalog,
    IN: InputProvider,
{
    pub fn new(
        mut content: WS,
        input: IN,
        mut config: ReaderConfig,
        app_title: &'static str,
        countdown_seconds: u8,
    ) -> Self {
        if config.max_wpm < config.min_wpm {
            core::mem::swap(&mut config.max_wpm, &mut config.min_wpm);
        }
        config.wpm = config.wpm.clamp(config.min_wpm, config.max_wpm);

        let initial_index = content
            .selected_index()
            .min(content.title_count().saturating_sub(1));
        let _ = content.select_text(initial_index);

        Self {
            content,
            input,
            config,
            app_title,
            countdown_seconds: countdown_seconds.max(1),
            style: VisualStyle::default(),
            ui: UiState::Library {
                cursor: initial_index,
            },
            pending_redraw: true,
            transition: None,
            word_buffer: WordBuffer::new(),
            paragraph_word_index: 0,
            paragraph_word_total: 1,
            last_ends_sentence: false,
            last_ends_clause: false,
            words_since_drain: 0,
            last_reading_press_ms: None,
            paused_since_ms: None,
            last_pause_anim_slot: None,
        }
    }

    pub fn tick(&mut self, now_ms: u64) -> TickResult {
        self.process_inputs(now_ms);

        let rendered = match self.ui {
            UiState::Countdown { .. } => self.tick_countdown(now_ms),
            UiState::Reading { .. } => self.tick_reading(now_ms),
            UiState::Library { .. }
            | UiState::Settings { .. }
            | UiState::NavigateChapter { .. }
            | UiState::NavigateParagraph { .. }
            | UiState::Status { .. } => {
                if self.pending_redraw {
                    self.pending_redraw = false;
                    TickResult::RenderRequested
                } else {
                    TickResult::NoRender
                }
            }
        };

        if self.transition_frame(now_ms).is_some() {
            TickResult::RenderRequested
        } else {
            rendered
        }
    }

    pub fn with_screen<F>(&self, now_ms: u64, f: F)
    where
        F: FnOnce(Screen<'_>),
    {
        let animation = self.transition_frame(now_ms);

        match self.ui {
            UiState::Library { cursor } => {
                let mut items = [MenuItemView::default(); MAX_LIBRARY_ITEMS];
                let mut count = 0usize;

                let total_titles = self.total_title_count() as usize;
                let visible_slots = MAX_LIBRARY_ITEMS.saturating_sub(1);
                let selected_cursor = cursor as usize;
                let selected_book = selected_cursor.min(total_titles.saturating_sub(1));
                let window_start = if total_titles <= visible_slots {
                    0
                } else {
                    selected_book
                        .saturating_sub(visible_slots / 2)
                        .min(total_titles.saturating_sub(visible_slots))
                };
                let window_end = core::cmp::min(total_titles, window_start + visible_slots);

                for idx in window_start..window_end {
                    let label = self.content.title_at(idx as u16).unwrap_or("Untitled");
                    items[count] = MenuItemView {
                        label,
                        kind: MenuItemKind::Text,
                    };
                    count += 1;
                }

                items[count] = MenuItemView {
                    label: "Settings",
                    kind: MenuItemKind::Settings,
                };
                count += 1;

                let settings_cursor = self.settings_item_index();
                let cursor = if cursor == settings_cursor {
                    count.saturating_sub(1)
                } else {
                    selected_book
                        .saturating_sub(window_start)
                        .min(count.saturating_sub(1))
                };
                f(Screen::Library {
                    title: self.app_title,
                    subtitle: "Library",
                    items: &items[..count],
                    cursor,
                    style: self.style,
                    animation,
                });
            }
            UiState::Settings { cursor, editing } => {
                let rows = [
                    SettingRowView {
                        key: "Font",
                        value: SettingValue::Label(font_family_label(self.style.font_family)),
                    },
                    SettingRowView {
                        key: "Size",
                        value: SettingValue::Label(font_size_label(self.style.font_size)),
                    },
                    SettingRowView {
                        key: "Invert",
                        value: SettingValue::Toggle(self.style.inverted),
                    },
                    SettingRowView {
                        key: "WPM",
                        value: SettingValue::Number(self.config.wpm),
                    },
                    SettingRowView {
                        key: "Back",
                        value: SettingValue::Action("Return"),
                    },
                ];

                f(Screen::Settings {
                    title: self.app_title,
                    subtitle: "Settings",
                    rows: &rows,
                    cursor: (cursor as usize).min(rows.len() - 1),
                    editing,
                    style: self.style,
                    animation,
                });
            }
            UiState::Countdown {
                selected_book,
                remaining,
                ..
            } => {
                let title = self.content.title_at(selected_book).unwrap_or("Untitled");
                f(Screen::Countdown {
                    title,
                    cover_slot: selected_book,
                    has_cover: self.content.has_cover_at(selected_book),
                    wpm: self.config.wpm,
                    remaining,
                    style: self.style,
                    animation,
                });
            }
            UiState::Reading {
                selected_book,
                paused,
                ..
            } => {
                let book_title = self.content.title_at(selected_book).unwrap_or("Untitled");
                let paused_elapsed_ms = if paused {
                    now_ms.saturating_sub(self.paused_since_ms.unwrap_or(now_ms)) as u32
                } else {
                    0
                };
                let current_paragraph = self.content.paragraph_index().saturating_sub(1);
                let current_chapter = self.current_chapter_index();
                let chapter_label_raw = self
                    .content
                    .chapter_at(current_chapter)
                    .and_then(|chapter| {
                        if chapter.label.trim().is_empty() {
                            self.content.paragraph_preview(chapter.start_paragraph)
                        } else {
                            Some(chapter.label)
                        }
                    })
                    .or_else(|| self.content.paragraph_preview(current_paragraph))
                    .unwrap_or("Section");
                let mut header_title_buf = [0u8; NAV_LABEL_BYTES];
                let mut pause_chapter_label_buf = [0u8; NAV_LABEL_BYTES];
                let title = preview_limited(chapter_label_raw, &mut header_title_buf, 4, 36);
                let pause_chapter_label = preview_compact(book_title, &mut pause_chapter_label_buf);
                f(Screen::Reading {
                    title,
                    wpm: self.config.wpm,
                    word: self.word_buffer.as_str(),
                    paragraph_word_index: self.paragraph_word_index,
                    paragraph_word_total: self.paragraph_word_total,
                    paused,
                    paused_elapsed_ms,
                    pause_chapter_label,
                    style: self.style,
                    animation,
                });
            }
            UiState::NavigateChapter {
                selected_book,
                chapter_cursor,
            } => {
                let title = self.content.title_at(selected_book).unwrap_or("Untitled");
                let chapter_total = self.content.chapter_count().max(1);
                let chapter_cursor = chapter_cursor.min(chapter_total.saturating_sub(1));
                let current_chapter = self.current_chapter_index();

                let current_label_raw = self
                    .content
                    .chapter_at(current_chapter)
                    .map(|c| c.label)
                    .unwrap_or("Current");
                let target_label_raw = self
                    .content
                    .chapter_at(chapter_cursor)
                    .map(|c| c.label)
                    .unwrap_or("Target");

                let mut current_label_buf = [0u8; NAV_LABEL_BYTES];
                let mut target_label_buf = [0u8; NAV_LABEL_BYTES];
                let mut current_secondary_buf = [0u8; NAV_LABEL_BYTES];
                let mut target_secondary_buf = [0u8; NAV_LABEL_BYTES];
                let current_label = preview_compact(current_label_raw, &mut current_label_buf);
                let target_label = preview_compact(target_label_raw, &mut target_label_buf);
                let current_secondary = section_secondary_label(
                    current_chapter.saturating_add(1),
                    chapter_total,
                    "Current",
                    &mut current_secondary_buf,
                );
                let target_secondary = section_secondary_label(
                    chapter_cursor.saturating_add(1),
                    chapter_total,
                    "Target  Press for paragraphs",
                    &mut target_secondary_buf,
                );

                f(Screen::NavigateChapters {
                    title,
                    wpm: self.config.wpm,
                    current_chapter: current_chapter.saturating_add(1),
                    target_chapter: chapter_cursor.saturating_add(1),
                    chapter_total,
                    current_label,
                    target_label,
                    current_secondary,
                    target_secondary,
                    style: self.style,
                    animation,
                });
            }
            UiState::NavigateParagraph {
                selected_book,
                chapter_index,
                paragraph_cursor,
            } => {
                let title = self.content.title_at(selected_book).unwrap_or("Untitled");
                let chapter = self.content.chapter_at(chapter_index);

                let (chapter_label_raw, chapter_start, chapter_count) = match chapter {
                    Some(info) => (
                        info.label,
                        info.start_paragraph,
                        info.paragraph_count.max(1),
                    ),
                    None => ("Chapter", 0, 1),
                };

                let max_cursor = chapter_start.saturating_add(chapter_count.saturating_sub(1));
                let paragraph_cursor = paragraph_cursor.clamp(chapter_start, max_cursor);
                let current_paragraph = self.content.paragraph_index().saturating_sub(1);

                let target_preview_raw = self
                    .content
                    .paragraph_preview(paragraph_cursor)
                    .unwrap_or("Target paragraph");

                let mut chapter_label_buf = [0u8; NAV_LABEL_BYTES];
                let mut chapter_number_buf = [0u8; 10];
                let mut target_preview_buf = [0u8; NAV_PREVIEW_BYTES];
                let chapter_label = preview_compact(chapter_label_raw, &mut chapter_label_buf);
                let current_preview =
                    chapter_number_label(chapter_index.saturating_add(1), &mut chapter_number_buf);
                let target_preview =
                    preview_limited(target_preview_raw, &mut target_preview_buf, 48, 220);

                let current_index_in_chapter =
                    if current_paragraph >= chapter_start && current_paragraph <= max_cursor {
                        current_paragraph
                            .saturating_sub(chapter_start)
                            .saturating_add(1)
                    } else {
                        1
                    };

                let target_index_in_chapter = paragraph_cursor
                    .saturating_sub(chapter_start)
                    .saturating_add(1);

                let mut current_secondary_buf = [0u8; NAV_LABEL_BYTES];
                let mut target_secondary_buf = [0u8; NAV_LABEL_BYTES];
                let current_secondary = section_secondary_label(
                    current_index_in_chapter,
                    chapter_count,
                    "Current",
                    &mut current_secondary_buf,
                );
                let target_secondary = section_secondary_label(
                    target_index_in_chapter,
                    chapter_count,
                    "Target  Press to jump",
                    &mut target_secondary_buf,
                );

                f(Screen::NavigateParagraphs {
                    title,
                    wpm: self.config.wpm,
                    chapter_label,
                    current_preview,
                    target_preview,
                    current_secondary,
                    target_secondary,
                    target_index_in_chapter,
                    paragraph_total_in_chapter: chapter_count,
                    style: self.style,
                    animation,
                });
            }
            UiState::Status { line1, line2 } => {
                f(Screen::Status {
                    title: self.app_title,
                    wpm: self.config.wpm,
                    line1,
                    line2,
                    style: self.style,
                    animation,
                });
            }
        }
    }

    pub fn with_content_mut<R, F>(&mut self, f: F) -> R
    where
        F: FnOnce(&mut WS) -> R,
    {
        f(&mut self.content)
    }

    pub fn drain_word_updates(&mut self) -> u32 {
        let count = self.words_since_drain;
        self.words_since_drain = 0;
        count
    }

    pub fn persisted_settings(&self) -> PersistedSettings {
        PersistedSettings::new(self.config.wpm, self.style)
    }

    pub fn apply_persisted_settings(&mut self, settings: PersistedSettings) {
        self.style = settings.style;
        self.config.wpm = settings.wpm.clamp(self.config.min_wpm, self.config.max_wpm);
        self.pending_redraw = true;
    }

    fn process_inputs(&mut self, now_ms: u64) {
        loop {
            match self.input.poll_event() {
                Ok(Some(event)) => self.apply_input_event(event, now_ms),
                Ok(None) => break,
                Err(_) => {
                    self.set_status("INPUT ERROR", "CHECK PROVIDER", now_ms);
                    break;
                }
            }
        }
    }

    fn apply_input_event(&mut self, event: InputEvent, now_ms: u64) {
        match self.ui {
            UiState::Library { cursor } => self.apply_library_input(cursor, event, now_ms),
            UiState::Settings { cursor, editing } => {
                self.apply_settings_input(cursor, editing, event, now_ms)
            }
            UiState::Countdown {
                selected_book,
                remaining,
                next_step_ms,
            } => self.apply_countdown_input(selected_book, remaining, next_step_ms, event, now_ms),
            UiState::Reading {
                selected_book,
                paused,
                next_word_ms,
            } => self.apply_reading_input(selected_book, paused, next_word_ms, event, now_ms),
            UiState::NavigateChapter {
                selected_book,
                chapter_cursor,
            } => self.apply_chapter_navigation_input(selected_book, chapter_cursor, event, now_ms),
            UiState::NavigateParagraph {
                selected_book,
                chapter_index,
                paragraph_cursor,
            } => self.apply_paragraph_navigation_input(
                selected_book,
                chapter_index,
                paragraph_cursor,
                event,
                now_ms,
            ),
            UiState::Status { .. } => {
                if matches!(event, InputEvent::Press) {
                    self.enter_library(self.content.selected_index(), now_ms);
                }
            }
        }
    }

    fn apply_library_input(&mut self, cursor: u16, event: InputEvent, now_ms: u64) {
        let total_items = self.library_item_count().max(1);

        match event {
            InputEvent::RotateCw => {
                self.ui = UiState::Library {
                    cursor: rotate_cw(cursor, total_items),
                };
                self.start_transition(AnimationKind::SlideLeft, now_ms, 120);
                self.pending_redraw = true;
            }
            InputEvent::RotateCcw => {
                self.ui = UiState::Library {
                    cursor: rotate_ccw(cursor, total_items),
                };
                self.start_transition(AnimationKind::SlideRight, now_ms, 120);
                self.pending_redraw = true;
            }
            InputEvent::Press => {
                let settings_index = self.settings_item_index();
                if cursor == settings_index {
                    self.enter_settings(0, false, now_ms);
                    return;
                }

                if self.content.select_text(cursor).is_err() {
                    self.set_status("CONTENT ERROR", "INVALID TITLE", now_ms);
                    return;
                }

                self.enter_countdown(cursor, now_ms);
            }
        }
    }

    fn apply_settings_input(&mut self, cursor: u8, editing: bool, event: InputEvent, now_ms: u64) {
        if editing {
            match event {
                InputEvent::Press => self.enter_settings(cursor, false, now_ms),
                InputEvent::RotateCw => {
                    self.rotate_setting(SettingsRow::from_index(cursor), true);
                    self.pending_redraw = true;
                }
                InputEvent::RotateCcw => {
                    self.rotate_setting(SettingsRow::from_index(cursor), false);
                    self.pending_redraw = true;
                }
            }
            return;
        }

        match event {
            InputEvent::RotateCw => {
                let next = rotate_cw(cursor as u16, SettingsRow::COUNT as u16) as u8;
                self.enter_settings(next, false, now_ms);
            }
            InputEvent::RotateCcw => {
                let next = rotate_ccw(cursor as u16, SettingsRow::COUNT as u16) as u8;
                self.enter_settings(next, false, now_ms);
            }
            InputEvent::Press => {
                let row = SettingsRow::from_index(cursor);
                if matches!(row, SettingsRow::Back) {
                    self.enter_library(self.settings_item_index(), now_ms);
                } else {
                    self.enter_settings(cursor, true, now_ms);
                }
            }
        }
    }

    fn apply_countdown_input(
        &mut self,
        selected_book: u16,
        remaining: u8,
        next_step_ms: u64,
        event: InputEvent,
        now_ms: u64,
    ) {
        match event {
            InputEvent::Press => self.enter_reading(selected_book, now_ms),
            InputEvent::RotateCw => {
                if self.adjust_wpm(true) {
                    self.ui = UiState::Countdown {
                        selected_book,
                        remaining,
                        next_step_ms,
                    };
                    self.pending_redraw = true;
                }
            }
            InputEvent::RotateCcw => {
                if self.adjust_wpm(false) {
                    self.ui = UiState::Countdown {
                        selected_book,
                        remaining,
                        next_step_ms,
                    };
                    self.pending_redraw = true;
                }
            }
        }
    }

    fn apply_reading_input(
        &mut self,
        selected_book: u16,
        paused: bool,
        next_word_ms: u64,
        event: InputEvent,
        now_ms: u64,
    ) {
        match event {
            InputEvent::Press => {
                let double_press = self
                    .last_reading_press_ms
                    .is_some_and(|last| now_ms.saturating_sub(last) <= EXIT_DOUBLE_PRESS_MS);
                self.last_reading_press_ms = Some(now_ms);

                if double_press {
                    self.last_reading_press_ms = None;
                    self.enter_library(selected_book, now_ms);
                    return;
                }

                self.ui = UiState::Reading {
                    selected_book,
                    paused: !paused,
                    next_word_ms,
                };
                if paused {
                    self.paused_since_ms = None;
                } else {
                    self.paused_since_ms = Some(now_ms);
                }
                self.last_pause_anim_slot = None;
                self.pending_redraw = true;
            }
            InputEvent::RotateCw => {
                if paused {
                    let chapter_total = self.content.chapter_count().max(1);
                    let current_chapter = self.current_chapter_index();
                    let next_chapter = rotate_cw(current_chapter, chapter_total);
                    debug!(
                        "ui-nav: paused rotate_cw selected_book={} current_chapter={}/{} next_chapter={}/{}",
                        selected_book,
                        current_chapter.saturating_add(1),
                        chapter_total,
                        next_chapter.saturating_add(1),
                        chapter_total
                    );
                    self.enter_chapter_navigation(selected_book, next_chapter, now_ms);
                    return;
                }

                if self.adjust_wpm(true) {
                    self.ui = UiState::Reading {
                        selected_book,
                        paused,
                        next_word_ms: if paused { next_word_ms } else { now_ms },
                    };
                    self.pending_redraw = true;
                }
            }
            InputEvent::RotateCcw => {
                if paused {
                    let chapter_total = self.content.chapter_count().max(1);
                    let current_chapter = self.current_chapter_index();
                    let next_chapter = rotate_ccw(current_chapter, chapter_total);
                    debug!(
                        "ui-nav: paused rotate_ccw selected_book={} current_chapter={}/{} next_chapter={}/{}",
                        selected_book,
                        current_chapter.saturating_add(1),
                        chapter_total,
                        next_chapter.saturating_add(1),
                        chapter_total
                    );
                    self.enter_chapter_navigation(selected_book, next_chapter, now_ms);
                    return;
                }

                if self.adjust_wpm(false) {
                    self.ui = UiState::Reading {
                        selected_book,
                        paused,
                        next_word_ms: if paused { next_word_ms } else { now_ms },
                    };
                    self.pending_redraw = true;
                }
            }
        }
    }

    fn apply_chapter_navigation_input(
        &mut self,
        selected_book: u16,
        chapter_cursor: u16,
        event: InputEvent,
        now_ms: u64,
    ) {
        let chapter_total = self.content.chapter_count().max(1);
        let chapter_cursor = chapter_cursor.min(chapter_total.saturating_sub(1));

        match event {
            InputEvent::RotateCw => {
                let next = rotate_cw(chapter_cursor, chapter_total);
                debug!(
                    "ui-nav: chapter rotate_cw selected_book={} chapter_cursor={}/{} -> {}/{}",
                    selected_book,
                    chapter_cursor.saturating_add(1),
                    chapter_total,
                    next.saturating_add(1),
                    chapter_total
                );
                self.ui = UiState::NavigateChapter {
                    selected_book,
                    chapter_cursor: next,
                };
                self.start_transition(AnimationKind::SlideLeft, now_ms, ANIM_NAV_ROTATE_MS);
                self.pending_redraw = true;
            }
            InputEvent::RotateCcw => {
                let next = rotate_ccw(chapter_cursor, chapter_total);
                debug!(
                    "ui-nav: chapter rotate_ccw selected_book={} chapter_cursor={}/{} -> {}/{}",
                    selected_book,
                    chapter_cursor.saturating_add(1),
                    chapter_total,
                    next.saturating_add(1),
                    chapter_total
                );
                self.ui = UiState::NavigateChapter {
                    selected_book,
                    chapter_cursor: next,
                };
                self.start_transition(AnimationKind::SlideRight, now_ms, ANIM_NAV_ROTATE_MS);
                self.pending_redraw = true;
            }
            InputEvent::Press => {
                let Some(chapter) = self.content.chapter_at(chapter_cursor) else {
                    self.set_status("NAVIGATION ERROR", "CHAPTER INVALID", now_ms);
                    return;
                };

                let current_paragraph = self.content.paragraph_index().saturating_sub(1);
                let chapter_start = chapter.start_paragraph;
                let chapter_end =
                    chapter_start.saturating_add(chapter.paragraph_count.saturating_sub(1));

                let initial_cursor = if (chapter_start..=chapter_end).contains(&current_paragraph) {
                    current_paragraph
                } else {
                    chapter_start
                };

                debug!(
                    "ui-nav: chapter press selected_book={} chapter_cursor={}/{} label={:?} start_paragraph={} paragraph_count={} current_paragraph={} initial_cursor={}",
                    selected_book,
                    chapter_cursor.saturating_add(1),
                    chapter_total,
                    chapter.label,
                    chapter_start,
                    chapter.paragraph_count,
                    current_paragraph,
                    initial_cursor
                );

                match self.content.seek_chapter(chapter_cursor) {
                    Ok(true) => {
                        debug!(
                            "ui-nav: chapter press seek accepted selected_book={} chapter_cursor={}/{} -> direct-reading",
                            selected_book,
                            chapter_cursor.saturating_add(1),
                            chapter_total
                        );
                        self.word_buffer.clear();
                        self.paragraph_word_index = 0;
                        self.paragraph_word_total = 1;
                        self.last_ends_clause = false;
                        self.last_ends_sentence = false;
                        let _ = self.advance_word();
                        self.ui = UiState::Reading {
                            selected_book,
                            paused: true,
                            next_word_ms: now_ms,
                        };
                        self.paused_since_ms = Some(now_ms);
                        self.last_pause_anim_slot = None;
                        self.start_transition(AnimationKind::SlideRight, now_ms, ANIM_NAV_MS);
                        self.pending_redraw = true;
                        return;
                    }
                    Ok(false) => {
                        debug!(
                            "ui-nav: chapter press seek unsupported selected_book={} chapter_cursor={}/{} -> paragraph-navigation",
                            selected_book,
                            chapter_cursor.saturating_add(1),
                            chapter_total
                        );
                    }
                    Err(_) => {
                        debug!(
                            "ui-nav: chapter press seek failed selected_book={} chapter_cursor={}/{}",
                            selected_book,
                            chapter_cursor.saturating_add(1),
                            chapter_total
                        );
                        self.set_status("NAVIGATION ERROR", "CHAPTER SEEK FAILED", now_ms);
                        return;
                    }
                }

                self.enter_paragraph_navigation(
                    selected_book,
                    chapter_cursor,
                    initial_cursor,
                    now_ms,
                );
            }
        }
    }

    fn apply_paragraph_navigation_input(
        &mut self,
        selected_book: u16,
        chapter_index: u16,
        paragraph_cursor: u16,
        event: InputEvent,
        now_ms: u64,
    ) {
        let Some(chapter) = self.content.chapter_at(chapter_index) else {
            self.set_status("NAVIGATION ERROR", "CHAPTER INVALID", now_ms);
            return;
        };

        let chapter_start = chapter.start_paragraph;
        let chapter_total = chapter.paragraph_count.max(1);
        let chapter_end = chapter_start.saturating_add(chapter_total.saturating_sub(1));
        let paragraph_cursor = paragraph_cursor.clamp(chapter_start, chapter_end);

        match event {
            InputEvent::RotateCw => {
                let rel = paragraph_cursor.saturating_sub(chapter_start);
                let next_rel = rotate_cw(rel, chapter_total);
                debug!(
                    "ui-nav: paragraph rotate_cw selected_book={} chapter_index={} cursor={} rel={}/{} -> rel={}",
                    selected_book,
                    chapter_index.saturating_add(1),
                    paragraph_cursor,
                    rel.saturating_add(1),
                    chapter_total,
                    next_rel.saturating_add(1)
                );
                self.ui = UiState::NavigateParagraph {
                    selected_book,
                    chapter_index,
                    paragraph_cursor: chapter_start.saturating_add(next_rel),
                };
                self.start_transition(AnimationKind::SlideLeft, now_ms, ANIM_NAV_ROTATE_MS);
                self.pending_redraw = true;
            }
            InputEvent::RotateCcw => {
                let rel = paragraph_cursor.saturating_sub(chapter_start);
                let next_rel = rotate_ccw(rel, chapter_total);
                debug!(
                    "ui-nav: paragraph rotate_ccw selected_book={} chapter_index={} cursor={} rel={}/{} -> rel={}",
                    selected_book,
                    chapter_index.saturating_add(1),
                    paragraph_cursor,
                    rel.saturating_add(1),
                    chapter_total,
                    next_rel.saturating_add(1)
                );
                self.ui = UiState::NavigateParagraph {
                    selected_book,
                    chapter_index,
                    paragraph_cursor: chapter_start.saturating_add(next_rel),
                };
                self.start_transition(AnimationKind::SlideRight, now_ms, ANIM_NAV_ROTATE_MS);
                self.pending_redraw = true;
            }
            InputEvent::Press => {
                debug!(
                    "ui-nav: paragraph press selected_book={} chapter_index={} target_paragraph={}",
                    selected_book,
                    chapter_index.saturating_add(1),
                    paragraph_cursor
                );
                self.apply_navigation_confirm(selected_book, paragraph_cursor, now_ms)
            }
        }
    }

    fn tick_countdown(&mut self, now_ms: u64) -> TickResult {
        if self.pending_redraw {
            self.pending_redraw = false;
            return TickResult::RenderRequested;
        }

        let (selected_book, mut remaining, mut next_step_ms) = match self.ui {
            UiState::Countdown {
                selected_book,
                remaining,
                next_step_ms,
            } => (selected_book, remaining, next_step_ms),
            _ => return TickResult::NoRender,
        };

        if now_ms < next_step_ms {
            return TickResult::NoRender;
        }

        if remaining > 1 {
            remaining -= 1;
            next_step_ms += 1_000;
            self.ui = UiState::Countdown {
                selected_book,
                remaining,
                next_step_ms,
            };
            self.start_transition(AnimationKind::Pulse, now_ms, 900);
            return TickResult::RenderRequested;
        }

        self.enter_reading(selected_book, now_ms);
        self.tick_reading(now_ms)
    }

    fn tick_reading(&mut self, now_ms: u64) -> TickResult {
        let (selected_book, paused, next_word_ms) = match self.ui {
            UiState::Reading {
                selected_book,
                paused,
                next_word_ms,
            } => (selected_book, paused, next_word_ms),
            _ => return TickResult::NoRender,
        };

        if paused {
            let slot = (now_ms / PAUSE_ANIM_FRAME_MS) as u32;
            if self.pending_redraw || self.last_pause_anim_slot != Some(slot) {
                self.pending_redraw = false;
                self.last_pause_anim_slot = Some(slot);
                return TickResult::RenderRequested;
            }
            return TickResult::NoRender;
        }
        self.last_pause_anim_slot = None;

        if self.pending_redraw && !self.word_buffer.is_empty() {
            self.pending_redraw = false;
            return TickResult::RenderRequested;
        }

        if self.word_buffer.is_empty() || now_ms >= next_word_ms {
            match self.advance_word() {
                Ok(AdvanceWordResult::Advanced) => {
                    self.ui = UiState::Reading {
                        selected_book,
                        paused: false,
                        next_word_ms: now_ms + self.current_word_delay_ms() as u64,
                    };
                    self.pending_redraw = false;
                    self.words_since_drain = self.words_since_drain.saturating_add(1);
                    return TickResult::RenderRequested;
                }
                Ok(AdvanceWordResult::AwaitingRefill) => {
                    self.ui = UiState::Reading {
                        selected_book,
                        paused: false,
                        next_word_ms: now_ms + 40,
                    };
                    self.pending_redraw = false;
                    return TickResult::NoRender;
                }
                Ok(AdvanceWordResult::EndOfText) => {
                    self.enter_library(selected_book, now_ms);
                    self.pending_redraw = false;
                    return TickResult::RenderRequested;
                }
                Err(()) => {
                    self.set_status("CONTENT ERROR", "CHECK SOURCE", now_ms);
                    self.pending_redraw = false;
                    return TickResult::RenderRequested;
                }
            }
        }

        TickResult::NoRender
    }

    fn advance_word(&mut self) -> Result<AdvanceWordResult, ()> {
        match self.content.next_word() {
            Ok(Some(token)) => {
                let mut staged_word = WordBuffer::<WORD_BUFFER_BYTES>::new();
                let (ends_sentence, ends_clause) = {
                    staged_word.set(token.text);
                    (token.ends_sentence, token.ends_clause)
                };

                self.word_buffer = staged_word;
                self.last_ends_sentence = ends_sentence;
                self.last_ends_clause = ends_clause;

                let (index, total) = self.content.paragraph_progress();
                self.paragraph_word_index = index;
                self.paragraph_word_total = total.max(1);
                Ok(AdvanceWordResult::Advanced)
            }
            Ok(None) => {
                if self.content.is_waiting_for_refill() {
                    Ok(AdvanceWordResult::AwaitingRefill)
                } else {
                    Ok(AdvanceWordResult::EndOfText)
                }
            }
            Err(_) => Err(()),
        }
    }

    fn current_word_delay_ms(&self) -> u32 {
        let base = 60_000u32 / self.config.wpm.max(1) as u32;
        let punctuation = if self.last_ends_sentence {
            self.config.dot_pause_ms as u32
        } else if self.last_ends_clause {
            self.config.comma_pause_ms as u32
        } else {
            0
        };

        base + punctuation
    }

    fn rotate_setting(&mut self, row: SettingsRow, clockwise: bool) {
        match row {
            SettingsRow::Font => {
                self.style.font_family = match (self.style.font_family, clockwise) {
                    (FontFamily::Serif, _) => FontFamily::Pixel,
                    (FontFamily::Pixel, _) => FontFamily::Serif,
                };
            }
            SettingsRow::Size => {
                self.style.font_size = match (self.style.font_size, clockwise) {
                    (FontSize::Small, true) => FontSize::Medium,
                    (FontSize::Medium, true) => FontSize::Large,
                    (FontSize::Large, true) => FontSize::Small,
                    (FontSize::Small, false) => FontSize::Large,
                    (FontSize::Medium, false) => FontSize::Small,
                    (FontSize::Large, false) => FontSize::Medium,
                };
            }
            SettingsRow::Invert => {
                self.style.inverted = !self.style.inverted;
            }
            SettingsRow::Wpm => {
                let _ = self.adjust_wpm(clockwise);
            }
            SettingsRow::Back => {}
        }
    }

    fn adjust_wpm(&mut self, increase: bool) -> bool {
        let next = if increase {
            self.config
                .wpm
                .saturating_add(WPM_STEP)
                .min(self.config.max_wpm)
        } else {
            self.config
                .wpm
                .saturating_sub(WPM_STEP)
                .max(self.config.min_wpm)
        };

        if next != self.config.wpm {
            self.config.wpm = next;
            true
        } else {
            false
        }
    }

    fn enter_library(&mut self, cursor: u16, now_ms: u64) {
        self.last_reading_press_ms = None;
        let max_index = self.library_item_count().saturating_sub(1);
        self.ui = UiState::Library {
            cursor: cursor.min(max_index),
        };
        self.start_transition(AnimationKind::SlideRight, now_ms, ANIM_MENU_MS);
        self.pending_redraw = true;
    }

    fn enter_settings(&mut self, cursor: u8, editing: bool, now_ms: u64) {
        self.ui = UiState::Settings { cursor, editing };
        self.start_transition(AnimationKind::SlideLeft, now_ms, ANIM_MENU_MS);
        self.pending_redraw = true;
    }

    fn enter_countdown(&mut self, selected_book: u16, now_ms: u64) {
        self.last_reading_press_ms = None;
        self.paused_since_ms = None;
        self.last_pause_anim_slot = None;
        self.word_buffer.clear();
        self.paragraph_word_index = 0;
        self.paragraph_word_total = 1;
        self.last_ends_clause = false;
        self.last_ends_sentence = false;

        self.ui = UiState::Countdown {
            selected_book,
            remaining: self.countdown_seconds,
            next_step_ms: now_ms + 1_000,
        };
        self.start_transition(AnimationKind::Pulse, now_ms, 900);
        self.pending_redraw = true;
    }

    fn enter_reading(&mut self, selected_book: u16, now_ms: u64) {
        self.last_reading_press_ms = None;
        self.paused_since_ms = None;
        self.last_pause_anim_slot = None;
        self.ui = UiState::Reading {
            selected_book,
            paused: false,
            next_word_ms: now_ms,
        };
        self.start_transition(AnimationKind::Fade, now_ms, ANIM_SCREEN_MS);
        self.pending_redraw = true;
    }

    fn enter_chapter_navigation(&mut self, selected_book: u16, chapter_cursor: u16, now_ms: u64) {
        self.last_reading_press_ms = None;
        self.last_pause_anim_slot = None;
        let chapter_total = self.content.chapter_count().max(1);
        debug!(
            "ui-nav: enter chapter navigation selected_book={} chapter_cursor={}/{}",
            selected_book,
            chapter_cursor.saturating_add(1),
            chapter_total
        );
        self.ui = UiState::NavigateChapter {
            selected_book,
            chapter_cursor: chapter_cursor.min(chapter_total.saturating_sub(1)),
        };
        self.start_transition(AnimationKind::SlideLeft, now_ms, ANIM_NAV_MS);
        self.pending_redraw = true;
    }

    fn enter_paragraph_navigation(
        &mut self,
        selected_book: u16,
        chapter_index: u16,
        paragraph_cursor: u16,
        now_ms: u64,
    ) {
        let Some(chapter) = self.content.chapter_at(chapter_index) else {
            self.set_status("NAVIGATION ERROR", "CHAPTER INVALID", now_ms);
            return;
        };

        let chapter_start = chapter.start_paragraph;
        let chapter_end = chapter_start.saturating_add(chapter.paragraph_count.saturating_sub(1));
        debug!(
            "ui-nav: enter paragraph navigation selected_book={} chapter_index={} label={:?} chapter_start={} chapter_end={} requested_cursor={}",
            selected_book,
            chapter_index.saturating_add(1),
            chapter.label,
            chapter_start,
            chapter_end,
            paragraph_cursor
        );

        self.ui = UiState::NavigateParagraph {
            selected_book,
            chapter_index,
            paragraph_cursor: paragraph_cursor.clamp(chapter_start, chapter_end),
        };
        self.start_transition(AnimationKind::SlideLeft, now_ms, ANIM_NAV_MS);
        self.pending_redraw = true;
    }

    fn apply_navigation_confirm(&mut self, selected_book: u16, target_paragraph: u16, now_ms: u64) {
        debug!(
            "ui-nav: confirm selected_book={} target_paragraph={}",
            selected_book, target_paragraph
        );
        if self.content.seek_paragraph(target_paragraph).is_err() {
            debug!(
                "ui-nav: confirm failed selected_book={} target_paragraph={} status=invalid_paragraph",
                selected_book, target_paragraph
            );
            self.set_status("NAVIGATION ERROR", "PARAGRAPH INVALID", now_ms);
            return;
        }
        debug!(
            "ui-nav: confirm applied selected_book={} target_paragraph={} paragraph_index={} paragraph_total={} chapter={}/{} chapter_label={:?} preview={:?}",
            selected_book,
            target_paragraph,
            self.content.paragraph_index(),
            self.content.paragraph_total(),
            self.current_chapter_index().saturating_add(1),
            self.content.chapter_count().max(1),
            self.content
                .chapter_at(self.current_chapter_index())
                .map(|chapter| chapter.label),
            self.content.paragraph_preview(target_paragraph)
        );

        self.word_buffer.clear();
        self.paragraph_word_index = 0;
        self.paragraph_word_total = 1;
        self.last_ends_clause = false;
        self.last_ends_sentence = false;

        let _ = self.advance_word();

        self.ui = UiState::Reading {
            selected_book,
            paused: true,
            next_word_ms: now_ms,
        };
        self.paused_since_ms = Some(now_ms);
        self.last_pause_anim_slot = None;
        self.start_transition(AnimationKind::SlideRight, now_ms, ANIM_NAV_MS);
        self.pending_redraw = true;
    }

    fn set_status(&mut self, line1: &'static str, line2: &'static str, now_ms: u64) {
        self.last_reading_press_ms = None;
        self.ui = UiState::Status { line1, line2 };
        self.start_transition(AnimationKind::Fade, now_ms, ANIM_SCREEN_MS);
        self.pending_redraw = true;
    }

    fn start_transition(&mut self, kind: AnimationKind, now_ms: u64, duration_ms: u16) {
        self.transition = Some(AnimationSpec::new(kind, now_ms, duration_ms));
    }

    fn transition_frame(&self, now_ms: u64) -> Option<crate::render::AnimationFrame> {
        self.transition.and_then(|anim| anim.frame(now_ms))
    }

    fn total_title_count(&self) -> u16 {
        self.content.title_count()
    }

    fn library_item_count(&self) -> u16 {
        self.total_title_count().saturating_add(1)
    }

    fn settings_item_index(&self) -> u16 {
        self.total_title_count()
    }

    fn chapter_for_paragraph(&self, paragraph_index: u16) -> u16 {
        let chapter_count = self.content.chapter_count().max(1);

        for chapter_idx in 0..chapter_count {
            if let Some(chapter) = self.content.chapter_at(chapter_idx) {
                let start = chapter.start_paragraph;
                let end = start.saturating_add(chapter.paragraph_count.saturating_sub(1));
                if (start..=end).contains(&paragraph_index) {
                    return chapter_idx;
                }
            }
        }

        0
    }

    fn current_chapter_index(&self) -> u16 {
        if let Some(index) = self.content.current_chapter_index() {
            return index.min(self.content.chapter_count().saturating_sub(1));
        }

        let current_paragraph = self.content.paragraph_index().saturating_sub(1);
        self.chapter_for_paragraph(current_paragraph)
    }
}

#[derive(Clone)]
struct WordBuffer<const N: usize> {
    bytes: [u8; N],
    len: usize,
}

impl<const N: usize> WordBuffer<N> {
    const fn new() -> Self {
        Self {
            bytes: [0u8; N],
            len: 0,
        }
    }

    fn clear(&mut self) {
        self.len = 0;
    }

    fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn set(&mut self, word: &str) {
        self.len = 0;

        for ch in word.chars() {
            let mut utf8 = [0u8; 4];
            let encoded = ch.encode_utf8(&mut utf8).as_bytes();
            if self.len + encoded.len() > N {
                break;
            }

            self.bytes[self.len..self.len + encoded.len()].copy_from_slice(encoded);
            self.len += encoded.len();
        }

        if self.len == 0 {
            self.bytes[0] = b'?';
            self.len = 1;
        }
    }

    fn as_str(&self) -> &str {
        if self.len == 0 {
            return "";
        }

        str::from_utf8(&self.bytes[..self.len]).unwrap_or("?")
    }
}

fn rotate_cw(current: u16, total: u16) -> u16 {
    if total == 0 { 0 } else { (current + 1) % total }
}

fn rotate_ccw(current: u16, total: u16) -> u16 {
    if total == 0 {
        0
    } else if current == 0 {
        total - 1
    } else {
        current - 1
    }
}

fn font_family_label(font: FontFamily) -> &'static str {
    match font {
        FontFamily::Serif => "Serif",
        FontFamily::Pixel => "Pixel",
    }
}

fn font_size_label(size: FontSize) -> &'static str {
    match size {
        FontSize::Small => "Small",
        FontSize::Medium => "Medium",
        FontSize::Large => "Large",
    }
}
