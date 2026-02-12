use super::{
    ChapterInfo, NavigationCatalog, ParagraphNavigator, SelectableWordSource, TextCatalog,
    WordSource, WordToken,
    text_utils::{count_words, first_words_excerpt},
};
use crate::content::static_source::DON_QUIJOTE_PARAGRAPHS;
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

/// In-memory fake SD source used until card hardware integration is ready.
#[derive(Debug, Clone)]
pub struct FakeSdCatalogSource {
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

impl Default for FakeSdCatalogSource {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeSdCatalogSource {
    pub fn new() -> Self {
        let mut source = Self {
            catalog_titles: Vec::new(),
            catalog_has_cover: Vec::new(),
            catalog_text_chunks: Vec::new(),
            catalog_stream_mode: Vec::new(),
            catalog_stream_end: Vec::new(),
            catalog_stream_terminal: Vec::new(),
            catalog_stream_chapter_index: Vec::new(),
            catalog_stream_chapter_total_hint: Vec::new(),
            catalog_stream_chapter_label: Vec::new(),
            catalog_html_state: Vec::new(),
            catalog_html_tail: Vec::new(),
            catalog_refill_requested: Vec::new(),
            catalog_stream_seek_target: Vec::new(),
            catalog_stream_paths: Vec::new(),
            waiting_for_refill: false,
            selected_book: 0,
            paragraph_index: 0,
            paragraph_cursor: 0,
            paragraph_word_index: 0,
            paragraph_word_total: 1,
        };
        source.reset_catalog_titles_to_defaults();
        source.paragraph_word_total = source.compute_current_word_total();
        source
    }

    pub fn set_catalog_titles_from_iter<'a, I>(&mut self, titles: I) -> SdCatalogLoadResult
    where
        I: IntoIterator<Item = &'a str>,
    {
        self.catalog_titles.clear();
        self.catalog_has_cover.clear();
        self.catalog_text_chunks.clear();
        self.catalog_stream_mode.clear();
        self.catalog_stream_end.clear();
        self.catalog_stream_terminal.clear();
        self.catalog_stream_chapter_index.clear();
        self.catalog_stream_chapter_total_hint.clear();
        self.catalog_stream_chapter_label.clear();
        self.catalog_html_state.clear();
        self.catalog_html_tail.clear();
        self.catalog_refill_requested.clear();
        self.catalog_stream_seek_target.clear();
        self.catalog_stream_paths.clear();
        self.waiting_for_refill = false;

        let mut truncated = false;
        for title in titles {
            if self.catalog_titles.len() >= SD_CATALOG_MAX_TITLES {
                truncated = true;
                break;
            }

            if title.is_empty() {
                continue;
            }

            truncated |= self.push_catalog_title(title, false);
        }

        if self.selected_book >= self.catalog_titles.len() {
            self.selected_book = 0;
        }
        self.reset_read_pointer();

        SdCatalogLoadResult {
            loaded: self.catalog_titles.len().clamp(0, u16::MAX as usize) as u16,
            truncated,
        }
    }

    pub fn set_catalog_display_titles_from_iter<'a, I>(&mut self, titles: I) -> SdCatalogLoadResult
    where
        I: IntoIterator<Item = &'a str>,
    {
        self.catalog_titles.clear();
        self.catalog_has_cover.clear();
        self.catalog_text_chunks.clear();
        self.catalog_stream_mode.clear();
        self.catalog_stream_end.clear();
        self.catalog_stream_terminal.clear();
        self.catalog_stream_chapter_index.clear();
        self.catalog_stream_chapter_total_hint.clear();
        self.catalog_stream_chapter_label.clear();
        self.catalog_html_state.clear();
        self.catalog_html_tail.clear();
        self.catalog_refill_requested.clear();
        self.catalog_stream_seek_target.clear();
        self.catalog_stream_paths.clear();
        self.waiting_for_refill = false;

        let mut truncated = false;
        for title in titles {
            if self.catalog_titles.len() >= SD_CATALOG_MAX_TITLES {
                truncated = true;
                break;
            }

            if title.is_empty() {
                continue;
            }

            truncated |= self.push_catalog_title_verbatim(title, false);
        }

        if self.selected_book >= self.catalog_titles.len() {
            self.selected_book = 0;
        }
        self.reset_read_pointer();

        SdCatalogLoadResult {
            loaded: self.catalog_titles.len().clamp(0, u16::MAX as usize) as u16,
            truncated,
        }
    }

