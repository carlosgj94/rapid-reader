use super::{
    ChapterInfo, NavigationCatalog, ParagraphNavigator, SelectableWordSource, TextCatalog,
    WordSource, WordToken,
    text_utils::{count_words, first_words_excerpt, next_word_at},
};
use crate::content::static_source::DON_QUIJOTE_PARAGRAPHS;

const PARAGRAPHS_PER_CHAPTER: usize = 2;
const CHAPTER_LABEL_WORDS: usize = 6;

const ALICE_PARAGRAPHS: [&str; 2] = [
    "Alice was beginning to get very tired of sitting by her sister on the bank, \
and of having nothing to do: once or twice she had peeped into the book her sister \
was reading, but it had no pictures or conversations in it.",
    "So she was considering in her own mind whether the pleasure of making a \
daisy-chain would be worth the trouble of getting up and picking the daisies, when \
suddenly a White Rabbit with pink eyes ran close by her.",
];

const MOBY_PARAGRAPHS: [&str; 2] = [
    "Call me Ishmael. Some years ago, never mind how long precisely, having little \
or no money in my purse, and nothing particular to interest me on shore, I thought I \
would sail about a little and see the watery part of the world.",
    "It is a way I have of driving off the spleen and regulating the circulation. \
Whenever I find myself growing grim about the mouth; whenever it is a damp, drizzly \
November in my soul; then, I account it high time to get to sea as soon as I can.",
];

#[derive(Clone, Copy)]
struct BookEntry {
    title: &'static str,
    paragraphs: &'static [&'static str],
}

const BOOKS: [BookEntry; 3] = [
    BookEntry {
        title: "Don Quijote",
        paragraphs: &DON_QUIJOTE_PARAGRAPHS,
    },
    BookEntry {
        title: "Alice in Wonderland",
        paragraphs: &ALICE_PARAGRAPHS,
    },
    BookEntry {
        title: "Moby Dick",
        paragraphs: &MOBY_PARAGRAPHS,
    },
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SdStubError {
    InvalidTextIndex,
    InvalidParagraphIndex,
}

/// In-memory fake SD source used until card hardware integration is ready.
#[derive(Debug, Clone)]
pub struct FakeSdCatalogSource {
    selected_book: usize,
    paragraph_index: usize,
    paragraph_cursor: usize,
    paragraph_word_index: u16,
    paragraph_word_total: u16,
}

impl Default for FakeSdCatalogSource {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeSdCatalogSource {
    pub fn new() -> Self {
        let mut source = Self {
            selected_book: 0,
            paragraph_index: 0,
            paragraph_cursor: 0,
            paragraph_word_index: 0,
            paragraph_word_total: 1,
        };
        source.paragraph_word_total = source.compute_current_word_total();
        source
    }

    fn selected_paragraphs(&self) -> &'static [&'static str] {
        BOOKS[self.selected_book].paragraphs
    }

    fn compute_current_word_total(&self) -> u16 {
        let paragraphs = self.selected_paragraphs();
        if paragraphs.is_empty() {
            return 1;
        }

        let count = count_words(paragraphs[self.paragraph_index]);
        count.clamp(1, u16::MAX as usize) as u16
    }

    fn reset_read_pointer(&mut self) {
        self.paragraph_index = 0;
        self.paragraph_cursor = 0;
        self.paragraph_word_index = 0;
        self.paragraph_word_total = self.compute_current_word_total();
    }

    fn advance_paragraph(&mut self) -> bool {
        let paragraphs = self.selected_paragraphs();
        if paragraphs.is_empty() {
            return false;
        }

        if self.paragraph_index + 1 >= paragraphs.len() {
            return false;
        }

        self.paragraph_index += 1;
        self.paragraph_cursor = 0;
        self.paragraph_word_index = 0;
        self.paragraph_word_total = self.compute_current_word_total();
        true
    }
}

impl TextCatalog for FakeSdCatalogSource {
    fn title_count(&self) -> u16 {
        BOOKS.len().clamp(0, u16::MAX as usize) as u16
    }

    fn title_at(&self, index: u16) -> Option<&str> {
        BOOKS.get(index as usize).map(|entry| entry.title)
    }
}

impl WordSource for FakeSdCatalogSource {
    type Error = SdStubError;

    fn reset(&mut self) -> Result<(), Self::Error> {
        self.reset_read_pointer();
        Ok(())
    }

    fn next_word<'a>(&'a mut self) -> Result<Option<WordToken<'a>>, Self::Error> {
        let paragraphs = self.selected_paragraphs();
        if paragraphs.is_empty() {
            return Ok(None);
        }

        loop {
            let paragraph = paragraphs[self.paragraph_index];
            if let Some((word, next_cursor)) = next_word_at(paragraph, self.paragraph_cursor) {
                self.paragraph_cursor = next_cursor;
                self.paragraph_word_index = self.paragraph_word_index.saturating_add(1);

                let ends_sentence =
                    word.ends_with('.') || word.ends_with('!') || word.ends_with('?');
                let ends_clause = word.ends_with(',');

                return Ok(Some(WordToken {
                    text: word,
                    ends_sentence,
                    ends_clause,
                }));
            }

            if !self.advance_paragraph() {
                return Ok(None);
            }
        }
    }

    fn paragraph_progress(&self) -> (u16, u16) {
        (self.paragraph_word_index, self.paragraph_word_total.max(1))
    }

    fn paragraph_index(&self) -> u16 {
        let paragraphs = self.selected_paragraphs();
        if paragraphs.is_empty() {
            0
        } else {
            (self.paragraph_index + 1) as u16
        }
    }

    fn paragraph_total(&self) -> u16 {
        self.selected_paragraphs().len().clamp(0, u16::MAX as usize) as u16
    }
}

