use super::*;

impl SdCatalogSource {
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
        self.clear_catalog_state();

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

        self.finalize_catalog_load(truncated)
    }

    pub fn set_catalog_display_titles_from_iter<'a, I>(&mut self, titles: I) -> SdCatalogLoadResult
    where
        I: IntoIterator<Item = &'a str>,
    {
        self.clear_catalog_state();

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

        self.finalize_catalog_load(truncated)
    }

    pub fn set_catalog_entries_from_iter<'a, I>(&mut self, entries: I) -> SdCatalogLoadResult
    where
        I: IntoIterator<Item = (&'a str, bool)>,
    {
        self.clear_catalog_state();

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

        self.finalize_catalog_load(truncated)
    }

    fn clear_catalog_state(&mut self) {
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
    }

    fn finalize_catalog_load(&mut self, truncated: bool) -> SdCatalogLoadResult {
        if self.selected_book >= self.catalog_titles.len() {
            self.selected_book = 0;
        }
        self.reset_read_pointer();

        SdCatalogLoadResult {
            loaded: self.catalog_titles.len().clamp(0, u16::MAX as usize) as u16,
            truncated,
        }
    }

    fn reset_catalog_titles_to_defaults(&mut self) {
        self.clear_catalog_state();
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

        truncated | self.push_catalog_entry(label, has_cover)
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

        truncated | self.push_catalog_entry(label, has_cover)
    }

    fn push_catalog_entry(
        &mut self,
        label: String<SD_CATALOG_TITLE_BYTES>,
        has_cover: bool,
    ) -> bool {
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

        false
    }
}