    pub fn set_catalog_entries_from_iter<'a, I>(&mut self, entries: I) -> SdCatalogLoadResult
    where
        I: IntoIterator<Item = (&'a str, bool)>,
    {
        self.catalog_titles.clear();
        self.catalog_has_cover.clear();
        self.catalog_text_chunks.clear();
        self.catalog_stream_mode.clear();
        self.catalog_stream_end.clear();
        self.catalog_stream_terminal.clear();
        self.catalog_stream_chapter_index.clear();
        self.catalog_stream_chapter_total_hint.clear();
        self.catalog_stream_chapter_label.clear();
        self.catalog_html_state.clear();
        self.catalog_html_tail.clear();
        self.catalog_refill_requested.clear();
        self.catalog_stream_seek_target.clear();
        self.catalog_stream_paths.clear();
        self.waiting_for_refill = false;

        let mut truncated = false;
        for (title, has_cover) in entries {
            if self.catalog_titles.len() >= SD_CATALOG_MAX_TITLES {
                truncated = true;
                break;
            }

            if title.is_empty() {
                continue;
            }

            truncated |= self.push_catalog_title_verbatim(title, has_cover);
        }

        if self.selected_book >= self.catalog_titles.len() {
            self.selected_book = 0;
        }
        self.reset_read_pointer();

        SdCatalogLoadResult {
            loaded: self.catalog_titles.len().clamp(0, u16::MAX as usize) as u16,
            truncated,
        }
    }

    pub fn set_catalog_text_chunk_from_bytes(
        &mut self,
        index: u16,
        chunk: &[u8],
        end_of_stream: bool,
        resource_path: &str,
    ) -> Result<SdCatalogTextLoadResult, SdStubError> {
        let idx = index as usize;
        let slot = self
            .catalog_text_chunks
            .get_mut(idx)
            .ok_or(SdStubError::InvalidTextIndex)?;
        let stream_mode = self
            .catalog_stream_mode
            .get_mut(idx)
            .ok_or(SdStubError::InvalidTextIndex)?;
        let stream_end = self
            .catalog_stream_end
            .get_mut(idx)
            .ok_or(SdStubError::InvalidTextIndex)?;
        let stream_terminal = self
            .catalog_stream_terminal
            .get_mut(idx)
            .ok_or(SdStubError::InvalidTextIndex)?;
        let chapter_index = self
            .catalog_stream_chapter_index
            .get_mut(idx)
            .ok_or(SdStubError::InvalidTextIndex)?;
        let chapter_total_hint = self
            .catalog_stream_chapter_total_hint
            .get_mut(idx)
            .ok_or(SdStubError::InvalidTextIndex)?;
        let chapter_label = self
            .catalog_stream_chapter_label
            .get_mut(idx)
            .ok_or(SdStubError::InvalidTextIndex)?;
        let html_state = self
            .catalog_html_state
            .get_mut(idx)
            .ok_or(SdStubError::InvalidTextIndex)?;
        let html_tail = self
            .catalog_html_tail
            .get_mut(idx)
            .ok_or(SdStubError::InvalidTextIndex)?;
        let refill_requested = self
            .catalog_refill_requested
            .get_mut(idx)
            .ok_or(SdStubError::InvalidTextIndex)?;
        let stream_path = self
            .catalog_stream_paths
            .get_mut(idx)
            .ok_or(SdStubError::InvalidTextIndex)?;
        let resource_changed = stream_path.as_str() != resource_path;
        let had_previous_resource = !stream_path.is_empty();
        if resource_changed {
            *html_state = HtmlParseState::default();
            html_tail.clear();
        }
        let treat_as_plain_text = path_is_plain_text(resource_path.as_bytes());
        let mut parse_input = [0u8; SD_CATALOG_TEXT_BYTES + HTML_TAIL_BYTES];
        let mut parse_len = 0usize;
        for &byte in html_tail.iter() {
            if parse_len >= parse_input.len() {
                break;
            }
            parse_input[parse_len] = byte;
            parse_len += 1;
        }
        for &byte in chunk {
            if parse_len >= parse_input.len() {
                break;
            }
            parse_input[parse_len] = byte;
            parse_len += 1;
        }

        let (sanitized, truncated, tail_start) =
            sanitize_epub_chunk(&parse_input[..parse_len], html_state, treat_as_plain_text);
        html_tail.clear();
        if let Some(start) = tail_start {
            for &byte in &parse_input[start..parse_len] {
                if html_tail.push(byte).is_err() {
                    break;
                }
            }
        }
        let loaded = !sanitized.is_empty();
        *slot = sanitized;
        *stream_mode = !resource_path.is_empty();
        *stream_end = end_of_stream;
        *stream_terminal = !*stream_mode;
        *refill_requested = false;
        if *stream_mode {
            if resource_changed {
                if had_previous_resource {
                    *chapter_index = chapter_index.saturating_add(1);
                }
                update_chapter_label_from_resource(resource_path, chapter_label);
            }
            *chapter_total_hint = (*chapter_total_hint).max(chapter_index.saturating_add(1));
        } else {
            *chapter_index = 0;
            *chapter_total_hint = 1;
            chapter_label.clear();
            let _ = chapter_label.push_str("Section");
        }
        stream_path.clear();
        for ch in resource_path.chars() {
            if stream_path.push(ch).is_err() {
                break;
            }
        }
        let parsed_paragraphs = Self::chunk_paragraph_count(slot.as_str());
        let first_paragraph = Self::chunk_paragraph_at(slot.as_str(), 0)
            .map(|paragraph| first_words_excerpt(paragraph, CHAPTER_LABEL_WORDS))
            .unwrap_or("");
        let last_paragraph = parsed_paragraphs
            .checked_sub(1)
            .and_then(|last| Self::chunk_paragraph_at(slot.as_str(), last))
            .map(|paragraph| first_words_excerpt(paragraph, CHAPTER_LABEL_WORDS))
            .unwrap_or("");
        let raw_preview_len = chunk.len().min(96);
        let raw_preview = core::str::from_utf8(&chunk[..raw_preview_len]).unwrap_or("");

        debug!(
            "sd-stream: chunk-set idx={} resource={} in_bytes={} loaded={} truncated={} stream_mode={} end={} terminal={} chapter={}/{} chapter_label={} html_flags=0x{:02x} tail_bytes={} parsed_paragraphs={} first_paragraph={:?} last_paragraph={:?} raw_preview={:?}",
            idx,
            stream_path.as_str(),
            chunk.len(),
            loaded,
            truncated,
            *stream_mode,
            *stream_end,
            *stream_terminal,
            chapter_index.saturating_add(1),
            *chapter_total_hint,
            chapter_label.as_str(),
            html_state.flags,
            html_tail.len(),
            parsed_paragraphs,
            first_paragraph,
            last_paragraph,
            raw_preview
        );
        let stream_mode_now = *stream_mode;
        let stream_end_now = *stream_end;
        let stream_terminal_now = *stream_terminal;
        let mut stream_path_log = String::<SD_CATALOG_TEXT_PATH_BYTES>::new();
        for ch in stream_path.chars() {
            if stream_path_log.push(ch).is_err() {
                break;
            }
        }
        if !loaded {
            debug!(
                "sd-stream: chunk-set yielded no text idx={} resource={} end={} terminal={} html_flags=0x{:02x} tail_bytes={} raw_bytes={}",
                idx,
                stream_path.as_str(),
                stream_end_now,
                stream_terminal_now,
                html_state.flags,
                html_tail.len(),
                chunk.len()
            );
        }

        if idx == self.selected_book {
            self.reset_read_pointer();
            if stream_mode_now && !stream_terminal_now && !loaded {
                if let Some(requested) = self.catalog_refill_requested.get_mut(idx) {
                    *requested = true;
                }
                self.waiting_for_refill = true;
                debug!(
                    "sd-stream: chunk-set auto-refill idx={} resource={} reason=no_parsed_text end={} terminal={}",
                    idx,
                    stream_path_log.as_str(),
                    stream_end_now,
                    stream_terminal_now
                );
            } else {
                self.waiting_for_refill = false;
            }
        }

        Ok(SdCatalogTextLoadResult { loaded, truncated })
    }

    pub fn mark_catalog_stream_exhausted(&mut self, index: u16) -> Result<(), SdStubError> {
        let idx = index as usize;
        let Some(stream_end) = self.catalog_stream_end.get_mut(idx) else {
            return Err(SdStubError::InvalidTextIndex);
        };
        let Some(refill_requested) = self.catalog_refill_requested.get_mut(idx) else {
            return Err(SdStubError::InvalidTextIndex);
        };
        let Some(stream_terminal) = self.catalog_stream_terminal.get_mut(idx) else {
            return Err(SdStubError::InvalidTextIndex);
        };
        let Some(seek_target) = self.catalog_stream_seek_target.get_mut(idx) else {
            return Err(SdStubError::InvalidTextIndex);
        };
        let Some(chapter_index) = self.catalog_stream_chapter_index.get(idx).copied() else {
            return Err(SdStubError::InvalidTextIndex);
        };
        let Some(chapter_total_hint) = self.catalog_stream_chapter_total_hint.get_mut(idx) else {
            return Err(SdStubError::InvalidTextIndex);
        };
        let Some(html_tail) = self.catalog_html_tail.get_mut(idx) else {
            return Err(SdStubError::InvalidTextIndex);
        };

        *stream_end = true;
        *stream_terminal = true;
        *chapter_total_hint = (*chapter_total_hint).max(chapter_index.saturating_add(1));
        *refill_requested = false;
        *seek_target = NO_CHAPTER_SEEK_TARGET;
        html_tail.clear();
        if idx == self.selected_book {
            self.waiting_for_refill = false;
        }
        debug!(
            "sd-stream: stream-exhausted idx={} selected={} path={} chapter={}/{} terminal={}",
            idx,
            self.selected_book,
            self.catalog_stream_paths
                .get(idx)
                .map(|path| path.as_str())
                .unwrap_or(""),
            chapter_index.saturating_add(1),
            *chapter_total_hint,
            *stream_terminal
        );
        Ok(())
    }

    pub fn take_chunk_refill_request(&mut self) -> Option<SdChunkRefillRequest> {
        let idx = self.selected_book;
        if !self.catalog_stream_mode.get(idx).copied().unwrap_or(false) {
            return None;
        }

        let requested = self.catalog_refill_requested.get_mut(idx)?;
        if !*requested {
            return None;
        }

        *requested = false;
        let chapter_index = self
            .catalog_stream_chapter_index
            .get(idx)
            .copied()
            .unwrap_or(0);
        let chapter_total_hint = self
            .catalog_stream_chapter_total_hint
            .get(idx)
            .copied()
            .unwrap_or(1);
        let seek_target = self
            .catalog_stream_seek_target
            .get(idx)
            .copied()
            .filter(|target| *target != NO_CHAPTER_SEEK_TARGET);
        if let Some(slot) = self.catalog_stream_seek_target.get_mut(idx) {
            *slot = NO_CHAPTER_SEEK_TARGET;
        }
        debug!(
            "sd-stream: refill-request idx={} path={} chapter={}/{} seek_target={:?} end={} terminal={} waiting_for_refill={} paragraph_index={} paragraph_total={} paragraph_word_index={} paragraph_word_total={}",
            idx,
            self.catalog_stream_paths
                .get(idx)
                .map(|path| path.as_str())
                .unwrap_or(""),
            chapter_index.saturating_add(1),
            chapter_total_hint,
            seek_target.map(|target| target.saturating_add(1)),
            self.catalog_stream_end.get(idx).copied().unwrap_or(true),
            self.catalog_stream_terminal
                .get(idx)
                .copied()
                .unwrap_or(true),
            self.waiting_for_refill,
            self.paragraph_index.saturating_add(1),
            self.selected_paragraph_count(),
            self.paragraph_word_index,
            self.paragraph_word_total
        );
        Some(SdChunkRefillRequest {
            book_index: idx as u16,
            target_chapter: seek_target,
        })
    }

    pub fn stream_resource_path(&self, index: u16) -> Option<&str> {
        self.catalog_stream_paths
            .get(index as usize)
            .map(|path| path.as_str())
            .filter(|path| !path.is_empty())
    }

    pub fn set_catalog_stream_chapter_hint(
        &mut self,
        index: u16,
        chapter_index: u16,
        chapter_total: u16,
    ) -> Result<(), SdStubError> {
        let idx = index as usize;
        let Some(chapter_idx_slot) = self.catalog_stream_chapter_index.get_mut(idx) else {
            return Err(SdStubError::InvalidTextIndex);
        };
        let Some(chapter_total_slot) = self.catalog_stream_chapter_total_hint.get_mut(idx) else {
            return Err(SdStubError::InvalidTextIndex);
        };

        let total = chapter_total.max(1);
        *chapter_total_slot = total;
        *chapter_idx_slot = chapter_index.min(total.saturating_sub(1));
        debug!(
            "sd-stream: chapter-hint idx={} chapter={}/{}",
            idx,
            chapter_idx_slot.saturating_add(1),
            *chapter_total_slot
        );
        Ok(())
    }

    fn reset_catalog_titles_to_defaults(&mut self) {
        self.catalog_titles.clear();
        self.catalog_has_cover.clear();
        self.catalog_text_chunks.clear();
        self.catalog_stream_mode.clear();
        self.catalog_stream_end.clear();
        self.catalog_stream_terminal.clear();
        self.catalog_stream_chapter_index.clear();
        self.catalog_stream_chapter_total_hint.clear();
        self.catalog_stream_chapter_label.clear();
        self.catalog_html_state.clear();
        self.catalog_html_tail.clear();
        self.catalog_refill_requested.clear();
        self.catalog_stream_seek_target.clear();
        self.catalog_stream_paths.clear();
        self.waiting_for_refill = false;
        for entry in BOOKS {
            let _ = self.push_catalog_title_verbatim(entry.title, false);
        }
    }

    fn push_catalog_title_verbatim(&mut self, title: &str, has_cover: bool) -> bool {
        let mut label = String::<SD_CATALOG_TITLE_BYTES>::new();
        let mut truncated = false;
        for ch in title.chars() {
            if label.push(ch).is_err() {
                truncated = true;
                break;
            }
        }

        if self.catalog_titles.push(label).is_err() {
            return true;
        }
        if self.catalog_has_cover.push(has_cover).is_err() {
            return true;
        }
        if self
            .catalog_text_chunks
            .push(String::<SD_CATALOG_TEXT_BYTES>::new())
            .is_err()
        {
            return true;
        }
        if self.catalog_stream_mode.push(false).is_err() {
            return true;
        }
        if self.catalog_stream_end.push(true).is_err() {
            return true;
        }
        if self.catalog_stream_terminal.push(true).is_err() {
            return true;
        }
        if self.catalog_stream_chapter_index.push(0).is_err() {
            return true;
        }
        if self.catalog_stream_chapter_total_hint.push(1).is_err() {
            return true;
        }
        let mut chapter_label = String::<SD_CATALOG_TITLE_BYTES>::new();
        let _ = chapter_label.push_str("Section");
        if self
            .catalog_stream_chapter_label
            .push(chapter_label)
            .is_err()
        {
            return true;
        }
        if self
            .catalog_html_state
            .push(HtmlParseState::default())
            .is_err()
        {
            return true;
        }
        if self
            .catalog_html_tail
            .push(Vec::<u8, HTML_TAIL_BYTES>::new())
            .is_err()
        {
            return true;
        }
        if self.catalog_refill_requested.push(false).is_err() {
            return true;
        }
        if self
            .catalog_stream_seek_target
            .push(NO_CHAPTER_SEEK_TARGET)
            .is_err()
        {
            return true;
        }
        if self
            .catalog_stream_paths
            .push(String::<SD_CATALOG_TEXT_PATH_BYTES>::new())
            .is_err()
        {
            return true;
        }

        truncated
    }

    fn push_catalog_title(&mut self, title: &str, has_cover: bool) -> bool {
        let mut label = String::<SD_CATALOG_TITLE_BYTES>::new();
        let mut truncated = false;

        let stem = title
            .rsplit_once('.')
            .map(|(left, _)| left)
            .unwrap_or(title)
            .trim();

        let mut word_start = true;
        let mut wrote_any = false;
        for byte in stem.as_bytes() {
            let mut out = *byte;

            if out == b'_' || out == b'-' {
                out = b' ';
            }

            if out == b' ' {
                if !wrote_any || word_start {
                    continue;
                }
                if label.push(' ').is_err() {
                    truncated = true;
                    break;
                }
                word_start = true;
                continue;
            }

            let ch = if out.is_ascii_alphabetic() {
                if word_start {
                    (out as char).to_ascii_uppercase()
                } else {
                    (out as char).to_ascii_lowercase()
                }
            } else {
                out as char
            };

            if label.push(ch).is_err() {
                truncated = true;
                break;
            }
            wrote_any = true;
            word_start = false;
        }

        if label.is_empty() {
            return self.push_catalog_title_verbatim(title, has_cover);
        }

        if self.catalog_titles.push(label).is_err() {
            return true;
        }
        if self.catalog_has_cover.push(has_cover).is_err() {
            return true;
        }
        if self
            .catalog_text_chunks
            .push(String::<SD_CATALOG_TEXT_BYTES>::new())
            .is_err()
        {
            return true;
        }
        if self.catalog_stream_mode.push(false).is_err() {
            return true;
        }
        if self.catalog_stream_end.push(true).is_err() {
            return true;
        }
        if self.catalog_stream_terminal.push(true).is_err() {
            return true;
        }
        if self.catalog_stream_chapter_index.push(0).is_err() {
            return true;
        }
        if self.catalog_stream_chapter_total_hint.push(1).is_err() {
            return true;
        }
        let mut chapter_label = String::<SD_CATALOG_TITLE_BYTES>::new();
        let _ = chapter_label.push_str("Section");
        if self
            .catalog_stream_chapter_label
            .push(chapter_label)
            .is_err()
        {
            return true;
        }
        if self
            .catalog_html_state
            .push(HtmlParseState::default())
            .is_err()
        {
            return true;
        }
        if self
            .catalog_html_tail
            .push(Vec::<u8, HTML_TAIL_BYTES>::new())
            .is_err()
        {
            return true;
        }
        if self.catalog_refill_requested.push(false).is_err() {
            return true;
        }
        if self
            .catalog_stream_seek_target
            .push(NO_CHAPTER_SEEK_TARGET)
            .is_err()
        {
            return true;
        }
        if self
            .catalog_stream_paths
            .push(String::<SD_CATALOG_TEXT_PATH_BYTES>::new())
            .is_err()
        {
            return true;
        }

        truncated
    }

    fn selected_fallback_paragraphs(&self) -> &'static [&'static str] {
        if BOOKS.is_empty() {
            &[]
        } else {
            BOOKS[self.selected_book % BOOKS.len()].paragraphs
        }
    }

