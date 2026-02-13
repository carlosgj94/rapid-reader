use super::{
    ChapterInfo, NavigationCatalog, ParagraphNavigator, SelectableWordSource, TextCatalog,
    WordSource, WordToken,
    text_utils::{count_words, first_words_excerpt},
};
use heapless::{String, Vec};
use log::debug;

const PARAGRAPHS_PER_CHAPTER: usize = 2;
const CHAPTER_LABEL_WORDS: usize = 6;
const NO_CHAPTER_SEEK_TARGET: u16 = u16::MAX;
pub const SD_CATALOG_MAX_TITLES: usize = 16;
pub const SD_CATALOG_TITLE_BYTES: usize = 48;
pub const SD_CATALOG_TEXT_BYTES: usize = 480;
pub const SD_CATALOG_TEXT_PATH_BYTES: usize = 192;
const HTML_ENTITY_BYTES: usize = 16;
const HTML_TAIL_BYTES: usize = 96;
const HTML_FLAG_IN_HEAD: u8 = 1 << 0;
const HTML_FLAG_IN_BODY: u8 = 1 << 1;
const HTML_FLAG_BODY_SEEN: u8 = 1 << 2;
const HTML_FLAG_IN_SCRIPT: u8 = 1 << 3;
const HTML_FLAG_IN_STYLE: u8 = 1 << 4;

const EMPTY_FALLBACK_PARAGRAPHS: [&str; 0] = [];

#[derive(Clone, Copy)]
struct BookEntry {
    title: &'static str,
    paragraphs: &'static [&'static str],
}

const BOOKS: [BookEntry; 3] = [
    BookEntry {
        title: "Don Quijote",
        paragraphs: &EMPTY_FALLBACK_PARAGRAPHS,
    },
    BookEntry {
        title: "Alice in Wonderland",
        paragraphs: &EMPTY_FALLBACK_PARAGRAPHS,
    },
    BookEntry {
        title: "Moby Dick",
        paragraphs: &EMPTY_FALLBACK_PARAGRAPHS,
    },
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SdCatalogError {
    InvalidTextIndex,
    InvalidParagraphIndex,
    InvalidChapterIndex,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SdCatalogLoadResult {
    pub loaded: u16,
    pub truncated: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SdCatalogTextLoadResult {
    pub loaded: bool,
    pub truncated: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SdChunkRefillRequest {
    pub book_index: u16,
    pub target_chapter: Option<u16>,
}

/// SD-backed catalog/content source used by the reader runtime.
#[derive(Debug, Clone)]
pub struct SdCatalogSource {
    catalog_titles: Vec<String<SD_CATALOG_TITLE_BYTES>, SD_CATALOG_MAX_TITLES>,
    catalog_has_cover: Vec<bool, SD_CATALOG_MAX_TITLES>,
    catalog_text_chunks: Vec<String<SD_CATALOG_TEXT_BYTES>, SD_CATALOG_MAX_TITLES>,
    catalog_stream_mode: Vec<bool, SD_CATALOG_MAX_TITLES>,
    catalog_stream_end: Vec<bool, SD_CATALOG_MAX_TITLES>,
    catalog_stream_terminal: Vec<bool, SD_CATALOG_MAX_TITLES>,
    catalog_stream_chapter_index: Vec<u16, SD_CATALOG_MAX_TITLES>,
    catalog_stream_chapter_total_hint: Vec<u16, SD_CATALOG_MAX_TITLES>,
    catalog_stream_chapter_label: Vec<String<SD_CATALOG_TITLE_BYTES>, SD_CATALOG_MAX_TITLES>,
    catalog_html_state: Vec<HtmlParseState, SD_CATALOG_MAX_TITLES>,
    catalog_html_tail: Vec<Vec<u8, HTML_TAIL_BYTES>, SD_CATALOG_MAX_TITLES>,
    catalog_refill_requested: Vec<bool, SD_CATALOG_MAX_TITLES>,
    catalog_stream_seek_target: Vec<u16, SD_CATALOG_MAX_TITLES>,
    catalog_stream_paths: Vec<String<SD_CATALOG_TEXT_PATH_BYTES>, SD_CATALOG_MAX_TITLES>,
    waiting_for_refill: bool,
    selected_book: usize,
    paragraph_index: usize,
    paragraph_cursor: usize,
    paragraph_word_index: u16,
    paragraph_word_total: u16,
}

#[derive(Clone, Copy, Debug, Default)]
struct HtmlParseState {
    flags: u8,
}

impl Default for SdCatalogSource {
    fn default() -> Self {
        Self::new()
    }
}

mod catalog_setup;
mod catalog_stream;
mod html_entities;
mod parsing_utils;
mod sanitize_chunk;
mod source_state;
mod traits_catalog;
mod traits_navigation;
mod traits_word;

#[cfg(test)]
mod tests;
