//! Content sources for RSVP rendering.

pub mod sd_catalog;
mod text_utils;

/// One token emitted by a [`WordSource`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WordToken<'a> {
    pub text: &'a str,
    pub ends_sentence: bool,
    pub ends_clause: bool,
}

/// Read-only catalog of available texts.
pub trait TextCatalog {
    fn title_count(&self) -> u16;
    fn title_at(&self, index: u16) -> Option<&str>;
    fn has_cover_at(&self, _index: u16) -> bool {
        false
    }
}

/// Abstract source of word tokens.
pub trait WordSource {
    type Error;

    /// Reset source to the beginning.
    fn reset(&mut self) -> Result<(), Self::Error>;

    /// Return next word token or `None` if source is exhausted.
    fn next_word<'a>(&'a mut self) -> Result<Option<WordToken<'a>>, Self::Error>;

    /// Current paragraph progress as `(word_index, word_total)`.
    fn paragraph_progress(&self) -> (u16, u16);

    /// Current paragraph number (1-based, `0` when unavailable).
    fn paragraph_index(&self) -> u16;

    /// Total paragraph count.
    fn paragraph_total(&self) -> u16;

    /// Whether the source has temporarily run out of buffered words but expects
    /// more data to be loaded.
    fn is_waiting_for_refill(&self) -> bool {
        false
    }
}

/// Word source that can switch to a selected text from a catalog.
pub trait SelectableWordSource: WordSource {
    /// Select a text by its catalog index and reset position to the start.
    fn select_text(&mut self, index: u16) -> Result<(), Self::Error>;

    /// Returns the currently selected catalog index.
    fn selected_index(&self) -> u16;
}

/// Word source that can jump to a paragraph boundary.
pub trait ParagraphNavigator: WordSource {
    /// Jump to a paragraph by zero-based index and reset word position.
    fn seek_paragraph(&mut self, paragraph_index: u16) -> Result<(), Self::Error>;
}

/// Coarse/fine navigation metadata for chapter -> paragraph flows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChapterInfo<'a> {
    pub label: &'a str,
    pub start_paragraph: u16,
    pub paragraph_count: u16,
}

/// Navigation-aware catalog. Current implementation uses pseudo chapters.
pub trait NavigationCatalog: WordSource {
    /// Number of chapters for the selected text.
    fn chapter_count(&self) -> u16;

    /// Chapter metadata by zero-based index.
    fn chapter_at(&self, index: u16) -> Option<ChapterInfo<'_>>;

    /// Current chapter index for the selected text when available.
    fn current_chapter_index(&self) -> Option<u16> {
        None
    }

    /// Request chapter-level seek when source supports it.
    /// Returns `Ok(true)` when seek request was accepted.
    fn seek_chapter(&mut self, _chapter_index: u16) -> Result<bool, Self::Error> {
        Ok(false)
    }

    /// Preview string for a paragraph (zero-based index).
    fn paragraph_preview(&self, paragraph_index: u16) -> Option<&str>;
}