    fn selected_chunk_text(&self) -> Option<&str> {
        self.catalog_text_chunks
            .get(self.selected_book)
            .map(|text| text.as_str())
            .map(str::trim)
            .filter(|text| !text.is_empty())
    }

    fn chunk_paragraph_count(chunk: &str) -> usize {
        chunk
            .split('\n')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .count()
    }

    fn chunk_paragraph_at(chunk: &str, index: usize) -> Option<&str> {
        chunk
            .split('\n')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .nth(index)
    }

    fn selected_is_stream_mode(&self) -> bool {
        self.catalog_stream_mode
            .get(self.selected_book)
            .copied()
            .unwrap_or(false)
    }

    fn selected_stream_chapter_index(&self) -> u16 {
        self.catalog_stream_chapter_index
            .get(self.selected_book)
            .copied()
            .unwrap_or(0)
    }

    fn selected_stream_chapter_total_hint(&self) -> u16 {
        self.catalog_stream_chapter_total_hint
            .get(self.selected_book)
            .copied()
            .unwrap_or(1)
            .max(1)
    }

    fn selected_stream_chapter_label(&self) -> &str {
        self.catalog_stream_chapter_label
            .get(self.selected_book)
            .map(|label| label.as_str())
            .filter(|label| !label.trim().is_empty())
            .unwrap_or("Section")
    }

    fn selected_stream_path(&self) -> &str {
        self.catalog_stream_paths
            .get(self.selected_book)
            .map(|path| path.as_str())
            .unwrap_or("")
    }

    fn selected_paragraph_count(&self) -> usize {
        if let Some(chunk_text) = self.selected_chunk_text() {
            return Self::chunk_paragraph_count(chunk_text);
        }

        if self.selected_is_stream_mode() {
            0
        } else {
            self.selected_fallback_paragraphs().len()
        }
    }

    fn selected_paragraph_at(&self, index: usize) -> Option<&str> {
        if let Some(chunk_text) = self.selected_chunk_text() {
            return Self::chunk_paragraph_at(chunk_text, index);
        }

        if self.selected_is_stream_mode() {
            return None;
        }

        self.selected_fallback_paragraphs().get(index).copied()
    }

    fn compute_current_word_total(&self) -> u16 {
        let Some(paragraph) = self.selected_paragraph_at(self.paragraph_index) else {
            return 1;
        };

        let count = count_words(paragraph);
        count.clamp(1, u16::MAX as usize) as u16
    }

    fn reset_read_pointer(&mut self) {
        self.paragraph_index = 0;
        self.paragraph_cursor = 0;
        self.paragraph_word_index = 0;
        self.paragraph_word_total = self.compute_current_word_total();
        let paragraph_total = self.selected_paragraph_count();
        let preview = self
            .selected_paragraph_at(self.paragraph_index)
            .map(|paragraph| first_words_excerpt(paragraph, CHAPTER_LABEL_WORDS))
            .unwrap_or("");
        debug!(
            "sd-stream: reset-pointer selected_book={} stream_mode={} chapter={}/{} path={} paragraph_total={} paragraph_index={} paragraph_word_total={} first_paragraph={:?}",
            self.selected_book,
            self.selected_is_stream_mode(),
            self.selected_stream_chapter_index().saturating_add(1),
            self.selected_stream_chapter_total_hint(),
            self.selected_stream_path(),
            paragraph_total,
            self.paragraph_index.saturating_add(1),
            self.paragraph_word_total,
            preview
        );
    }

    fn advance_paragraph(&mut self) -> bool {
        let paragraph_count = self.selected_paragraph_count();
        if paragraph_count == 0 {
            debug!(
                "sd-stream: advance-paragraph blocked selected_book={} reason=no_paragraphs stream_mode={} chapter={}/{} path={}",
                self.selected_book,
                self.selected_is_stream_mode(),
                self.selected_stream_chapter_index().saturating_add(1),
                self.selected_stream_chapter_total_hint(),
                self.selected_stream_path()
            );
            return false;
        }

        if self.paragraph_index + 1 >= paragraph_count {
            debug!(
                "sd-stream: advance-paragraph blocked selected_book={} reason=end_of_chunk paragraph_index={}/{} chapter={}/{} path={}",
                self.selected_book,
                self.paragraph_index.saturating_add(1),
                paragraph_count,
                self.selected_stream_chapter_index().saturating_add(1),
                self.selected_stream_chapter_total_hint(),
                self.selected_stream_path()
            );
            return false;
        }

        let from = self.paragraph_index;
        self.paragraph_index += 1;
        self.paragraph_cursor = 0;
        self.paragraph_word_index = 0;
        self.paragraph_word_total = self.compute_current_word_total();
        let preview = self
            .selected_paragraph_at(self.paragraph_index)
            .map(|paragraph| first_words_excerpt(paragraph, CHAPTER_LABEL_WORDS))
            .unwrap_or("");
        debug!(
            "sd-stream: advance-paragraph selected_book={} from={}/{} to={}/{} chapter={}/{} path={} preview={:?}",
            self.selected_book,
            from.saturating_add(1),
            paragraph_count,
            self.paragraph_index.saturating_add(1),
            paragraph_count,
            self.selected_stream_chapter_index().saturating_add(1),
            self.selected_stream_chapter_total_hint(),
            self.selected_stream_path(),
            preview
        );
        true
    }
}

impl TextCatalog for FakeSdCatalogSource {
    fn title_count(&self) -> u16 {
        self.catalog_titles.len().clamp(0, u16::MAX as usize) as u16
    }

    fn title_at(&self, index: u16) -> Option<&str> {
        self.catalog_titles
            .get(index as usize)
            .map(|title| title.as_str())
    }

    fn has_cover_at(&self, index: u16) -> bool {
        self.catalog_has_cover
            .get(index as usize)
            .copied()
            .unwrap_or(false)
    }
}

impl WordSource for FakeSdCatalogSource {
    type Error = SdStubError;

