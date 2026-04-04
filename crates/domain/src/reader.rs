extern crate alloc;

use alloc::boxed::Box;

use crate::{
    content::{ArticleId, CONTENT_TITLE_MAX_BYTES, CollectionKind},
    formatter::{ReadingDocument, ReadingUnit},
    text::InlineText,
};

const EMPTY_READING_DOCUMENT: ReadingDocument = ReadingDocument::empty();

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
    pub unit_index: u16,
    pub paragraph_index: u8,
    pub total_paragraphs: u8,
    pub completion_percent: u8,
}

#[derive(Debug, Clone)]
pub struct ReaderSession {
    pub active_article: ArticleId,
    pub active_collection: CollectionKind,
    pub title: InlineText<CONTENT_TITLE_MAX_BYTES>,
    document: Option<Box<ReadingDocument>>,
    pub progress: ReaderProgress,
    pub mode: ReaderMode,
    pub resume_mode: ReaderMode,
    pub chat_available: bool,
    pub next_due_at_ms: Option<u64>,
}

impl ReaderSession {
    pub fn new() -> Self {
        Self {
            active_article: ArticleId(102),
            active_collection: CollectionKind::Saved,
            title: InlineText::new(),
            document: None,
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
        self.active_collection = collection;
        self.active_article = article;
        self.title = title;
        self.document = Some(document);
        self.progress.unit_index = 0;
        self.sync_progress();
        self.mode = ReaderMode::Normal;
        self.resume_mode = ReaderMode::Normal;
        self.chat_available = chat_available;
        self.next_due_at_ms = None;
    }

    pub fn unload_document(&mut self) {
        self.document = None;
        self.progress = ReaderProgress {
            unit_index: 0,
            paragraph_index: 1,
            total_paragraphs: 1,
            completion_percent: 0,
        };
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

    pub fn commit_paragraph_navigation(&mut self) {
        if matches!(self.mode, ReaderMode::ParagraphNavigation) {
            self.mode = self.resume_mode;
            self.progress.unit_index = self
                .document()
                .paragraph_start(self.progress.paragraph_index);
            self.sync_progress();
            self.next_due_at_ms = None;
        }
    }

    pub fn move_paragraph(&mut self, previous: bool) {
        let min_paragraph = 1;
        let max_paragraph = self.progress.total_paragraphs.max(1);

        self.progress.paragraph_index = if previous {
            self.progress
                .paragraph_index
                .saturating_sub(1)
                .max(min_paragraph)
        } else {
            (self.progress.paragraph_index.saturating_add(1)).min(max_paragraph)
        };
    }

    pub const fn is_active_reading(&self) -> bool {
        matches!(self.mode, ReaderMode::Normal | ReaderMode::Chat)
    }

    pub fn advance_if_due(&mut self, now_ms: u64, wpm: u16) -> bool {
        let document = self.document();
        if !self.is_active_reading() || document.is_empty() {
            return false;
        }

        let current = self.current_unit();
        let next_due = self
            .next_due_at_ms
            .unwrap_or_else(|| now_ms.saturating_add(current.dwell_ms(wpm) as u64));

        if self.next_due_at_ms.is_none() {
            self.next_due_at_ms = Some(next_due);
            return false;
        }

        if now_ms < next_due {
            return false;
        }

        if (self.progress.unit_index as usize + 1) >= document.unit_count as usize {
            self.next_due_at_ms = None;
            self.progress.completion_percent = 100;
            return false;
        }

        self.progress.unit_index = self.progress.unit_index.saturating_add(1);
        self.sync_progress();
        self.next_due_at_ms = Some(now_ms.saturating_add(self.current_unit().dwell_ms(wpm) as u64));
        true
    }

    pub fn document(&self) -> &ReadingDocument {
        self.document.as_deref().unwrap_or(&EMPTY_READING_DOCUMENT)
    }

    pub fn current_unit(&self) -> &ReadingUnit {
        self.document().unit(self.progress.unit_index)
    }

    pub const fn progress_width_px(&self) -> u16 {
        ((400u32 * self.progress.completion_percent as u32) / 100u32) as u16
    }

    fn sync_progress(&mut self) {
        let (total_paragraphs, current_paragraph, unit_count) = {
            let document = self.document();
            (
                document.paragraph_count.max(1),
                document
                    .unit(self.progress.unit_index)
                    .paragraph_index
                    .max(1),
                document.unit_count,
            )
        };

        self.progress.total_paragraphs = total_paragraphs;
        self.progress.paragraph_index = current_paragraph;

        let total_units = unit_count.max(1) as u32;
        let current = self.progress.unit_index.min(unit_count.saturating_sub(1)) as u32 + 1;
        self.progress.completion_percent = ((current * 100) / total_units) as u8;
    }
}

impl Default for ReaderSession {
    fn default() -> Self {
        Self::new()
    }
}
