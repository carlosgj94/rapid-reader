use super::*;

impl SdCatalogSource {
    pub(super) fn selected_fallback_paragraphs(&self) -> &'static [&'static str] {
        if BOOKS.is_empty() {
            &[]
        } else {
            BOOKS[self.selected_book % BOOKS.len()].paragraphs
        }
    }

    pub(super) fn selected_chunk_text(&self) -> Option<&str> {
        self.catalog_text_chunks
            .get(self.selected_book)
            .map(|text| text.as_str())
            .map(str::trim)
            .filter(|text| !text.is_empty())
    }

    pub(super) fn chunk_paragraph_count(chunk: &str) -> usize {
        chunk
            .split('\n')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .count()
    }

    pub(super) fn chunk_paragraph_at(chunk: &str, index: usize) -> Option<&str> {
        chunk
            .split('\n')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .nth(index)
    }

    pub(super) fn selected_is_stream_mode(&self) -> bool {
        self.catalog_stream_mode
            .get(self.selected_book)
            .copied()
            .unwrap_or(false)
    }

    pub(super) fn selected_stream_chapter_index(&self) -> u16 {
        self.catalog_stream_chapter_index
            .get(self.selected_book)
            .copied()
            .unwrap_or(0)
    }

    pub(super) fn selected_stream_chapter_total_hint(&self) -> u16 {
        self.catalog_stream_chapter_total_hint
            .get(self.selected_book)
            .copied()
            .unwrap_or(1)
            .max(1)
    }

    pub(super) fn selected_stream_chapter_label(&self) -> &str {
        self.catalog_stream_chapter_label
            .get(self.selected_book)
            .map(|label| label.as_str())
            .filter(|label| !label.trim().is_empty())
            .unwrap_or("Section")
    }

    pub(super) fn selected_stream_path(&self) -> &str {
        self.catalog_stream_paths
            .get(self.selected_book)
            .map(|path| path.as_str())
            .unwrap_or("")
    }

    pub(super) fn selected_paragraph_count(&self) -> usize {
        if let Some(chunk_text) = self.selected_chunk_text() {
            return Self::chunk_paragraph_count(chunk_text);
        }

        if self.selected_is_stream_mode() {
            0
        } else {
            self.selected_fallback_paragraphs().len()
        }
    }

    pub(super) fn selected_paragraph_at(&self, index: usize) -> Option<&str> {
        if let Some(chunk_text) = self.selected_chunk_text() {
            return Self::chunk_paragraph_at(chunk_text, index);
        }

        if self.selected_is_stream_mode() {
            return None;
        }

        self.selected_fallback_paragraphs().get(index).copied()
    }

    pub(super) fn compute_current_word_total(&self) -> u16 {
        let Some(paragraph) = self.selected_paragraph_at(self.paragraph_index) else {
            return 1;
        };

        let count = count_words(paragraph);
        count.clamp(1, u16::MAX as usize) as u16
    }

    pub(super) fn reset_read_pointer(&mut self) {
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

    pub(super) fn advance_paragraph(&mut self) -> bool {
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