    fn reset(&mut self) -> Result<(), Self::Error> {
        self.waiting_for_refill = false;
        self.reset_read_pointer();
        Ok(())
    }

    fn next_word<'a>(&'a mut self) -> Result<Option<WordToken<'a>>, Self::Error> {
        if self.selected_paragraph_count() == 0 {
            if self.selected_is_stream_mode()
                && !self
                    .catalog_stream_terminal
                    .get(self.selected_book)
                    .copied()
                    .unwrap_or(true)
            {
                self.waiting_for_refill = true;
                if let Some(requested) = self.catalog_refill_requested.get_mut(self.selected_book) {
                    *requested = true;
                }
                debug!(
                    "sd-stream: next-word empty -> request refill idx={} path={} end={} terminal={}",
                    self.selected_book,
                    self.catalog_stream_paths
                        .get(self.selected_book)
                        .map(|path| path.as_str())
                        .unwrap_or(""),
                    self.catalog_stream_end
                        .get(self.selected_book)
                        .copied()
                        .unwrap_or(true),
                    self.catalog_stream_terminal
                        .get(self.selected_book)
                        .copied()
                        .unwrap_or(true)
                );
            }
            return Ok(None);
        }

        loop {
            let paragraph_index = self.paragraph_index;
            let paragraph_cursor = self.paragraph_cursor;
            let next = {
                let Some(paragraph) = self.selected_paragraph_at(paragraph_index) else {
                    return Ok(None);
                };
                next_word_bounds(paragraph, paragraph_cursor)
            };

            if let Some((start, end, next_cursor)) = next {
                self.paragraph_cursor = next_cursor;
                self.paragraph_word_index = self.paragraph_word_index.saturating_add(1);
                let Some(paragraph) = self.selected_paragraph_at(paragraph_index) else {
                    return Ok(None);
                };
                let word = &paragraph[start..end];

                let ends_sentence =
                    word.ends_with('.') || word.ends_with('!') || word.ends_with('?');
                let ends_clause = word.ends_with(',');
                if self.paragraph_word_index == 1 {
                    debug!(
                        "sd-stream: paragraph-enter idx={} paragraph={}/{} chapter={}/{} path={} preview={:?}",
                        self.selected_book,
                        paragraph_index.saturating_add(1),
                        self.selected_paragraph_count(),
                        self.selected_stream_chapter_index().saturating_add(1),
                        self.selected_stream_chapter_total_hint(),
                        self.selected_stream_path(),
                        first_words_excerpt(paragraph, CHAPTER_LABEL_WORDS)
                    );
                }

                return Ok(Some(WordToken {
                    text: word,
                    ends_sentence,
                    ends_clause,
                }));
            }

            if !self.advance_paragraph() {
                if self.selected_is_stream_mode()
                    && !self
                        .catalog_stream_terminal
                        .get(self.selected_book)
                        .copied()
                        .unwrap_or(true)
                {
                    self.waiting_for_refill = true;
                    if let Some(requested) =
                        self.catalog_refill_requested.get_mut(self.selected_book)
                    {
                        *requested = true;
                    }
                    if let Some(chunk) = self.catalog_text_chunks.get_mut(self.selected_book) {
                        chunk.clear();
                    }
                    self.paragraph_cursor = 0;
                    self.paragraph_word_index = 0;
                    self.paragraph_word_total = 1;
                    debug!(
                        "sd-stream: paragraph exhausted -> request refill idx={} path={} end={} terminal={}",
                        self.selected_book,
                        self.catalog_stream_paths
                            .get(self.selected_book)
                            .map(|path| path.as_str())
                            .unwrap_or(""),
                        self.catalog_stream_end
                            .get(self.selected_book)
                            .copied()
                            .unwrap_or(true),
                        self.catalog_stream_terminal
                            .get(self.selected_book)
                            .copied()
                            .unwrap_or(true)
                    );
                    return Ok(None);
                }
                if self.selected_is_stream_mode() {
                    debug!(
                        "sd-stream: next-word reached terminal end idx={} path={} end={} terminal={}",
                        self.selected_book,
                        self.catalog_stream_paths
                            .get(self.selected_book)
                            .map(|path| path.as_str())
                            .unwrap_or(""),
                        self.catalog_stream_end
                            .get(self.selected_book)
                            .copied()
                            .unwrap_or(true),
                        self.catalog_stream_terminal
                            .get(self.selected_book)
                            .copied()
                            .unwrap_or(true)
                    );
                }
                return Ok(None);
            }
        }
    }

    fn paragraph_progress(&self) -> (u16, u16) {
        (self.paragraph_word_index, self.paragraph_word_total.max(1))
    }

    fn paragraph_index(&self) -> u16 {
        if self.selected_paragraph_count() == 0 {
            0
        } else {
            (self.paragraph_index + 1) as u16
        }
    }

    fn paragraph_total(&self) -> u16 {
        self.selected_paragraph_count().clamp(0, u16::MAX as usize) as u16
    }

    fn is_waiting_for_refill(&self) -> bool {
        self.waiting_for_refill
    }
}

impl SelectableWordSource for FakeSdCatalogSource {
    fn select_text(&mut self, index: u16) -> Result<(), Self::Error> {
        let idx = index as usize;
        if idx >= self.catalog_titles.len() {
            return Err(SdStubError::InvalidTextIndex);
        }

        self.selected_book = idx;
        self.waiting_for_refill = false;
        if let Some(requested) = self.catalog_refill_requested.get_mut(idx) {
            *requested = false;
        }
        if let Some(seek_target) = self.catalog_stream_seek_target.get_mut(idx) {
            *seek_target = NO_CHAPTER_SEEK_TARGET;
        }
        self.reset_read_pointer();
        debug!(
            "sd-nav: select_text selected_book={} title={:?} stream_mode={} chapter={}/{} path={}",
            self.selected_book,
            self.catalog_titles
                .get(self.selected_book)
                .map(|title| title.as_str())
                .unwrap_or(""),
            self.selected_is_stream_mode(),
            self.selected_stream_chapter_index().saturating_add(1),
            self.selected_stream_chapter_total_hint(),
            self.selected_stream_path()
        );
        Ok(())
    }

    fn selected_index(&self) -> u16 {
        self.selected_book as u16
    }
}

impl ParagraphNavigator for FakeSdCatalogSource {
    fn seek_paragraph(&mut self, paragraph_index: u16) -> Result<(), Self::Error> {
        let paragraph_total = self.selected_paragraph_count();
        debug!(
            "sd-nav: seek_paragraph selected_book={} requested={} paragraph_total={} stream_mode={} chapter={}/{} path={}",
            self.selected_book,
            paragraph_index,
            paragraph_total,
            self.selected_is_stream_mode(),
            self.selected_stream_chapter_index().saturating_add(1),
            self.selected_stream_chapter_total_hint(),
            self.catalog_stream_paths
                .get(self.selected_book)
                .map(|p| p.as_str())
                .unwrap_or("")
        );
        if paragraph_total == 0 {
            self.paragraph_index = 0;
            self.paragraph_cursor = 0;
            self.paragraph_word_index = 0;
            self.paragraph_word_total = 1;
            return Ok(());
        }

        let index = paragraph_index as usize;
        if index >= paragraph_total {
            return Err(SdStubError::InvalidParagraphIndex);
        }

        self.paragraph_index = index;
        self.paragraph_cursor = 0;
        self.paragraph_word_index = 0;
        self.paragraph_word_total = self.compute_current_word_total();
        self.waiting_for_refill = false;
        let preview = self
            .selected_paragraph_at(index)
            .map(|paragraph| first_words_excerpt(paragraph, CHAPTER_LABEL_WORDS))
            .unwrap_or("");
        debug!(
            "sd-nav: seek_paragraph applied selected_book={} paragraph={}/{} word_total={} chapter={}/{} path={} preview={:?}",
            self.selected_book,
            self.paragraph_index.saturating_add(1),
            paragraph_total,
            self.paragraph_word_total,
            self.selected_stream_chapter_index().saturating_add(1),
            self.selected_stream_chapter_total_hint(),
            self.selected_stream_path(),
            preview
        );
        Ok(())
    }
}

impl NavigationCatalog for FakeSdCatalogSource {
    fn chapter_count(&self) -> u16 {
        if self.selected_is_stream_mode() {
            return self.selected_stream_chapter_total_hint();
        }

        let len = self.selected_paragraph_count();
        if len == 0 {
            return 1;
        }

        len.div_ceil(PARAGRAPHS_PER_CHAPTER)
            .clamp(1, u16::MAX as usize) as u16
    }

    fn chapter_at(&self, index: u16) -> Option<ChapterInfo<'_>> {
        if self.selected_is_stream_mode() {
            let total = self.selected_stream_chapter_total_hint();
            if index >= total {
                debug!(
                    "sd-nav: chapter_at stream selected_book={} requested={}/{} out_of_range current={}/{} path={}",
                    self.selected_book,
                    index.saturating_add(1),
                    total,
                    self.selected_stream_chapter_index().saturating_add(1),
                    total,
                    self.catalog_stream_paths
                        .get(self.selected_book)
                        .map(|p| p.as_str())
                        .unwrap_or("")
                );
                return None;
            }

            let current = self
                .selected_stream_chapter_index()
                .min(total.saturating_sub(1));
            if index == current {
                let paragraph_count = self.selected_paragraph_count().max(1);
                return Some(ChapterInfo {
                    label: self.selected_stream_chapter_label(),
                    start_paragraph: 0,
                    paragraph_count: paragraph_count.clamp(1, u16::MAX as usize) as u16,
                });
            }

            return Some(ChapterInfo {
                label: "Chapter",
                start_paragraph: 0,
                paragraph_count: 1,
            });
        }

