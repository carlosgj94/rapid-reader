//! EPUB contracts and a lightweight stub parser.
//!
//! The traits here are intentionally streaming-oriented so implementations can
//! work on low-memory devices without loading full books into RAM.

/// Display-safe cover pixel formats.
///
/// The current target display is a Sharp memory LCD, so only 1-bit monochrome
/// is modeled for now.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoverPixelFormat {
    Mono1Bpp,
}

/// Lightweight metadata used to list books before opening them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EpubBookSummary<'a> {
    pub title: &'a str,
    pub author: Option<&'a str>,
    pub chapter_count_hint: u16,
    pub has_cover: bool,
}

/// Cover metadata discovered from EPUB package metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EpubCoverSummary<'a> {
    pub resource_href: &'a str,
    pub media_type: &'a str,
    pub width_px: u16,
    pub height_px: u16,
    pub pixel_format: CoverPixelFormat,
}

/// Chapter metadata for navigation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EpubChapterSummary<'a> {
    pub index: u16,
    pub title: &'a str,
}

/// Stream position inside the currently opened book.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EpubStreamPosition {
    pub chapter_index: u16,
    pub byte_offset_in_chapter: u32,
}

/// Result of streaming one text chunk into a caller-provided buffer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EpubChunkRead {
    pub bytes_written: usize,
    pub chapter_break: bool,
    pub end_of_book: bool,
}

/// Catalog of EPUB books available to the app.
pub trait EpubCatalog {
    type Error;

    fn epub_count(&self) -> Result<u16, Self::Error>;
    fn epub_summary(&self, index: u16) -> Result<Option<EpubBookSummary<'_>>, Self::Error>;
}

/// Streaming EPUB parser/reader interface.
pub trait EpubReader {
    type Error;

    fn open_epub(&mut self, index: u16) -> Result<(), Self::Error>;
    fn close_epub(&mut self) -> Result<(), Self::Error>;

    fn opened_epub(&self) -> Option<u16>;

    fn chapter_count(&self) -> Result<u16, Self::Error>;
    fn chapter_summary(
        &self,
        chapter_index: u16,
    ) -> Result<Option<EpubChapterSummary<'_>>, Self::Error>;
    fn cover_summary(&self) -> Result<Option<EpubCoverSummary<'_>>, Self::Error>;

    fn seek_chapter(&mut self, chapter_index: u16) -> Result<(), Self::Error>;
    fn read_text_chunk(&mut self, out: &mut [u8]) -> Result<EpubChunkRead, Self::Error>;
    fn position(&self) -> EpubStreamPosition;
}

/// Errors exposed by the stub EPUB parser.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StubEpubError {
    InvalidBookIndex,
    InvalidChapterIndex,
    NoBookOpened,
}

#[derive(Clone, Copy)]
struct StubChapter {
    title: &'static str,
    text: &'static [u8],
}

#[derive(Clone, Copy)]
struct StubBook {
    title: &'static str,
    author: Option<&'static str>,
    cover: Option<EpubCoverSummary<'static>>,
    chapters: &'static [StubChapter],
}

const BOOK_1_CHAPTERS: [StubChapter; 2] = [
    StubChapter {
        title: "Chapter 1",
        text: b"In a village of La Mancha there lived a gentleman.",
    },
    StubChapter {
        title: "Chapter 2",
        text: b"His habits were old and his books were many.",
    },
];

const BOOK_2_CHAPTERS: [StubChapter; 1] = [StubChapter {
    title: "Opening",
    text: b"Alice was beginning to get very tired of sitting by her sister.",
}];

const STUB_BOOKS: [StubBook; 2] = [
    StubBook {
        title: "Don Quijote (Stub EPUB)",
        author: Some("Miguel de Cervantes"),
        cover: Some(EpubCoverSummary {
            resource_href: "images/cover.pbm",
            media_type: "image/x-portable-bitmap",
            width_px: 240,
            height_px: 320,
            pixel_format: CoverPixelFormat::Mono1Bpp,
        }),
        chapters: &BOOK_1_CHAPTERS,
    },
    StubBook {
        title: "Alice in Wonderland (Stub EPUB)",
        author: Some("Lewis Carroll"),
        cover: None,
        chapters: &BOOK_2_CHAPTERS,
    },
];

/// In-memory stub EPUB parser used to validate app wiring before real parsing.
#[derive(Debug, Clone, Default)]
pub struct StubEpubParser {
    opened_book: Option<usize>,
    chapter_index: usize,
    byte_offset_in_chapter: usize,
}

impl StubEpubParser {
    pub const fn new() -> Self {
        Self {
            opened_book: None,
            chapter_index: 0,
            byte_offset_in_chapter: 0,
        }
    }

    fn opened_book(&self) -> Result<&'static StubBook, StubEpubError> {
        let idx = self.opened_book.ok_or(StubEpubError::NoBookOpened)?;
        Ok(&STUB_BOOKS[idx])
    }

    fn opened_chapter(&self) -> Result<&'static StubChapter, StubEpubError> {
        let book = self.opened_book()?;
        book.chapters
            .get(self.chapter_index)
            .ok_or(StubEpubError::InvalidChapterIndex)
    }
}

impl EpubCatalog for StubEpubParser {
    type Error = StubEpubError;

    fn epub_count(&self) -> Result<u16, Self::Error> {
        Ok(STUB_BOOKS.len().clamp(0, u16::MAX as usize) as u16)
    }

