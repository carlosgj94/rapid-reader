extern crate alloc;

use alloc::boxed::Box;

use crate::{
    content::{ArticleId, CONTENT_ID_MAX_BYTES, CONTENT_TITLE_MAX_BYTES, CollectionKind},
    formatter::{MAX_PARAGRAPH_PREVIEW_BYTES, ReadingDocument, ReadingUnit},
    text::InlineText,
};

pub const READER_WINDOW_MAX_UNITS: usize = 128;
const READER_WINDOW_OVERLAP_UNITS: u32 = 32;
const READER_WINDOW_PREFETCH_THRESHOLD_UNITS: u32 = 24;

const EMPTY_READER_WINDOW: ReaderWindow = ReaderWindow::empty();

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum ReaderMode {
    #[default]
    Normal,
    Chat,
    Paused,
    ParagraphNavigation,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub struct ReaderProgress {
    pub unit_index: u32,
    pub paragraph_index: u16,
    pub total_paragraphs: u16,
    pub completion_percent: u8,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub struct ReaderParagraphInfo {
    pub start_unit_index: u32,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ReaderWindow {
    pub start_unit_index: u32,
    pub unit_count: u16,
    pub units: [ReadingUnit; READER_WINDOW_MAX_UNITS],
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ReaderWindowLoadRequest {
    pub content_id: InlineText<CONTENT_ID_MAX_BYTES>,
    pub window_start_unit_index: u32,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub struct ReaderAdvanceOutcome {
    pub advanced: bool,
    pub load_request: Option<ReaderWindowLoadRequest>,
}

#[derive(Debug, Clone)]
pub struct ReaderSession {
    pub active_article: ArticleId,
    pub active_collection: CollectionKind,
    pub active_content_id: InlineText<CONTENT_ID_MAX_BYTES>,
    pub title: InlineText<CONTENT_TITLE_MAX_BYTES>,
    active_window: Option<Box<ReaderWindow>>,
    prefetched_window: Option<Box<ReaderWindow>>,
    paragraphs: Option<Box<[ReaderParagraphInfo]>>,
    total_units: u32,
    pending_window_start_unit_index: Option<u32>,
    pending_seek_unit_index: Option<u32>,
    pub progress: ReaderProgress,
    pub mode: ReaderMode,
    pub resume_mode: ReaderMode,
    pub chat_available: bool,
    pub next_due_at_ms: Option<u64>,
}

impl ReaderWindow {
    pub const fn empty() -> Self {
        Self {
            start_unit_index: 0,
            unit_count: 0,
            units: [ReadingUnit::new(); READER_WINDOW_MAX_UNITS],
        }
    }

    pub const fn is_empty(&self) -> bool {
        self.unit_count == 0
    }

    pub fn contains(&self, global_unit_index: u32) -> bool {
        !self.is_empty()
            && global_unit_index >= self.start_unit_index
            && global_unit_index < self.start_unit_index.saturating_add(self.unit_count as u32)
    }

    pub fn unit_at(&self, global_unit_index: u32) -> &ReadingUnit {
        if self.is_empty() {
            return &self.units[0];
        }

        let relative = global_unit_index
            .saturating_sub(self.start_unit_index)
            .min(self.unit_count.saturating_sub(1) as u32) as usize;
        &self.units[relative]
    }
}

impl ReaderSession {
    pub fn new() -> Self {
        Self {
            active_article: ArticleId(102),
            active_collection: CollectionKind::Saved,
            active_content_id: InlineText::new(),
            title: InlineText::new(),
            active_window: None,
            prefetched_window: None,
            paragraphs: None,
            total_units: 0,
            pending_window_start_unit_index: None,
            pending_seek_unit_index: None,
            progress: ReaderProgress {
                unit_index: 0,
                paragraph_index: 1,
                total_paragraphs: 1,
                completion_percent: 0,
            },
            mode: ReaderMode::Normal,
            resume_mode: ReaderMode::Normal,
            chat_available: true,
            next_due_at_ms: None,
        }
    }

    pub fn open_article(
        &mut self,
        collection: CollectionKind,
        article: ArticleId,
        title: InlineText<CONTENT_TITLE_MAX_BYTES>,
        document: Box<ReadingDocument>,
        chat_available: bool,
    ) {
        let paragraph_count = document.paragraph_count as usize;
        let mut paragraphs = alloc::vec::Vec::with_capacity(paragraph_count);
        let mut paragraph_index = 0usize;
        while paragraph_index < paragraph_count {
            paragraphs.push(ReaderParagraphInfo {
                start_unit_index: document.paragraphs[paragraph_index].start_unit_index as u32,
            });
            paragraph_index += 1;
        }

        let mut window = Box::new(ReaderWindow::empty());
        let unit_count = document.unit_count.min(READER_WINDOW_MAX_UNITS as u16);
        let mut unit_index = 0usize;
        while unit_index < unit_count as usize {
            window.units[unit_index] = document.units[unit_index];
            unit_index += 1;
        }
        window.unit_count = unit_count;

        self.open_cached_reader_content(
            collection,
            article,
            InlineText::new(),
            title,
            document.unit_count as u32,
            paragraphs.into_boxed_slice(),
            window,
            chat_available,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn open_cached_reader_content(
        &mut self,
        collection: CollectionKind,
        article: ArticleId,
        content_id: InlineText<CONTENT_ID_MAX_BYTES>,
        title: InlineText<CONTENT_TITLE_MAX_BYTES>,
        total_units: u32,
        paragraphs: Box<[ReaderParagraphInfo]>,
        window: Box<ReaderWindow>,
        chat_available: bool,
    ) {
        self.active_collection = collection;
        self.active_article = article;
        self.active_content_id = content_id;
        self.title = title;
        self.total_units = total_units;
        self.paragraphs = Some(paragraphs);
        self.active_window = Some(window);
        self.prefetched_window = None;
        self.pending_window_start_unit_index = None;
        self.pending_seek_unit_index = None;
        self.progress.unit_index = 0;
        self.sync_progress();
        self.mode = ReaderMode::Normal;
        self.resume_mode = ReaderMode::Normal;
        self.chat_available = chat_available;
        self.next_due_at_ms = None;
    }

    pub fn apply_loaded_window(&mut self, window: Box<ReaderWindow>) {
        let pending_seek = self.pending_seek_unit_index;
        self.pending_window_start_unit_index = None;

        if let Some(target_unit_index) = pending_seek
            && window.contains(target_unit_index)
        {
            self.active_window = Some(window);
            self.prefetched_window = None;
            self.pending_seek_unit_index = None;
            self.progress.unit_index = target_unit_index;
            self.sync_progress();
            self.next_due_at_ms = None;
            return;
        }

        let replace_active = self.active_window.as_ref().is_none_or(|active| {
            active.is_empty() || window.start_unit_index <= active.start_unit_index
        });

        if replace_active {
            self.active_window = Some(window);
            self.sync_progress();
        } else {
            self.prefetched_window = Some(window);
        }
    }

    pub fn unload_document(&mut self) {
        self.active_content_id = InlineText::new();
        self.active_window = None;
        self.prefetched_window = None;
        self.paragraphs = None;
        self.total_units = 0;
        self.pending_window_start_unit_index = None;
        self.pending_seek_unit_index = None;
        self.progress = ReaderProgress {
            unit_index: 0,
            paragraph_index: 1,
            total_paragraphs: 1,
            completion_percent: 0,
        };
        self.mode = ReaderMode::Normal;
        self.resume_mode = ReaderMode::Normal;
        self.next_due_at_ms = None;
    }

    pub fn clear_pending_window_request(&mut self) {
        self.pending_window_start_unit_index = None;
        self.pending_seek_unit_index = None;
        self.prefetched_window = None;
        self.next_due_at_ms = None;
    }

    pub fn show_normal(&mut self) {
        if matches!(self.mode, ReaderMode::Normal | ReaderMode::Chat) {
            self.mode = ReaderMode::Normal;
            self.resume_mode = ReaderMode::Normal;
        }
    }

    pub fn show_chat(&mut self) {
        if self.chat_available && matches!(self.mode, ReaderMode::Normal | ReaderMode::Chat) {
            self.mode = ReaderMode::Chat;
            self.resume_mode = ReaderMode::Chat;
        }
    }

    pub fn pause(&mut self) {
        if matches!(self.mode, ReaderMode::Normal | ReaderMode::Chat) {
            self.resume_mode = self.mode;
            self.mode = ReaderMode::Paused;
            self.next_due_at_ms = None;
        }
    }

    pub fn resume(&mut self) {
        if matches!(self.mode, ReaderMode::Paused) {
            self.mode = self.resume_mode;
            self.next_due_at_ms = None;
        }
    }

    pub fn open_paragraph_navigation(&mut self) {
        if matches!(self.mode, ReaderMode::Paused) {
            self.mode = ReaderMode::ParagraphNavigation;
            self.next_due_at_ms = None;
        }
    }

    pub fn close_paragraph_navigation(&mut self) {
        if matches!(self.mode, ReaderMode::ParagraphNavigation) {
            self.mode = ReaderMode::Paused;
        }
    }

    pub fn commit_paragraph_navigation(&mut self) -> Option<ReaderWindowLoadRequest> {
        if !matches!(self.mode, ReaderMode::ParagraphNavigation) {
            return None;
        }

        self.mode = self.resume_mode;
        self.next_due_at_ms = None;

        let target_unit_index = self.paragraph_start(self.progress.paragraph_index);
        if self.active_window().contains(target_unit_index) {
            self.progress.unit_index = target_unit_index;
            self.sync_progress();
            return None;
        }

        self.pending_seek_unit_index = Some(target_unit_index);
        self.load_request_for_window_start(self.window_start_for_unit(target_unit_index))
    }

    pub fn move_paragraph(&mut self, previous: bool) {
        let min_paragraph = 1u16;
        let max_paragraph = self.progress.total_paragraphs.max(1);

        self.progress.paragraph_index = if previous {
            self.progress
                .paragraph_index
                .saturating_sub(1)
                .max(min_paragraph)
        } else {
            self.progress
                .paragraph_index
                .saturating_add(1)
                .min(max_paragraph)
        };
    }

    pub const fn is_active_reading(&self) -> bool {
        matches!(self.mode, ReaderMode::Normal | ReaderMode::Chat)
    }

    pub fn advance_if_due(&mut self, now_ms: u64, wpm: u16) -> ReaderAdvanceOutcome {
        let mut outcome = ReaderAdvanceOutcome::default();
        if !self.is_active_reading() || self.active_window().is_empty() {
            return outcome;
        }

        let current = self.current_unit();
        let next_due = self
            .next_due_at_ms
            .unwrap_or_else(|| now_ms.saturating_add(current.dwell_ms(wpm) as u64));

        if self.next_due_at_ms.is_none() {
            self.next_due_at_ms = Some(next_due);
            outcome.load_request = self.maybe_request_prefetch();
            return outcome;
        }

        if now_ms < next_due {
            outcome.load_request = self.maybe_request_prefetch();
            return outcome;
        }

        if self.progress.unit_index.saturating_add(1) >= self.total_units.max(1) {
            self.next_due_at_ms = None;
            self.progress.completion_percent = 100;
            return outcome;
        }

        let next_unit_index = self.progress.unit_index.saturating_add(1);
        if !self.active_window().contains(next_unit_index) {
            if self
                .prefetched_window
                .as_ref()
                .is_some_and(|window| window.contains(next_unit_index))
            {
                self.active_window = self.prefetched_window.take();
            } else {
                self.next_due_at_ms = None;
                outcome.load_request =
                    self.load_request_for_window_start(self.window_start_for_unit(next_unit_index));
                return outcome;
            }
        }

        self.progress.unit_index = next_unit_index;
        self.sync_progress();
        self.next_due_at_ms = Some(now_ms.saturating_add(self.current_unit().dwell_ms(wpm) as u64));
        outcome.advanced = true;
        outcome.load_request = self.maybe_request_prefetch();
        outcome
    }

    pub fn current_unit(&self) -> &ReadingUnit {
        self.active_window().unit_at(self.progress.unit_index)
    }

    pub fn preview_for_paragraph(
        &self,
        paragraph_index: u16,
    ) -> InlineText<MAX_PARAGRAPH_PREVIEW_BYTES> {
        Self::preview_from_window(self.active_window(), paragraph_index)
            .or_else(|| {
                self.prefetched_window
                    .as_deref()
                    .and_then(|window| Self::preview_from_window(window, paragraph_index))
            })
            .unwrap_or_default()
    }

    pub fn paragraph_start(&self, paragraph_index: u16) -> u32 {
        let Some(paragraphs) = self.paragraphs.as_deref() else {
            return 0;
        };
        let safe_index = paragraph_index
            .saturating_sub(1)
            .min(paragraphs.len().saturating_sub(1) as u16) as usize;
        paragraphs[safe_index].start_unit_index
    }

    pub fn active_window(&self) -> &ReaderWindow {
        self.active_window
            .as_deref()
            .unwrap_or(&EMPTY_READER_WINDOW)
    }

    pub const fn progress_width_px(&self) -> u16 {
        ((400u32 * self.progress.completion_percent as u32) / 100u32) as u16
    }

    pub fn is_empty(&self) -> bool {
        self.total_units == 0 || self.active_window().is_empty()
    }

    fn sync_progress(&mut self) {
        let total_paragraphs = self
            .paragraphs
            .as_deref()
            .map(|paragraphs| paragraphs.len() as u16)
            .unwrap_or(0)
            .max(1);
        self.progress.total_paragraphs = total_paragraphs;
        self.progress.paragraph_index = self.find_paragraph_for_unit(self.progress.unit_index);

        let total_units = self.total_units.max(1);
        let current = self.progress.unit_index.min(total_units.saturating_sub(1)) + 1;
        self.progress.completion_percent = ((current * 100) / total_units) as u8;
    }

    fn find_paragraph_for_unit(&self, unit_index: u32) -> u16 {
        let Some(paragraphs) = self.paragraphs.as_deref() else {
            return 1;
        };
        if paragraphs.is_empty() {
            return 1;
        }

        let mut low = 0usize;
        let mut high = paragraphs.len();
        while low + 1 < high {
            let mid = (low + high) / 2;
            if paragraphs[mid].start_unit_index <= unit_index {
                low = mid;
            } else {
                high = mid;
            }
        }

        (low + 1) as u16
    }

    fn maybe_request_prefetch(&mut self) -> Option<ReaderWindowLoadRequest> {
        if self.active_content_id.is_empty()
            || self.prefetched_window.is_some()
            || self.pending_window_start_unit_index.is_some()
            || self.total_units == 0
        {
            return None;
        }

        let active = self.active_window();
        if active.is_empty() {
            return None;
        }

        let window_end = active
            .start_unit_index
            .saturating_add(active.unit_count as u32);
        if window_end >= self.total_units {
            return None;
        }

        let remaining_in_window =
            window_end.saturating_sub(self.progress.unit_index.saturating_add(1));
        if remaining_in_window > READER_WINDOW_PREFETCH_THRESHOLD_UNITS {
            return None;
        }

        let next_start = active
            .start_unit_index
            .saturating_add(active.unit_count as u32)
            .saturating_sub(READER_WINDOW_OVERLAP_UNITS);
        self.load_request_for_window_start(next_start)
    }

    fn load_request_for_window_start(
        &mut self,
        window_start_unit_index: u32,
    ) -> Option<ReaderWindowLoadRequest> {
        if self.active_content_id.is_empty() {
            return None;
        }

        let clamped_start = window_start_unit_index.min(self.total_units.saturating_sub(1));
        if self.pending_window_start_unit_index == Some(clamped_start)
            || self.active_window().start_unit_index == clamped_start
            || self
                .prefetched_window
                .as_ref()
                .is_some_and(|window| window.start_unit_index == clamped_start)
        {
            return None;
        }

        self.pending_window_start_unit_index = Some(clamped_start);
        Some(ReaderWindowLoadRequest {
            content_id: self.active_content_id,
            window_start_unit_index: clamped_start,
        })
    }

    fn window_start_for_unit(&self, unit_index: u32) -> u32 {
        unit_index.saturating_sub(READER_WINDOW_OVERLAP_UNITS)
    }

    fn preview_from_window(
        window: &ReaderWindow,
        paragraph_index: u16,
    ) -> Option<InlineText<MAX_PARAGRAPH_PREVIEW_BYTES>> {
        if window.is_empty() {
            return None;
        }

        let target = paragraph_index.min(u8::MAX as u16) as u8;
        let mut preview = InlineText::new();
        let mut found = false;
        let mut unit_index = 0usize;
        while unit_index < window.unit_count as usize {
            let unit = &window.units[unit_index];
            if unit.paragraph_index < target {
                unit_index += 1;
                continue;
            }
            if unit.paragraph_index > target {
                break;
            }

            if found {
                let _ = preview.try_push_char(' ');
            }
            let _ = preview.try_push_str(unit.display.as_str());
            found = true;

            if unit.flags.paragraph_end {
                break;
            }
            unit_index += 1;
        }

        found.then_some(preview)
    }
}

impl Default for ReaderSession {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        content::{ArticleDocument, ReaderScript},
        formatter::format_article_document,
        source::SourceKind,
    };

    #[test]
    fn built_in_document_opens_inside_windowed_reader() {
        let document = format_article_document(&ArticleDocument::new(
            SourceKind::Unknown,
            ReaderScript::MachineSoul,
        ));
        let mut session = ReaderSession::new();

        session.open_article(
            CollectionKind::Saved,
            ArticleId(1),
            InlineText::from_slice("Example"),
            Box::new(document),
            false,
        );

        assert!(!session.is_empty());
        assert_eq!(session.progress.total_paragraphs, 8);
    }

    #[test]
    fn window_contains_checks_global_indices() {
        let mut window = ReaderWindow::empty();
        window.start_unit_index = 32;
        window.unit_count = 4;

        assert!(!window.contains(31));
        assert!(window.contains(32));
        assert!(window.contains(35));
        assert!(!window.contains(36));
    }

    #[test]
    fn paragraph_lookup_uses_global_indices() {
        let mut session = ReaderSession::new();
        session.paragraphs = Some(
            alloc::vec![
                ReaderParagraphInfo {
                    start_unit_index: 0,
                },
                ReaderParagraphInfo {
                    start_unit_index: 5,
                },
                ReaderParagraphInfo {
                    start_unit_index: 9,
                },
            ]
            .into_boxed_slice(),
        );

        assert_eq!(session.find_paragraph_for_unit(0), 1);
        assert_eq!(session.find_paragraph_for_unit(6), 2);
        assert_eq!(session.find_paragraph_for_unit(20), 3);
    }

    #[test]
    fn commit_navigation_requests_window_when_target_is_not_loaded() {
        let mut session = ReaderSession::new();
        session.active_content_id = InlineText::from_slice("content-1");
        session.total_units = 400;
        session.mode = ReaderMode::ParagraphNavigation;
        session.resume_mode = ReaderMode::Normal;
        session.progress.paragraph_index = 10;
        session.progress.total_paragraphs = 10;
        session.paragraphs = Some(
            alloc::vec![
                ReaderParagraphInfo {
                    start_unit_index: 0,
                };
                10
            ]
            .into_boxed_slice(),
        );
        if let Some(paragraphs) = session.paragraphs.as_mut() {
            paragraphs[9].start_unit_index = 256;
        }
        session.active_window = Some(Box::new(ReaderWindow::empty()));

        let request = session.commit_paragraph_navigation().unwrap();

        assert_eq!(request.window_start_unit_index, 224);
        assert_eq!(request.content_id.as_str(), "content-1");
    }
}