        let paragraph_total = self.selected_paragraph_count();
        if paragraph_total == 0 {
            return Some(ChapterInfo {
                label: "Empty",
                start_paragraph: 0,
                paragraph_count: 1,
            });
        }

        let chapter_index = index as usize;
        let chapter_count = paragraph_total.div_ceil(PARAGRAPHS_PER_CHAPTER);
        if chapter_index >= chapter_count {
            return None;
        }

        let start = chapter_index * PARAGRAPHS_PER_CHAPTER;
        let remaining = paragraph_total.saturating_sub(start);
        let count = remaining.min(PARAGRAPHS_PER_CHAPTER);
        let label = self
            .selected_paragraph_at(start)
            .map(|paragraph| first_words_excerpt(paragraph, CHAPTER_LABEL_WORDS))
            .unwrap_or("Chapter");

        Some(ChapterInfo {
            label,
            start_paragraph: start as u16,
            paragraph_count: count as u16,
        })
    }

    fn current_chapter_index(&self) -> Option<u16> {
        if self.selected_is_stream_mode() {
            let total = self.selected_stream_chapter_total_hint();
            return Some(
                self.selected_stream_chapter_index()
                    .min(total.saturating_sub(1)),
            );
        }
        None
    }

    fn seek_chapter(&mut self, chapter_index: u16) -> Result<bool, Self::Error> {
        if !self.selected_is_stream_mode() {
            return Ok(false);
        }

        let total = self.selected_stream_chapter_total_hint();
        if chapter_index >= total {
            return Err(SdStubError::InvalidChapterIndex);
        }

        let current = self
            .selected_stream_chapter_index()
            .min(total.saturating_sub(1));
        debug!(
            "sd-nav: seek_chapter request selected_book={} requested={}/{} current={}/{} path={} waiting_for_refill={}",
            self.selected_book,
            chapter_index.saturating_add(1),
            total,
            current.saturating_add(1),
            total,
            self.selected_stream_path(),
            self.waiting_for_refill
        );

        if chapter_index == current {
            self.waiting_for_refill = false;
            if let Some(refill_requested) =
                self.catalog_refill_requested.get_mut(self.selected_book)
            {
                *refill_requested = false;
            }
            if let Some(seek_target) = self.catalog_stream_seek_target.get_mut(self.selected_book) {
                *seek_target = NO_CHAPTER_SEEK_TARGET;
            }
            self.reset_read_pointer();
            debug!(
                "sd-nav: seek_chapter no-op selected_book={} chapter={}/{} path={}",
                self.selected_book,
                chapter_index.saturating_add(1),
                total,
                self.selected_stream_path()
            );
            return Ok(true);
        }

        let Some(chapter_slot) = self
            .catalog_stream_chapter_index
            .get_mut(self.selected_book)
        else {
            return Err(SdStubError::InvalidTextIndex);
        };
        *chapter_slot = chapter_index;

        if let Some(chapter_label) = self
            .catalog_stream_chapter_label
            .get_mut(self.selected_book)
        {
            chapter_label.clear();
            let _ = chapter_label.push_str("Chapter ");
            push_u32_as_ascii(chapter_label, chapter_index.saturating_add(1) as u32);
        }
        if let Some(chunk) = self.catalog_text_chunks.get_mut(self.selected_book) {
            chunk.clear();
        }
        if let Some(html_state) = self.catalog_html_state.get_mut(self.selected_book) {
            *html_state = HtmlParseState::default();
        }
        if let Some(html_tail) = self.catalog_html_tail.get_mut(self.selected_book) {
            html_tail.clear();
        }
        if let Some(stream_end) = self.catalog_stream_end.get_mut(self.selected_book) {
            *stream_end = false;
        }
        if let Some(stream_terminal) = self.catalog_stream_terminal.get_mut(self.selected_book) {
            *stream_terminal = false;
        }
        if let Some(refill_requested) = self.catalog_refill_requested.get_mut(self.selected_book) {
            *refill_requested = true;
        }
        if let Some(seek_target) = self.catalog_stream_seek_target.get_mut(self.selected_book) {
            *seek_target = chapter_index;
        }
        self.waiting_for_refill = true;
        self.paragraph_index = 0;
        self.paragraph_cursor = 0;
        self.paragraph_word_index = 0;
        self.paragraph_word_total = 1;
        debug!(
            "sd-nav: seek_chapter queued selected_book={} target={}/{} path={} paragraph_total={} waiting_for_refill={}",
            self.selected_book,
            chapter_index.saturating_add(1),
            total,
            self.selected_stream_path(),
            self.selected_paragraph_count(),
            self.waiting_for_refill
        );
        Ok(true)
    }

    fn paragraph_preview(&self, paragraph_index: u16) -> Option<&str> {
        self.selected_paragraph_at(paragraph_index as usize)
    }
}

fn next_word_bounds(text: &str, mut cursor: usize) -> Option<(usize, usize, usize)> {
    let bytes = text.as_bytes();
    let len = bytes.len();

    while cursor < len && bytes[cursor].is_ascii_whitespace() {
        cursor += 1;
    }
    if cursor >= len {
        return None;
    }

    let start = cursor;
    while cursor < len && !bytes[cursor].is_ascii_whitespace() {
        cursor += 1;
    }

    Some((start, cursor, cursor))
}

impl HtmlParseState {
    fn has(self, flag: u8) -> bool {
        self.flags & flag != 0
    }

    fn set(&mut self, flag: u8, enabled: bool) {
        if enabled {
            self.flags |= flag;
        } else {
            self.flags &= !flag;
        }
    }

    fn should_emit_text(self, treat_as_plain_text: bool) -> bool {
        if self.has(HTML_FLAG_IN_SCRIPT) || self.has(HTML_FLAG_IN_STYLE) {
            return false;
        }

        if treat_as_plain_text {
            return true;
        }

        if self.has(HTML_FLAG_BODY_SEEN) {
            return self.has(HTML_FLAG_IN_BODY);
        }

        !self.has(HTML_FLAG_IN_HEAD)
    }
}

fn eq_ascii_case_insensitive(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right.iter())
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
}

fn find_ascii_case_insensitive(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from >= haystack.len() || needle.len() > haystack.len() {
        return None;
    }
    let max_start = haystack.len().saturating_sub(needle.len());
    if from > max_start {
        return None;
    }

    (from..=max_start)
        .find(|&idx| eq_ascii_case_insensitive(&haystack[idx..idx + needle.len()], needle))
}

fn contains_ascii_case_insensitive(haystack: &[u8], needle: &[u8]) -> bool {
    find_ascii_case_insensitive(haystack, needle, 0).is_some()
}

fn trim_ascii(slice: &[u8]) -> &[u8] {
    let mut start = 0usize;
    let mut end = slice.len();
    while start < end && slice[start].is_ascii_whitespace() {
        start += 1;
    }
    while end > start && slice[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    &slice[start..end]
}

fn path_is_plain_text(path: &[u8]) -> bool {
    ends_with_ascii_case_insensitive(path, b".txt")
        || ends_with_ascii_case_insensitive(path, b".text")
}

fn ends_with_ascii_case_insensitive(path: &[u8], suffix: &[u8]) -> bool {
    if suffix.len() > path.len() {
        return false;
    }
    eq_ascii_case_insensitive(&path[path.len() - suffix.len()..], suffix)
}

#[derive(Clone, Copy)]
struct HtmlTagInfo<'a> {
    local_name: &'a [u8],
    is_closing: bool,
    is_self_closing: bool,
}

fn parse_html_tag_info(tag: &[u8]) -> Option<HtmlTagInfo<'_>> {
    let tag = trim_ascii(tag);
    if tag.is_empty() {
        return None;
    }

    if tag.starts_with(b"!--") || tag.starts_with(b"!") || tag.starts_with(b"?") {
        return None;
    }

    let (is_closing, name_start) = if tag[0] == b'/' {
        (true, 1usize)
    } else {
        (false, 0usize)
    };
    let rest = trim_ascii(&tag[name_start..]);
    if rest.is_empty() {
        return None;
    }

    let is_self_closing = rest.ends_with(b"/");
    let mut name_end = 0usize;
    while name_end < rest.len()
        && !rest[name_end].is_ascii_whitespace()
        && rest[name_end] != b'/'
        && rest[name_end] != b'>'
    {
        name_end += 1;
    }
    if name_end == 0 {
        return None;
    }

    let name = &rest[..name_end];
    let local_name = name
        .iter()
        .rposition(|b| *b == b':')
        .map(|idx| &name[idx + 1..])
        .unwrap_or(name);

    Some(HtmlTagInfo {
        local_name,
        is_closing,
        is_self_closing,
    })
}

fn apply_html_tag_state(tag: &[u8], state: &mut HtmlParseState) {
    let Some(tag_info) = parse_html_tag_info(tag) else {
        return;
    };
    let local_name = tag_info.local_name;
    let is_closing = tag_info.is_closing;
    let is_self_closing = tag_info.is_self_closing;

    if eq_ascii_case_insensitive(local_name, b"head") {
        state.set(HTML_FLAG_IN_HEAD, !is_closing && !is_self_closing);
        return;
    }

    if eq_ascii_case_insensitive(local_name, b"body") {
        if is_closing {
            state.set(HTML_FLAG_IN_BODY, false);
        } else {
            state.set(HTML_FLAG_BODY_SEEN, true);
            state.set(HTML_FLAG_IN_HEAD, false);
            state.set(HTML_FLAG_IN_BODY, !is_self_closing);
        }
        return;
    }

    if eq_ascii_case_insensitive(local_name, b"script") {
        state.set(HTML_FLAG_IN_SCRIPT, !is_closing && !is_self_closing);
        return;
    }

    if eq_ascii_case_insensitive(local_name, b"style") {
        state.set(HTML_FLAG_IN_STYLE, !is_closing && !is_self_closing);
    }
}

