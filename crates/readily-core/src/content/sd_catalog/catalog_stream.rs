use super::{
    parsing_utils::{path_is_plain_text, update_chapter_label_from_resource},
    sanitize_chunk::sanitize_epub_chunk,
    *,
};

impl SdCatalogSource {
    pub fn set_catalog_text_chunk_from_bytes(
        &mut self,
        index: u16,
        chunk: &[u8],
        end_of_stream: bool,
        resource_path: &str,
    ) -> Result<SdCatalogTextLoadResult, SdCatalogError> {
        let idx = index as usize;
        let slot = self
            .catalog_text_chunks
            .get_mut(idx)
            .ok_or(SdCatalogError::InvalidTextIndex)?;
        let stream_mode = self
            .catalog_stream_mode
            .get_mut(idx)
            .ok_or(SdCatalogError::InvalidTextIndex)?;
        let stream_end = self
            .catalog_stream_end
            .get_mut(idx)
            .ok_or(SdCatalogError::InvalidTextIndex)?;
        let stream_terminal = self
            .catalog_stream_terminal
            .get_mut(idx)
            .ok_or(SdCatalogError::InvalidTextIndex)?;
        let chapter_index = self
            .catalog_stream_chapter_index
            .get_mut(idx)
            .ok_or(SdCatalogError::InvalidTextIndex)?;
        let chapter_total_hint = self
            .catalog_stream_chapter_total_hint
            .get_mut(idx)
            .ok_or(SdCatalogError::InvalidTextIndex)?;
        let chapter_label = self
            .catalog_stream_chapter_label
            .get_mut(idx)
            .ok_or(SdCatalogError::InvalidTextIndex)?;
        let html_state = self
            .catalog_html_state
            .get_mut(idx)
            .ok_or(SdCatalogError::InvalidTextIndex)?;
        let html_tail = self
            .catalog_html_tail
            .get_mut(idx)
            .ok_or(SdCatalogError::InvalidTextIndex)?;
        let refill_requested = self
            .catalog_refill_requested
            .get_mut(idx)
            .ok_or(SdCatalogError::InvalidTextIndex)?;
        let stream_path = self
            .catalog_stream_paths
            .get_mut(idx)
            .ok_or(SdCatalogError::InvalidTextIndex)?;
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
            let carry_start = Self::carry_start_for_tail(&parse_input[..parse_len], start);
            for &byte in &parse_input[carry_start..parse_len] {
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
        debug!(
            "sd-stream: chunk-set idx={} resource={} in_bytes={} loaded={} truncated={} stream_mode={} end={} terminal={} chapter={}/{} chapter_label={} html_flags=0x{:02x} tail_bytes={} parsed_paragraphs={} first_paragraph={:?} last_paragraph={:?}",
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
            last_paragraph
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

    fn carry_start_for_tail(parse_input: &[u8], tail_start: usize) -> usize {
        if tail_start == 0 || tail_start > parse_input.len() {
            return tail_start.min(parse_input.len());
        }

        let mut carry_start = tail_start;
        while carry_start > 0 {
            let prev = parse_input[carry_start - 1];
            let part_of_word =
                prev.is_ascii_alphanumeric() || prev == b'_' || prev == b'-' || prev >= 0x80;
            if !part_of_word {
                break;
            }
            carry_start -= 1;
        }
        carry_start
    }

    pub fn mark_catalog_stream_exhausted(&mut self, index: u16) -> Result<(), SdCatalogError> {
        let idx = index as usize;
        let Some(stream_end) = self.catalog_stream_end.get_mut(idx) else {
            return Err(SdCatalogError::InvalidTextIndex);
        };
        let Some(refill_requested) = self.catalog_refill_requested.get_mut(idx) else {
            return Err(SdCatalogError::InvalidTextIndex);
        };
        let Some(stream_terminal) = self.catalog_stream_terminal.get_mut(idx) else {
            return Err(SdCatalogError::InvalidTextIndex);
        };
        let Some(seek_target) = self.catalog_stream_seek_target.get_mut(idx) else {
            return Err(SdCatalogError::InvalidTextIndex);
        };
        let Some(chapter_index) = self.catalog_stream_chapter_index.get(idx).copied() else {
            return Err(SdCatalogError::InvalidTextIndex);
        };
        let Some(chapter_total_hint) = self.catalog_stream_chapter_total_hint.get_mut(idx) else {
            return Err(SdCatalogError::InvalidTextIndex);
        };
        let Some(html_tail) = self.catalog_html_tail.get_mut(idx) else {
            return Err(SdCatalogError::InvalidTextIndex);
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

    pub fn set_catalog_stream_chapter_metadata(
        &mut self,
        index: u16,
        chapter_index: u16,
        chapter_total: u16,
        chapter_label: Option<&str>,
    ) -> Result<(), SdCatalogError> {
        let idx = index as usize;
        let Some(chapter_idx_slot) = self.catalog_stream_chapter_index.get_mut(idx) else {
            return Err(SdCatalogError::InvalidTextIndex);
        };
        let Some(chapter_total_slot) = self.catalog_stream_chapter_total_hint.get_mut(idx) else {
            return Err(SdCatalogError::InvalidTextIndex);
        };
        let Some(chapter_label_slot) = self.catalog_stream_chapter_label.get_mut(idx) else {
            return Err(SdCatalogError::InvalidTextIndex);
        };

        let total = chapter_total.max(1);
        *chapter_total_slot = total;
        *chapter_idx_slot = chapter_index.min(total.saturating_sub(1));
        if let Some(label) = chapter_label.filter(|value| !value.trim().is_empty()) {
            chapter_label_slot.clear();
            for ch in label.chars() {
                if chapter_label_slot.push(ch).is_err() {
                    break;
                }
            }
        }
        debug!(
            "sd-stream: chapter-hint idx={} chapter={}/{} label={:?}",
            idx,
            chapter_idx_slot.saturating_add(1),
            *chapter_total_slot,
            chapter_label_slot.as_str()
        );
        Ok(())
    }

    pub fn set_catalog_stream_chapter_hint(
        &mut self,
        index: u16,
        chapter_index: u16,
        chapter_total: u16,
    ) -> Result<(), SdCatalogError> {
        self.set_catalog_stream_chapter_metadata(index, chapter_index, chapter_total, None)
    }
}
