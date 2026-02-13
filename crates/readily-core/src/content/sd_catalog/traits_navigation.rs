use super::{parsing_utils::push_u32_as_ascii, *};

impl NavigationCatalog for SdCatalogSource {
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
            return Err(SdCatalogError::InvalidChapterIndex);
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
            return Err(SdCatalogError::InvalidTextIndex);
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

    fn chapter_data_ready(&self, chapter_index: u16) -> bool {
        if !self.selected_is_stream_mode() {
            return true;
        }

        let total = self.selected_stream_chapter_total_hint();
        if chapter_index >= total {
            return false;
        }

        let current = self
            .selected_stream_chapter_index()
            .min(total.saturating_sub(1));
        if chapter_index != current || self.waiting_for_refill {
            return false;
        }

        if self.selected_paragraph_count() > 0 {
            return true;
        }

        self.catalog_stream_terminal
            .get(self.selected_book)
            .copied()
            .unwrap_or(false)
    }

    fn paragraph_preview(&self, paragraph_index: u16) -> Option<&str> {
        self.selected_paragraph_at(paragraph_index as usize)
    }
}