fn is_block_level_tag(local_name: &[u8]) -> bool {
    eq_ascii_case_insensitive(local_name, b"p")
        || eq_ascii_case_insensitive(local_name, b"div")
        || eq_ascii_case_insensitive(local_name, b"section")
        || eq_ascii_case_insensitive(local_name, b"article")
        || eq_ascii_case_insensitive(local_name, b"aside")
        || eq_ascii_case_insensitive(local_name, b"header")
        || eq_ascii_case_insensitive(local_name, b"footer")
        || eq_ascii_case_insensitive(local_name, b"nav")
        || eq_ascii_case_insensitive(local_name, b"li")
        || eq_ascii_case_insensitive(local_name, b"ul")
        || eq_ascii_case_insensitive(local_name, b"ol")
        || eq_ascii_case_insensitive(local_name, b"h1")
        || eq_ascii_case_insensitive(local_name, b"h2")
        || eq_ascii_case_insensitive(local_name, b"h3")
        || eq_ascii_case_insensitive(local_name, b"h4")
        || eq_ascii_case_insensitive(local_name, b"h5")
        || eq_ascii_case_insensitive(local_name, b"h6")
        || eq_ascii_case_insensitive(local_name, b"blockquote")
        || eq_ascii_case_insensitive(local_name, b"pre")
        || eq_ascii_case_insensitive(local_name, b"table")
        || eq_ascii_case_insensitive(local_name, b"tr")
        || eq_ascii_case_insensitive(local_name, b"br")
        || eq_ascii_case_insensitive(local_name, b"hr")
}

fn tag_inserts_paragraph_break(tag: &[u8]) -> bool {
    parse_html_tag_info(tag)
        .map(|info| is_block_level_tag(info.local_name))
        .unwrap_or(false)
}

fn update_chapter_label_from_resource<const N: usize>(resource_path: &str, out: &mut String<N>) {
    out.clear();
    let stem = resource_path
        .rsplit('/')
        .next()
        .unwrap_or(resource_path)
        .rsplit_once('.')
        .map(|(left, _)| left)
        .unwrap_or(resource_path)
        .trim();

    if let Some(chapter_number) = inferred_chapter_number(stem) {
        let _ = out.push_str("Chapter ");
        push_u32_as_ascii(out, chapter_number.max(1));
        return;
    }

    let mut word_start = true;
    let mut wrote_any = false;
    for byte in stem.as_bytes() {
        let mut out_byte = *byte;
        if out_byte == b'_' || out_byte == b'-' || out_byte == b'.' {
            out_byte = b' ';
        }

        if out_byte == b' ' {
            if !wrote_any || word_start {
                continue;
            }
            if out.push(' ').is_err() {
                break;
            }
            word_start = true;
            continue;
        }

        let ch = if out_byte.is_ascii_alphabetic() {
            if word_start {
                (out_byte as char).to_ascii_uppercase()
            } else {
                (out_byte as char).to_ascii_lowercase()
            }
        } else if out_byte.is_ascii_digit() {
            out_byte as char
        } else {
            continue;
        };

        if out.push(ch).is_err() {
            break;
        }
        wrote_any = true;
        word_start = false;
    }

    if !wrote_any {
        let _ = out.push_str("Section");
    }
}

fn inferred_chapter_number(stem: &str) -> Option<u32> {
    let bytes = stem.as_bytes();
    if bytes.is_empty() {
        return None;
    }

    if let Some(pos) = find_ascii_case_insensitive(bytes, b"-h-", 0) {
        let suffix = &bytes[pos + 3..];
        if let Some(value) = parse_leading_ascii_u32(suffix) {
            return Some(value.saturating_add(1));
        }
    }

    if bytes.iter().all(|b| b.is_ascii_digit()) {
        return parse_ascii_u32(bytes);
    }

    if contains_ascii_case_insensitive(bytes, b"chapter")
        || contains_ascii_case_insensitive(bytes, b"capitulo")
        || contains_ascii_case_insensitive(bytes, b"cap")
    {
        let mut end = bytes.len();
        while end > 0 && bytes[end - 1].is_ascii_digit() {
            end -= 1;
        }
        if end < bytes.len() {
            return parse_ascii_u32(&bytes[end..]);
        }
    }

    None
}

fn parse_ascii_u32(bytes: &[u8]) -> Option<u32> {
    if bytes.is_empty() || !bytes.iter().all(|b| b.is_ascii_digit()) {
        return None;
    }

    let mut value = 0u32;
    for &digit in bytes {
        value = value
            .saturating_mul(10)
            .saturating_add((digit - b'0') as u32);
    }
    Some(value)
}

fn parse_leading_ascii_u32(bytes: &[u8]) -> Option<u32> {
    let mut len = 0usize;
    while len < bytes.len() && bytes[len].is_ascii_digit() {
        len += 1;
    }
    if len == 0 {
        return None;
    }
    parse_ascii_u32(&bytes[..len])
}

fn push_u32_as_ascii<const N: usize>(out: &mut String<N>, mut value: u32) {
    let mut digits = [0u8; 10];
    let mut len = 0usize;
    if value == 0 {
        let _ = out.push('0');
        return;
    }
    while value > 0 && len < digits.len() {
        digits[len] = (value % 10) as u8;
        value /= 10;
        len += 1;
    }
    for idx in (0..len).rev() {
        let _ = out.push((b'0' + digits[idx]) as char);
    }
}

