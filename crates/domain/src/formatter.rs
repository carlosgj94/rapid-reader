use alloc::alloc::{Layout, alloc_zeroed, handle_alloc_error};
use alloc::boxed::Box;

use crate::{
    content::{ArticleDocument, ReaderScript, script_paragraph, script_paragraph_count},
    text::InlineText,
};

// One reading unit is roughly one word-sized chunk on screen, so the previous
// 384/24 limits were too small for real longform articles. Keep a materially
// larger ceiling, but stay comfortably below the heap spike caused by a ~45 KiB
// boxed document on device.
pub const MAX_READING_UNITS: usize = 640;
pub const MAX_READING_PARAGRAPHS: usize = 40;
pub const MAX_READING_TOKEN_BYTES: usize = 32;
pub const MAX_STAGE_SEGMENT_BYTES: usize = 32;
pub const MAX_PARAGRAPH_PREVIEW_BYTES: usize = 64;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum StageFont {
    #[default]
    Large,
    Medium,
    Small,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub struct UnitFlags {
    pub clause_pause: bool,
    pub sentence_pause: bool,
    pub paragraph_start: bool,
    pub paragraph_end: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub struct ReadingUnit {
    pub display: InlineText<MAX_READING_TOKEN_BYTES>,
    pub paragraph_index: u8,
    pub anchor_index: u8,
    pub char_count: u8,
    pub font: StageFont,
    pub flags: UnitFlags,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub struct ParagraphAnchor {
    pub start_unit_index: u16,
    pub preview: InlineText<MAX_PARAGRAPH_PREVIEW_BYTES>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ReadingDocument {
    pub unit_count: u16,
    pub paragraph_count: u8,
    pub units: [ReadingUnit; MAX_READING_UNITS],
    pub paragraphs: [ParagraphAnchor; MAX_READING_PARAGRAPHS],
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub struct StageToken {
    pub left: InlineText<MAX_STAGE_SEGMENT_BYTES>,
    pub right: InlineText<MAX_STAGE_SEGMENT_BYTES>,
    pub font: StageFont,
}

impl ReadingDocument {
    pub const fn empty() -> Self {
        Self {
            unit_count: 0,
            paragraph_count: 0,
            units: [ReadingUnit::new(); MAX_READING_UNITS],
            paragraphs: [ParagraphAnchor::new(); MAX_READING_PARAGRAPHS],
        }
    }

    pub fn boxed_empty() -> Box<Self> {
        let layout = Layout::new::<Self>();
        let ptr = unsafe { alloc_zeroed(layout) };
        let ptr = core::ptr::NonNull::new(ptr).unwrap_or_else(|| handle_alloc_error(layout));
        unsafe { Box::from_raw(ptr.as_ptr().cast::<Self>()) }
    }

    pub const fn is_empty(&self) -> bool {
        self.unit_count == 0
    }

    pub fn unit(&self, index: u16) -> &ReadingUnit {
        let safe_index = index.min(self.unit_count.saturating_sub(1));
        &self.units[safe_index as usize]
    }

    pub fn preview_for_paragraph(
        &self,
        paragraph_index: u8,
    ) -> InlineText<MAX_PARAGRAPH_PREVIEW_BYTES> {
        let safe_index = paragraph_index
            .saturating_sub(1)
            .min(self.paragraph_count.saturating_sub(1)) as usize;
        self.paragraphs[safe_index].preview
    }

    pub fn paragraph_start(&self, paragraph_index: u8) -> u16 {
        let safe_index = paragraph_index
            .saturating_sub(1)
            .min(self.paragraph_count.saturating_sub(1)) as usize;
        self.paragraphs[safe_index].start_unit_index
    }

    pub fn push_paragraph_text(&mut self, paragraph: &str) -> bool {
        if paragraph.is_empty() || self.paragraph_count as usize >= MAX_READING_PARAGRAPHS {
            return false;
        }

        self.push_paragraph(self.paragraph_count.saturating_add(1), paragraph)
    }

    fn push_paragraph(&mut self, paragraph_index: u8, paragraph: &str) -> bool {
        if paragraph_index as usize > MAX_READING_PARAGRAPHS
            || self.unit_count as usize >= MAX_READING_UNITS
        {
            return false;
        }

        let paragraph_slot = paragraph_index.saturating_sub(1) as usize;
        self.paragraphs[paragraph_slot] = ParagraphAnchor {
            start_unit_index: self.unit_count,
            preview: preview_excerpt(paragraph),
        };
        self.paragraph_count = paragraph_index;

        let mut index = 0usize;
        let mut first_unit = true;
        let mut complete = true;

        while let Some((start, end)) = next_chunk_bounds(paragraph, index) {
            index = end;
            let chunk = &paragraph[start..end];

            if !contains_word_content(chunk) {
                self.attach_standalone_punctuation(chunk);
                continue;
            }

            self.push_chunk(paragraph_index, chunk, first_unit);
            first_unit = false;

            if self.unit_count as usize >= MAX_READING_UNITS {
                complete = false;
                break;
            }
        }

        if self.unit_count > 0 {
            let last_index = self.unit_count as usize - 1;
            self.units[last_index].flags.paragraph_end = true;
        }

        complete && index >= paragraph.len()
    }

    fn attach_standalone_punctuation(&mut self, chunk: &str) {
        if self.unit_count == 0 {
            return;
        }

        let last_index = self.unit_count as usize - 1;
        let flags = classify_trailing_punctuation(chunk, false);
        self.units[last_index].flags.clause_pause |= flags.clause_pause;
        self.units[last_index].flags.sentence_pause |= flags.sentence_pause;
    }

    fn push_chunk(&mut self, paragraph_index: u8, chunk: &str, paragraph_start: bool) {
        let segments = split_for_stage(chunk);
        let mut segment_index = 0usize;

        while segment_index < segments.len() {
            let segment = segments[segment_index];
            if segment.is_empty() {
                segment_index += 1;
                continue;
            }
            let core = lexical_core(segment);
            let display = InlineText::from_slice(segment);
            let char_count = display.char_count().min(u8::MAX as usize) as u8;
            let leading_chars = segment[..core.start].chars().count() as u8;
            let core_chars = core.text.chars().count();
            let anchor = leading_chars
                .saturating_add(preferred_anchor(core_chars) as u8)
                .min(char_count.saturating_sub(1));
            let mut flags = if segment_index + 1 == segments.len() {
                classify_trailing_punctuation(segment, looks_like_abbreviation(segment))
            } else {
                UnitFlags::default()
            };

            if paragraph_start && segment_index == 0 {
                flags.paragraph_start = true;
            }

            self.push_unit(ReadingUnit {
                display,
                paragraph_index,
                anchor_index: anchor,
                char_count,
                font: font_for_token(char_count as usize),
                flags,
            });

            if self.unit_count as usize >= MAX_READING_UNITS {
                return;
            }

            segment_index += 1;
        }
    }

    fn push_unit(&mut self, unit: ReadingUnit) {
        if self.unit_count as usize >= MAX_READING_UNITS {
            return;
        }

        self.units[self.unit_count as usize] = unit;
        self.unit_count = self.unit_count.saturating_add(1);
    }
}

impl Default for ReadingDocument {
    fn default() -> Self {
        Self::empty()
    }
}

impl UnitFlags {
    pub const fn new() -> Self {
        Self {
            clause_pause: false,
            sentence_pause: false,
            paragraph_start: false,
            paragraph_end: false,
        }
    }
}

impl ReadingUnit {
    pub const fn new() -> Self {
        Self {
            display: InlineText::new(),
            paragraph_index: 0,
            anchor_index: 0,
            char_count: 0,
            font: StageFont::Large,
            flags: UnitFlags::new(),
        }
    }
}

impl ParagraphAnchor {
    pub const fn new() -> Self {
        Self {
            start_unit_index: 0,
            preview: InlineText::new(),
        }
    }
}

impl ReadingUnit {
    pub fn dwell_ms(&self, wpm: u16) -> u32 {
        let base = 60_000u32 / wpm.max(1) as u32;
        let length_bonus = match self.char_count {
            0..=3 => 0,
            4..=5 => base / 10,
            6..=7 => base / 5,
            8..=10 => base / 3,
            11..=14 => base / 2,
            _ => (base * 3) / 4,
        };
        let clause_bonus = if self.flags.clause_pause { base / 2 } else { 0 };
        let sentence_bonus = if self.flags.sentence_pause {
            (base * 3) / 4
        } else {
            0
        };
        let paragraph_bonus = if self.flags.paragraph_end { base } else { 0 };

        base + length_bonus + clause_bonus + sentence_bonus + paragraph_bonus
    }

    pub fn stage_token(&self) -> StageToken {
        let mut token = StageToken {
            font: self.font,
            ..StageToken::default()
        };

        let split_byte = byte_index_for_char(self.display.as_str(), self.anchor_index as usize);
        token.left = InlineText::from_slice(&self.display.as_str()[..split_byte]);
        token.right = InlineText::from_slice(&self.display.as_str()[split_byte..]);
        token
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct CoreBounds<'a> {
    start: usize,
    text: &'a str,
}

pub fn format_article_document(article: &ArticleDocument) -> ReadingDocument {
    let mut document = ReadingDocument::empty();
    let paragraph_total = script_paragraph_count(article.script).min(MAX_READING_PARAGRAPHS);

    let mut paragraph_index = 0usize;
    while paragraph_index < paragraph_total {
        document.push_paragraph(
            (paragraph_index + 1) as u8,
            script_paragraph(article.script, paragraph_index),
        );
        paragraph_index += 1;
    }

    if document.is_empty() {
        document.push_unit(ReadingUnit {
            display: InlineText::from_slice("READY"),
            paragraph_index: 1,
            anchor_index: 1,
            char_count: 5,
            font: StageFont::Large,
            flags: UnitFlags {
                paragraph_start: true,
                paragraph_end: true,
                ..UnitFlags::default()
            },
        });
        document.paragraphs[0] = ParagraphAnchor {
            start_unit_index: 0,
            preview: InlineText::from_slice("Ready to read."),
        };
        document.paragraph_count = 1;
    }

    document
}

pub fn article_document_from_script(
    source: crate::source::SourceKind,
    script: ReaderScript,
) -> ArticleDocument {
    ArticleDocument::new(source, script)
}

fn preview_excerpt(paragraph: &str) -> InlineText<MAX_PARAGRAPH_PREVIEW_BYTES> {
    let mut preview = InlineText::new();
    let mut last_was_space = false;

    for ch in paragraph.chars() {
        if ch.is_whitespace() {
            if !last_was_space && !preview.is_empty() && !preview.try_push_char(' ') {
                break;
            }
            last_was_space = true;
            continue;
        }

        last_was_space = false;
        if !preview.try_push_char(ch) {
            break;
        }
    }

    preview
}

fn next_chunk_bounds(text: &str, start: usize) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut head = start;

    while head < bytes.len() && bytes[head].is_ascii_whitespace() {
        head += 1;
    }

    if head >= bytes.len() {
        return None;
    }

    let mut tail = head;
    while tail < bytes.len() && !bytes[tail].is_ascii_whitespace() {
        tail += 1;
    }

    Some((head, tail))
}

fn split_for_stage(chunk: &str) -> [&str; 2] {
    let mut parts = [chunk, ""];
    let char_count = chunk.chars().count();

    if char_count <= 24 {
        return parts;
    }

    if let Some(split_byte) = hyphen_split_index(chunk) {
        parts[0] = &chunk[..split_byte];
        parts[1] = &chunk[split_byte..];
    }

    parts
}

fn hyphen_split_index(chunk: &str) -> Option<usize> {
    let midpoint = chunk.chars().count() / 2;
    let mut best_before = None;
    let mut best_after = None;

    for (char_index, (byte_index, ch)) in chunk.char_indices().enumerate() {
        if ch == '-' {
            if char_index <= midpoint {
                best_before = Some(byte_index + ch.len_utf8());
            } else if best_after.is_none() {
                best_after = Some(byte_index + ch.len_utf8());
            }
        }
    }

    best_before.or(best_after)
}

fn lexical_core(chunk: &str) -> CoreBounds<'_> {
    let mut start = chunk.len();
    let mut end = 0usize;

    for (byte_index, ch) in chunk.char_indices() {
        if ch.is_alphanumeric() {
            if start == chunk.len() {
                start = byte_index;
            }
            end = byte_index + ch.len_utf8();
        }
    }

    if start == chunk.len() {
        return CoreBounds {
            start: 0,
            text: chunk,
        };
    }

    CoreBounds {
        start,
        text: &chunk[start..end],
    }
}

fn contains_word_content(chunk: &str) -> bool {
    chunk.chars().any(|ch| ch.is_alphanumeric())
}

fn classify_trailing_punctuation(chunk: &str, abbreviation: bool) -> UnitFlags {
    let mut flags = UnitFlags::default();
    let core = lexical_core(chunk);
    let trailing = &chunk[core.start + core.text.len()..];

    if trailing.contains(['!', '?']) {
        flags.sentence_pause = true;
        return flags;
    }

    if trailing.contains('.') && !abbreviation {
        flags.sentence_pause = true;
        return flags;
    }

    if trailing.contains([',', ';', ':', '—']) {
        flags.clause_pause = true;
    }

    flags
}

fn looks_like_abbreviation(chunk: &str) -> bool {
    let trimmed = lexical_core(chunk).text;
    let mut period_count = 0usize;
    let mut letter_count = 0usize;

    for ch in trimmed.chars() {
        if ch == '.' {
            period_count += 1;
        } else if ch.is_ascii_alphabetic() {
            letter_count += 1;
        } else {
            return false;
        }
    }

    period_count > 0 && letter_count > period_count
}

fn preferred_anchor(core_chars: usize) -> usize {
    match core_chars {
        0..=1 => 0,
        2..=5 => 1,
        6..=9 => 2,
        10..=13 => 3,
        _ => 4,
    }
}

fn font_for_token(char_count: usize) -> StageFont {
    match char_count {
        0..=11 => StageFont::Large,
        12..=17 => StageFont::Medium,
        _ => StageFont::Small,
    }
}

fn byte_index_for_char(text: &str, char_index: usize) -> usize {
    if char_index == 0 {
        return 0;
    }

    for (current_index, (byte_index, _)) in text.char_indices().enumerate() {
        if current_index == char_index {
            return byte_index;
        }
    }

    text.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceKind;

    #[test]
    fn contractions_keep_the_apostrophe() {
        let document = format_article_document(&ArticleDocument::new(
            SourceKind::Unknown,
            ReaderScript::MachineSoul,
        ));
        let mut index = 0usize;
        let mut found = false;

        while index < document.unit_count as usize {
            if document.units[index].display.as_str() == "there's" {
                found = true;
                break;
            }
            index += 1;
        }

        assert!(found);
    }

    #[test]
    fn apostrophes_and_periods_stay_inside_expected_units() {
        let segments = split_for_stage("There's");
        assert_eq!(segments[0], "There's");
        assert_eq!(segments[1], "");

        let segments = split_for_stage("e.g.");
        assert_eq!(segments[0], "e.g.");
        assert_eq!(segments[1], "");
    }

    #[test]
    fn stage_token_splits_on_anchor() {
        let unit = ReadingUnit {
            display: InlineText::from_slice("There's"),
            paragraph_index: 1,
            anchor_index: 2,
            char_count: 7,
            font: StageFont::Large,
            flags: UnitFlags::default(),
        };

        let token = unit.stage_token();

        assert_eq!(token.left.as_str(), "Th");
        assert_eq!(token.right.as_str(), "ere's");
    }

    #[test]
    fn scientific_dwell_adds_boundary_weight() {
        let short = ReadingUnit {
            display: InlineText::from_slice("Calm"),
            paragraph_index: 1,
            anchor_index: 1,
            char_count: 4,
            font: StageFont::Large,
            flags: UnitFlags::default(),
        };
        let long_sentence_end = ReadingUnit {
            display: InlineText::from_slice("sentence."),
            paragraph_index: 1,
            anchor_index: 2,
            char_count: 9,
            font: StageFont::Large,
            flags: UnitFlags {
                sentence_pause: true,
                ..UnitFlags::default()
            },
        };

        assert!(long_sentence_end.dwell_ms(260) > short.dwell_ms(260));
    }

    #[test]
    fn article_document_helper_keeps_source_and_script() {
        let article =
            article_document_from_script(SourceKind::EditorialFeed, ReaderScript::QuietCraft);

        assert_eq!(article.source, SourceKind::EditorialFeed);
        assert_eq!(article.script, ReaderScript::QuietCraft);
    }

    #[test]
    fn stage_font_thresholds_match_stage_sizes() {
        assert_eq!(font_for_token(11), StageFont::Large);
        assert_eq!(font_for_token(12), StageFont::Medium);
        assert_eq!(font_for_token(17), StageFont::Medium);
        assert_eq!(font_for_token(18), StageFont::Small);
    }
}