    fn epub_summary(&self, index: u16) -> Result<Option<EpubBookSummary<'_>>, Self::Error> {
        let Some(book) = STUB_BOOKS.get(index as usize) else {
            return Ok(None);
        };

        Ok(Some(EpubBookSummary {
            title: book.title,
            author: book.author,
            chapter_count_hint: book.chapters.len().clamp(0, u16::MAX as usize) as u16,
            has_cover: book.cover.is_some(),
        }))
    }
}

impl EpubReader for StubEpubParser {
    type Error = StubEpubError;

    fn open_epub(&mut self, index: u16) -> Result<(), Self::Error> {
        let idx = index as usize;
        if idx >= STUB_BOOKS.len() {
            return Err(StubEpubError::InvalidBookIndex);
        }

        self.opened_book = Some(idx);
        self.chapter_index = 0;
        self.byte_offset_in_chapter = 0;
        Ok(())
    }

    fn close_epub(&mut self) -> Result<(), Self::Error> {
        self.opened_book = None;
        self.chapter_index = 0;
        self.byte_offset_in_chapter = 0;
        Ok(())
    }

    fn opened_epub(&self) -> Option<u16> {
        self.opened_book.map(|idx| idx as u16)
    }

    fn chapter_count(&self) -> Result<u16, Self::Error> {
        let book = self.opened_book()?;
        Ok(book.chapters.len().clamp(0, u16::MAX as usize) as u16)
    }

    fn chapter_summary(
        &self,
        chapter_index: u16,
    ) -> Result<Option<EpubChapterSummary<'_>>, Self::Error> {
        let book = self.opened_book()?;
        let Some(chapter) = book.chapters.get(chapter_index as usize) else {
            return Ok(None);
        };

        Ok(Some(EpubChapterSummary {
            index: chapter_index,
            title: chapter.title,
        }))
    }

    fn cover_summary(&self) -> Result<Option<EpubCoverSummary<'_>>, Self::Error> {
        let book = self.opened_book()?;
        Ok(book.cover)
    }

    fn seek_chapter(&mut self, chapter_index: u16) -> Result<(), Self::Error> {
        let book = self.opened_book()?;
        let idx = chapter_index as usize;
        if idx >= book.chapters.len() {
            return Err(StubEpubError::InvalidChapterIndex);
        }

        self.chapter_index = idx;
        self.byte_offset_in_chapter = 0;
        Ok(())
    }

    fn read_text_chunk(&mut self, out: &mut [u8]) -> Result<EpubChunkRead, Self::Error> {
        if out.is_empty() {
            return Ok(EpubChunkRead {
                bytes_written: 0,
                chapter_break: false,
                end_of_book: false,
            });
        }

        let book = self.opened_book()?;
        let chapter = self.opened_chapter()?;

        if self.byte_offset_in_chapter >= chapter.text.len() {
            let last_chapter = self.chapter_index + 1 >= book.chapters.len();
            if last_chapter {
                return Ok(EpubChunkRead {
                    bytes_written: 0,
                    chapter_break: true,
                    end_of_book: true,
                });
            }

            self.chapter_index += 1;
            self.byte_offset_in_chapter = 0;
            return Ok(EpubChunkRead {
                bytes_written: 0,
                chapter_break: true,
                end_of_book: false,
            });
        }

        let remaining = &chapter.text[self.byte_offset_in_chapter..];
        let to_copy = remaining.len().min(out.len());
        out[..to_copy].copy_from_slice(&remaining[..to_copy]);
        self.byte_offset_in_chapter = self.byte_offset_in_chapter.saturating_add(to_copy);

        Ok(EpubChunkRead {
            bytes_written: to_copy,
            chapter_break: false,
            end_of_book: false,
        })
    }

    fn position(&self) -> EpubStreamPosition {
        EpubStreamPosition {
            chapter_index: self.chapter_index.clamp(0, u16::MAX as usize) as u16,
            byte_offset_in_chapter: self.byte_offset_in_chapter.clamp(0, u32::MAX as usize) as u32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_catalog_exposes_titles_and_cover_flags() {
        let parser = StubEpubParser::new();
        assert_eq!(parser.epub_count().unwrap(), 2);

        let first = parser.epub_summary(0).unwrap().unwrap();
        assert_eq!(first.title, "Don Quijote (Stub EPUB)");
        assert!(first.has_cover);

        let second = parser.epub_summary(1).unwrap().unwrap();
        assert_eq!(second.title, "Alice in Wonderland (Stub EPUB)");
        assert!(!second.has_cover);
    }

    #[test]
    fn stub_reader_streams_in_chunks() {
        let mut parser = StubEpubParser::new();
        parser.open_epub(0).unwrap();

        let mut buf = [0u8; 12];
        let first = parser.read_text_chunk(&mut buf).unwrap();
        assert_eq!(first.bytes_written, 12);
        assert!(!first.chapter_break);

        let position = parser.position();
        assert_eq!(position.chapter_index, 0);
        assert!(position.byte_offset_in_chapter > 0);
    }

    #[test]
    fn stub_reader_reports_chapter_break_and_end() {
        let mut parser = StubEpubParser::new();
        parser.open_epub(1).unwrap();

        let mut buf = [0u8; 128];
        loop {
            let read = parser.read_text_chunk(&mut buf).unwrap();
            if read.chapter_break {
                assert!(read.end_of_book);
                break;
            }
        }
    }
}