fn sanitize_epub_chunk(
    chunk: &[u8],
    html_state: &mut HtmlParseState,
    treat_as_plain_text: bool,
) -> (String<SD_CATALOG_TEXT_BYTES>, bool, Option<usize>) {
    let mut out = String::<SD_CATALOG_TEXT_BYTES>::new();
    let mut truncated = false;
    let mut tail_start = None;
    let mut last_was_space = true;
    let mut cursor = 0usize;

    while cursor < chunk.len() {
        let byte = chunk[cursor];
        if byte == b'<' {
            let Some(tag_end_rel) = chunk[cursor + 1..].iter().position(|b| *b == b'>') else {
                tail_start = Some(cursor);
                break;
            };
            let tag_end = cursor + 1 + tag_end_rel;
            let raw_tag = &chunk[cursor + 1..tag_end];
            let paragraph_break = tag_inserts_paragraph_break(raw_tag);
            apply_html_tag_state(raw_tag, html_state);
            cursor = tag_end + 1;
            if html_state.should_emit_text(treat_as_plain_text) {
                if paragraph_break {
                    push_paragraph_break(&mut out, &mut truncated, &mut last_was_space);
                } else {
                    push_normalized_char(&mut out, ' ', &mut truncated, &mut last_was_space);
                }
            }
            if truncated {
                break;
            }
            continue;
        }

        if byte == b'&' {
            let mut entity = [0u8; HTML_ENTITY_BYTES];
            let mut entity_len = 0usize;
            let mut entity_cursor = cursor + 1;
            let mut decoded = None;
            let mut incomplete = true;

            while entity_cursor < chunk.len() {
                let entity_byte = chunk[entity_cursor];
                if entity_byte == b';' {
                    decoded = Some(decode_html_entity(&entity[..entity_len]).unwrap_or(' '));
                    entity_cursor += 1;
                    incomplete = false;
                    break;
                }

                if entity_byte.is_ascii_alphanumeric() || matches!(entity_byte, b'#' | b'x' | b'X')
                {
                    if entity_len < entity.len() {
                        entity[entity_len] = entity_byte;
                        entity_len += 1;
                        entity_cursor += 1;
                        continue;
                    }
                    incomplete = false;
                    break;
                }

                incomplete = false;
                break;
            }

            if incomplete {
                tail_start = Some(cursor);
                break;
            }

            if html_state.should_emit_text(treat_as_plain_text) {
                push_normalized_char(
                    &mut out,
                    decoded.unwrap_or(' '),
                    &mut truncated,
                    &mut last_was_space,
                );
            }
            cursor = if decoded.is_some() {
                entity_cursor
            } else {
                cursor + 1
            };
            if truncated {
                break;
            }
            continue;
        }

        if !html_state.should_emit_text(treat_as_plain_text) {
            cursor += 1;
            continue;
        }

        match byte {
            b'\r' | b'\n' | b'\t' | b' ' => {
                push_normalized_char(&mut out, ' ', &mut truncated, &mut last_was_space);
                cursor += 1;
            }
            _ if byte.is_ascii_control() => {
                cursor += 1;
            }
            _ => match decode_utf8_char(chunk, cursor) {
                Utf8ChunkDecode::Char(ch, advance) => {
                    push_normalized_char(&mut out, ch, &mut truncated, &mut last_was_space);
                    cursor += advance;
                }
                Utf8ChunkDecode::Incomplete => {
                    tail_start = Some(cursor);
                    break;
                }
                Utf8ChunkDecode::Invalid => {
                    let fallback = decode_single_byte_fallback(byte);
                    push_normalized_char(&mut out, fallback, &mut truncated, &mut last_was_space);
                    cursor += 1;
                }
            },
        }

        if truncated {
            break;
        }
    }

    while out.ends_with(' ') || out.ends_with('\n') {
        let _ = out.pop();
    }

    (out, truncated, tail_start)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Utf8ChunkDecode {
    Char(char, usize),
    Incomplete,
    Invalid,
}

fn decode_utf8_char(chunk: &[u8], cursor: usize) -> Utf8ChunkDecode {
    let first = chunk[cursor];
    if first < 0x80 {
        return Utf8ChunkDecode::Char(first as char, 1);
    }

    let remaining = chunk.len().saturating_sub(cursor);
    if (0xC2..=0xDF).contains(&first) {
        if remaining < 2 {
            return Utf8ChunkDecode::Incomplete;
        }
        let b1 = chunk[cursor + 1];
        if !is_utf8_continuation(b1) {
            return Utf8ChunkDecode::Invalid;
        }
        let codepoint = (((first & 0x1f) as u32) << 6) | ((b1 & 0x3f) as u32);
        return core::char::from_u32(codepoint)
            .map(|ch| Utf8ChunkDecode::Char(ch, 2))
            .unwrap_or(Utf8ChunkDecode::Invalid);
    }

    if (0xE0..=0xEF).contains(&first) {
        if remaining < 3 {
            return Utf8ChunkDecode::Incomplete;
        }
        let b1 = chunk[cursor + 1];
        let b2 = chunk[cursor + 2];
        if !is_utf8_continuation(b1) || !is_utf8_continuation(b2) {
            return Utf8ChunkDecode::Invalid;
        }
        if (first == 0xE0 && b1 < 0xA0) || (first == 0xED && b1 >= 0xA0) {
            return Utf8ChunkDecode::Invalid;
        }
        let codepoint =
            (((first & 0x0f) as u32) << 12) | (((b1 & 0x3f) as u32) << 6) | ((b2 & 0x3f) as u32);
        return core::char::from_u32(codepoint)
            .map(|ch| Utf8ChunkDecode::Char(ch, 3))
            .unwrap_or(Utf8ChunkDecode::Invalid);
    }

    if (0xF0..=0xF4).contains(&first) {
        if remaining < 4 {
            return Utf8ChunkDecode::Incomplete;
        }
        let b1 = chunk[cursor + 1];
        let b2 = chunk[cursor + 2];
        let b3 = chunk[cursor + 3];
        if !is_utf8_continuation(b1) || !is_utf8_continuation(b2) || !is_utf8_continuation(b3) {
            return Utf8ChunkDecode::Invalid;
        }
        if (first == 0xF0 && b1 < 0x90) || (first == 0xF4 && b1 > 0x8F) {
            return Utf8ChunkDecode::Invalid;
        }
        let codepoint = (((first & 0x07) as u32) << 18)
            | (((b1 & 0x3f) as u32) << 12)
            | (((b2 & 0x3f) as u32) << 6)
            | ((b3 & 0x3f) as u32);
        return core::char::from_u32(codepoint)
            .map(|ch| Utf8ChunkDecode::Char(ch, 4))
            .unwrap_or(Utf8ChunkDecode::Invalid);
    }

    Utf8ChunkDecode::Invalid
}

fn is_utf8_continuation(byte: u8) -> bool {
    (byte & 0b1100_0000) == 0b1000_0000
}

fn decode_single_byte_fallback(byte: u8) -> char {
    match byte {
        0x91 | 0x92 => '\'',
        0x93 | 0x94 => '"',
        0x96 | 0x97 => '-',
        0x85 => '.',
        0xA0 => ' ',
        0xA1 => '¡',
        0xBF => '¿',
        0xC0 => 'À',
        0xC1 => 'Á',
        0xC8 => 'È',
        0xC9 => 'É',
        0xCC => 'Ì',
        0xCD => 'Í',
        0xD1 => 'Ñ',
        0xD2 => 'Ò',
        0xD3 => 'Ó',
        0xD9 => 'Ù',
        0xDA => 'Ú',
        0xDC => 'Ü',
        0xE0 => 'à',
        0xE1 => 'á',
        0xE7 => 'ç',
        0xE8 => 'è',
        0xE9 => 'é',
        0xEC => 'ì',
        0xED => 'í',
        0xF1 => 'ñ',
        0xF2 => 'ò',
        0xF3 => 'ó',
        0xF9 => 'ù',
        0xFA => 'ú',
        0xFC => 'ü',
        _ if byte.is_ascii() => byte as char,
        _ => '?',
    }
}

fn push_paragraph_break<const N: usize>(
    out: &mut String<N>,
    truncated: &mut bool,
    last_was_space: &mut bool,
) {
    while out.ends_with(' ') {
        let _ = out.pop();
    }

    if out.is_empty() || out.ends_with('\n') {
        *last_was_space = true;
        return;
    }

    if out.push('\n').is_err() {
        *truncated = true;
        return;
    }
    *last_was_space = true;
}

fn push_normalized_char<const N: usize>(
    out: &mut String<N>,
    ch: char,
    truncated: &mut bool,
    last_was_space: &mut bool,
) {
    if ch.is_ascii_whitespace() {
        if out.is_empty() || *last_was_space {
            return;
        }
        if out.push(' ').is_err() {
            *truncated = true;
            return;
        }
        *last_was_space = true;
        return;
    }

    if out.push(ch).is_err() {
        *truncated = true;
        return;
    }
    *last_was_space = false;
}

fn decode_html_entity(entity: &[u8]) -> Option<char> {
    if entity.eq_ignore_ascii_case(b"amp") {
        Some('&')
    } else if entity.eq_ignore_ascii_case(b"lt") {
        Some('<')
    } else if entity.eq_ignore_ascii_case(b"gt") {
        Some('>')
    } else if entity.eq_ignore_ascii_case(b"quot") {
        Some('"')
    } else if entity.eq_ignore_ascii_case(b"apos")
        || entity.eq_ignore_ascii_case(b"lsquo")
        || entity.eq_ignore_ascii_case(b"rsquo")
    {
        Some('\'')
    } else if entity.eq_ignore_ascii_case(b"ldquo")
        || entity.eq_ignore_ascii_case(b"rdquo")
        || entity.eq_ignore_ascii_case(b"laquo")
        || entity.eq_ignore_ascii_case(b"raquo")
    {
        Some('"')
    } else if entity.eq_ignore_ascii_case(b"nbsp") || entity == b"#160" {
        Some(' ')
    } else if entity == b"#39" {
        Some('\'')
    } else if entity.eq_ignore_ascii_case(b"ndash") || entity.eq_ignore_ascii_case(b"mdash") {
        Some('-')
    } else if entity.eq_ignore_ascii_case(b"hellip") {
        Some('.')
    } else if entity.eq_ignore_ascii_case(b"aacute") {
        Some('á')
    } else if entity.eq_ignore_ascii_case(b"eacute") {
        Some('é')
    } else if entity.eq_ignore_ascii_case(b"iacute") {
        Some('í')
    } else if entity.eq_ignore_ascii_case(b"oacute") {
        Some('ó')
    } else if entity.eq_ignore_ascii_case(b"uacute") {
        Some('ú')
    } else if entity.eq_ignore_ascii_case(b"ntilde") {
        Some('ñ')
    } else if entity.eq_ignore_ascii_case(b"uuml") {
        Some('ü')
    } else if entity.eq_ignore_ascii_case(b"agrave") {
        Some('à')
    } else if entity.eq_ignore_ascii_case(b"egrave") {
        Some('è')
    } else if entity.eq_ignore_ascii_case(b"igrave") {
        Some('ì')
    } else if entity.eq_ignore_ascii_case(b"ograve") {
        Some('ò')
    } else if entity.eq_ignore_ascii_case(b"ugrave") {
        Some('ù')
    } else if entity.eq_ignore_ascii_case(b"ccedil") {
        Some('ç')
    } else if entity.eq_ignore_ascii_case(b"iexcl") {
        Some('¡')
    } else if entity.eq_ignore_ascii_case(b"iquest") {
        Some('¿')
    } else {
        decode_numeric_entity(entity)
    }
}

fn decode_numeric_entity(entity: &[u8]) -> Option<char> {
    let first = entity.first().copied()?;
    if first != b'#' {
        return None;
    }

    let (digits, radix) = match entity.get(1).copied() {
        Some(b'x' | b'X') => (&entity[2..], 16),
        _ => (&entity[1..], 10),
    };
    if digits.is_empty() {
        return None;
    }

    let mut value = 0u32;
    for &digit in digits {
        let step = match digit {
            b'0'..=b'9' => (digit - b'0') as u32,
            b'a'..=b'f' if radix == 16 => (digit - b'a' + 10) as u32,
            b'A'..=b'F' if radix == 16 => (digit - b'A' + 10) as u32,
            _ => return None,
        };
        value = value.saturating_mul(radix).saturating_add(step);
    }

    core::char::from_u32(value)
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
    fn catalog_titles_can_be_replaced() {
        let mut src = FakeSdCatalogSource::new();
        let loaded = src.set_catalog_titles_from_iter(["QUIJOTE.EPU", "ALICE.EPUB"]);
        assert_eq!(
            loaded,
            SdCatalogLoadResult {
                loaded: 2,
                truncated: false
            }
        );
        assert_eq!(src.title_count(), 2);
        assert_eq!(src.title_at(0), Some("Quijote"));
        assert_eq!(src.title_at(1), Some("Alice"));
    }

    #[test]
    fn catalog_entries_keep_cover_flags() {
        let mut src = FakeSdCatalogSource::new();
        let loaded = src.set_catalog_entries_from_iter([("Book One", true), ("Book Two", false)]);
        assert_eq!(
            loaded,
            SdCatalogLoadResult {
                loaded: 2,
                truncated: false
            }
        );
        assert!(src.has_cover_at(0));
        assert!(!src.has_cover_at(1));
    }

    #[test]
    fn select_resets_and_streams_words() {
        let mut src = FakeSdCatalogSource::new();
        src.select_text(1).unwrap();

        let first = src.next_word().unwrap().unwrap();
        assert_eq!(first.text, "Alice");
        assert_eq!(src.paragraph_progress(), (1, src.paragraph_progress().1));
    }

    #[test]
    fn injected_epub_chunk_is_streamed() {
        let mut src = FakeSdCatalogSource::new();
        src.set_catalog_entries_from_iter([("Book One", false)]);
        let loaded = src
            .set_catalog_text_chunk_from_bytes(
                0,
                b"<html><body>Hola <b>mundo</b>.</body></html>",
                true,
                "OEBPS/chapter.xhtml",
            )
            .unwrap();
        assert!(loaded.loaded);

        src.select_text(0).unwrap();
        let first = src.next_word().unwrap().unwrap();
        let second = src.next_word().unwrap().unwrap();
        assert_eq!(first.text, "Hola");
        assert_eq!(second.text, "mundo.");
    }

    #[test]
    fn html_head_script_and_style_are_not_rendered() {
        let mut src = FakeSdCatalogSource::new();
        src.set_catalog_entries_from_iter([("Book One", false)]);
        src.set_catalog_text_chunk_from_bytes(
            0,
            b"<html><head><title>Book Title</title><style>.x{color:red}</style></head><body>Hello <script>ignore_me()</script>world</body></html>",
            true,
            "OEBPS/chapter.xhtml",
        )
        .unwrap();
        src.select_text(0).unwrap();

        assert_eq!(src.next_word().unwrap().map(|w| w.text), Some("Hello"));
        assert_eq!(src.next_word().unwrap().map(|w| w.text), Some("world"));
        assert_eq!(src.next_word().unwrap(), None);
    }

    #[test]
    fn html_state_resets_when_resource_changes() {
        let mut src = FakeSdCatalogSource::new();
        src.set_catalog_entries_from_iter([("Book One", false)]);
        src.set_catalog_text_chunk_from_bytes(
            0,
            b"<html><body>uno</body></html>",
            true,
            "OEBPS/chapter1.xhtml",
        )
        .unwrap();
        src.set_catalog_text_chunk_from_bytes(
            0,
            b"<p>dos tres</p>",
            true,
            "OEBPS/chapter2-fragment.xhtml",
        )
        .unwrap();
        src.select_text(0).unwrap();

        assert_eq!(src.next_word().unwrap().map(|w| w.text), Some("dos"));
        assert_eq!(src.next_word().unwrap().map(|w| w.text), Some("tres"));
        assert_eq!(src.next_word().unwrap(), None);
    }

    #[test]
    fn stream_detects_multiple_paragraphs_from_html_blocks() {
        let mut src = FakeSdCatalogSource::new();
        src.set_catalog_entries_from_iter([("Book One", false)]);
        src.set_catalog_text_chunk_from_bytes(
            0,
            b"<html><body><h1>Capitulo Uno</h1><p>Primer parrafo.</p><p>Segundo parrafo.</p></body></html>",
            true,
            "OEBPS/chapter-one.xhtml",
        )
        .unwrap();
        src.select_text(0).unwrap();

        assert_eq!(src.paragraph_total(), 3);
        assert_eq!(src.paragraph_preview(0), Some("Capitulo Uno"));
        assert_eq!(src.paragraph_preview(1), Some("Primer parrafo."));
        assert_eq!(src.paragraph_preview(2), Some("Segundo parrafo."));
    }

    #[test]
    fn stream_resource_transitions_advance_current_chapter() {
        let mut src = FakeSdCatalogSource::new();
        src.set_catalog_entries_from_iter([("Book One", false)]);
        src.set_catalog_text_chunk_from_bytes(
            0,
            b"<html><body><p>uno</p></body></html>",
            true,
            "OEBPS/chapter-one.xhtml",
        )
        .unwrap();
        assert_eq!(src.current_chapter_index(), Some(0));
        assert_eq!(src.chapter_count(), 1);

        src.set_catalog_text_chunk_from_bytes(
            0,
            b"<html><body><p>dos</p></body></html>",
            false,
            "OEBPS/chapter-two.xhtml",
        )
        .unwrap();

        assert_eq!(src.current_chapter_index(), Some(1));
        assert_eq!(src.chapter_count(), 2);
        assert_eq!(
            src.chapter_at(1).map(|chapter| chapter.label),
            Some("Chapter Two")
        );
    }

    #[test]
    fn html_entity_split_across_chunks_is_preserved() {
        let mut src = FakeSdCatalogSource::new();
        src.set_catalog_entries_from_iter([("Book One", false)]);
        src.set_catalog_text_chunk_from_bytes(0, b"<p>Uno &amp", false, "OEBPS/chapter.xhtml")
            .unwrap();
        src.set_catalog_text_chunk_from_bytes(0, b"; Dos</p>", false, "OEBPS/chapter.xhtml")
            .unwrap();
        src.select_text(0).unwrap();

        assert_eq!(src.next_word().unwrap().map(|w| w.text), Some("&"));
        assert_eq!(src.next_word().unwrap().map(|w| w.text), Some("Dos"));
    }

    #[test]
    fn stream_chapter_hint_sets_total_and_current() {
        let mut src = FakeSdCatalogSource::new();
        src.set_catalog_entries_from_iter([("Book One", false)]);
        src.set_catalog_text_chunk_from_bytes(
            0,
            b"<html><body><p>uno</p></body></html>",
            false,
            "OEBPS/0007.xhtml",
        )
        .unwrap();
        src.set_catalog_stream_chapter_hint(0, 6, 24).unwrap();

        assert_eq!(src.current_chapter_index(), Some(6));
        assert_eq!(src.chapter_count(), 24);
    }

    #[test]
    fn stream_seek_chapter_emits_targeted_refill_request() {
        let mut src = FakeSdCatalogSource::new();
        src.set_catalog_entries_from_iter([("Book One", false)]);
        src.set_catalog_text_chunk_from_bytes(
            0,
            b"<html><body><p>uno dos</p></body></html>",
            false,
            "OEBPS/0002.xhtml",
        )
        .unwrap();
        src.set_catalog_stream_chapter_hint(0, 1, 12).unwrap();
        src.select_text(0).unwrap();

        assert!(src.seek_chapter(6).unwrap());
        assert_eq!(
            src.take_chunk_refill_request(),
            Some(SdChunkRefillRequest {
                book_index: 0,
                target_chapter: Some(6)
            })
        );
    }

    #[test]
    fn chapter_label_infers_h_suffix_pattern() {
        let mut src = FakeSdCatalogSource::new();
        src.set_catalog_entries_from_iter([("Book One", false)]);
        src.set_catalog_text_chunk_from_bytes(
            0,
            b"<html><body><p>uno</p></body></html>",
            false,
            "OEBPS/4969397097534755666_768-h-0.htm.xhtml",
        )
        .unwrap();

        assert_eq!(
            src.chapter_at(0).map(|chapter| chapter.label),
            Some("Chapter 1")
        );
    }

    #[test]
    fn stream_chunk_requests_refill_when_depleted() {
        let mut src = FakeSdCatalogSource::new();
        src.set_catalog_entries_from_iter([("Book One", false)]);
        src.set_catalog_text_chunk_from_bytes(0, b"uno dos", false, "OEBPS/chapter.xhtml")
            .unwrap();
        src.select_text(0).unwrap();

        assert_eq!(src.next_word().unwrap().map(|w| w.text), Some("uno"));
        assert_eq!(src.next_word().unwrap().map(|w| w.text), Some("dos"));
        assert_eq!(src.next_word().unwrap(), None);
        assert!(src.is_waiting_for_refill());
        assert_eq!(
            src.take_chunk_refill_request(),
            Some(SdChunkRefillRequest {
                book_index: 0,
                target_chapter: None
            })
        );
    }

    #[test]
    fn stream_chunk_end_of_resource_still_requests_refill() {
        let mut src = FakeSdCatalogSource::new();
        src.set_catalog_entries_from_iter([("Book One", false)]);
        src.set_catalog_text_chunk_from_bytes(0, b"uno dos", true, "OEBPS/chapter.xhtml")
            .unwrap();
        src.select_text(0).unwrap();

        assert_eq!(src.next_word().unwrap().map(|w| w.text), Some("uno"));
        assert_eq!(src.next_word().unwrap().map(|w| w.text), Some("dos"));
        assert_eq!(src.next_word().unwrap(), None);
        assert!(src.is_waiting_for_refill());
        assert_eq!(
            src.take_chunk_refill_request(),
            Some(SdChunkRefillRequest {
                book_index: 0,
                target_chapter: None
            })
        );
    }

    #[test]
    fn sanitize_preserves_utf8_accents() {
        let mut html_state = HtmlParseState::default();
        let (sanitized, truncated, tail_start) =
            sanitize_epub_chunk("salió corazón".as_bytes(), &mut html_state, true);
        assert_eq!(sanitized.as_str(), "salió corazón");
        assert!(!truncated);
        assert_eq!(tail_start, None);
    }

    #[test]
    fn stream_chunk_reassembles_split_utf8_codepoint() {
        let mut src = FakeSdCatalogSource::new();
        src.set_catalog_entries_from_iter([("Book One", false)]);
        src.set_catalog_text_chunk_from_bytes(0, b"sali\xc3", false, "OEBPS/chapter.xhtml")
            .unwrap();
        src.set_catalog_text_chunk_from_bytes(0, b"\xb3 bien", false, "OEBPS/chapter.xhtml")
            .unwrap();
        src.select_text(0).unwrap();

        assert_eq!(src.next_word().unwrap().map(|w| w.text), Some("salió"));
        assert_eq!(src.next_word().unwrap().map(|w| w.text), Some("bien"));
    }

    #[test]
    fn stream_chunk_decodes_typographic_apostrophe_entity() {
        let mut src = FakeSdCatalogSource::new();
        src.set_catalog_entries_from_iter([("Book One", false)]);
        src.set_catalog_text_chunk_from_bytes(0, b"can&rsquo;t stop", false, "OEBPS/chapter.xhtml")
            .unwrap();
        src.select_text(0).unwrap();

        assert_eq!(src.next_word().unwrap().map(|w| w.text), Some("can't"));
        assert_eq!(src.next_word().unwrap().map(|w| w.text), Some("stop"));
    }
}