impl SelectableWordSource for FakeSdCatalogSource {
    fn select_text(&mut self, index: u16) -> Result<(), Self::Error> {
        let idx = index as usize;
        if idx >= BOOKS.len() {
            return Err(SdStubError::InvalidTextIndex);
        }

        self.selected_book = idx;
        self.reset_read_pointer();
        Ok(())
    }

    fn selected_index(&self) -> u16 {
        self.selected_book as u16
    }
}

impl ParagraphNavigator for FakeSdCatalogSource {
    fn seek_paragraph(&mut self, paragraph_index: u16) -> Result<(), Self::Error> {
        let paragraphs = self.selected_paragraphs();
        if paragraphs.is_empty() {
            self.paragraph_index = 0;
            self.paragraph_cursor = 0;
            self.paragraph_word_index = 0;
            self.paragraph_word_total = 1;
            return Ok(());
        }

        let index = paragraph_index as usize;
        if index >= paragraphs.len() {
            return Err(SdStubError::InvalidParagraphIndex);
        }

        self.paragraph_index = index;
        self.paragraph_cursor = 0;
        self.paragraph_word_index = 0;
        self.paragraph_word_total = self.compute_current_word_total();
        Ok(())
    }
}

impl NavigationCatalog for FakeSdCatalogSource {
    fn chapter_count(&self) -> u16 {
        let len = self.selected_paragraphs().len();
        if len == 0 {
            return 1;
        }

        len.div_ceil(PARAGRAPHS_PER_CHAPTER)
            .clamp(1, u16::MAX as usize) as u16
    }

    fn chapter_at(&self, index: u16) -> Option<ChapterInfo<'_>> {
        let paragraphs = self.selected_paragraphs();
        if paragraphs.is_empty() {
            return Some(ChapterInfo {
                label: "Empty",
                start_paragraph: 0,
                paragraph_count: 1,
            });
        }

        let chapter_index = index as usize;
        let chapter_count = paragraphs.len().div_ceil(PARAGRAPHS_PER_CHAPTER);
        if chapter_index >= chapter_count {
            return None;
        }

        let start = chapter_index * PARAGRAPHS_PER_CHAPTER;
        let remaining = paragraphs.len().saturating_sub(start);
        let count = remaining.min(PARAGRAPHS_PER_CHAPTER);
        let label = first_words_excerpt(paragraphs[start], CHAPTER_LABEL_WORDS);

        Some(ChapterInfo {
            label,
            start_paragraph: start as u16,
            paragraph_count: count as u16,
        })
    }

    fn paragraph_preview(&self, paragraph_index: u16) -> Option<&str> {
        self.selected_paragraphs()
            .get(paragraph_index as usize)
            .copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_are_exposed() {
        let src = FakeSdCatalogSource::new();
        assert_eq!(src.title_count(), 3);
        assert_eq!(src.title_at(0), Some("Don Quijote"));
        assert_eq!(src.title_at(2), Some("Moby Dick"));
    }

    #[test]
    fn select_resets_and_streams_words() {
        let mut src = FakeSdCatalogSource::new();
        src.select_text(1).unwrap();

        let first = src.next_word().unwrap().unwrap();
        assert_eq!(first.text, "Alice");
        assert_eq!(src.paragraph_progress(), (1, src.paragraph_progress().1));
    }
}
