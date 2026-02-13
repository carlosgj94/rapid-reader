use super::{parsing_utils::next_word_bounds, *};

impl WordSource for SdCatalogSource {
    type Error = SdCatalogError;

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

impl SelectableWordSource for SdCatalogSource {
    fn select_text(&mut self, index: u16) -> Result<(), Self::Error> {
        let idx = index as usize;
        if idx >= self.catalog_titles.len() {
            return Err(SdCatalogError::InvalidTextIndex);
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

impl ParagraphNavigator for SdCatalogSource {
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
            return Err(SdCatalogError::InvalidParagraphIndex);
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
