impl<WS, IN> ReaderApp<WS, IN>
where
    WS: WordSource + TextCatalog + SelectableWordSource + ParagraphNavigator + NavigationCatalog,
    IN: InputProvider,
{
    fn enter_library(&mut self, cursor: u16, now_ms: u64) {
        self.last_reading_press_ms = None;
        let max_index = self.library_item_count().saturating_sub(1);
        self.ui = UiState::Library {
            cursor: cursor.min(max_index),
        };
        self.start_transition(AnimationKind::SlideRight, now_ms, ANIM_MENU_MS);
        self.pending_redraw = true;
    }

    fn enter_settings(&mut self, cursor: u8, editing: bool, now_ms: u64) {
        self.ui = UiState::Settings { cursor, editing };
        self.start_transition(AnimationKind::SlideLeft, now_ms, ANIM_MENU_MS);
        self.pending_redraw = true;
    }

    fn enter_countdown(&mut self, selected_book: u16, now_ms: u64) {
        self.last_reading_press_ms = None;
        self.paused_since_ms = None;
        self.last_pause_anim_slot = None;
        self.word_buffer.clear();
        self.paragraph_word_index = 0;
        self.paragraph_word_total = 1;
        self.last_ends_clause = false;
        self.last_ends_sentence = false;

        self.ui = UiState::Countdown {
            selected_book,
            remaining: self.countdown_seconds,
            next_step_ms: now_ms + 1_000,
        };
        self.start_transition(AnimationKind::Pulse, now_ms, 900);
        self.pending_redraw = true;
    }

    fn enter_reading(&mut self, selected_book: u16, now_ms: u64) {
        self.last_reading_press_ms = None;
        self.paused_since_ms = None;
        self.last_pause_anim_slot = None;
        self.ui = UiState::Reading {
            selected_book,
            paused: false,
            next_word_ms: now_ms,
        };
        self.start_transition(AnimationKind::Fade, now_ms, ANIM_SCREEN_MS);
        self.pending_redraw = true;
    }

    fn enter_chapter_navigation(&mut self, selected_book: u16, chapter_cursor: u16, now_ms: u64) {
        self.last_reading_press_ms = None;
        self.last_pause_anim_slot = None;
        let chapter_total = self.content.chapter_count().max(1);
        debug!(
            "ui-nav: enter chapter navigation selected_book={} chapter_cursor={}/{}",
            selected_book,
            chapter_cursor.saturating_add(1),
            chapter_total
        );
        self.ui = UiState::NavigateChapter {
            selected_book,
            chapter_cursor: chapter_cursor.min(chapter_total.saturating_sub(1)),
        };
        self.start_transition(AnimationKind::SlideLeft, now_ms, ANIM_NAV_MS);
        self.pending_redraw = true;
    }

    fn enter_paragraph_navigation(
        &mut self,
        selected_book: u16,
        chapter_index: u16,
        paragraph_cursor: u16,
        now_ms: u64,
    ) {
        let Some(chapter) = self.content.chapter_at(chapter_index) else {
            self.set_status("NAVIGATION ERROR", "CHAPTER INVALID", now_ms);
            return;
        };

        let chapter_start = chapter.start_paragraph;
        let chapter_end = chapter_start.saturating_add(chapter.paragraph_count.saturating_sub(1));
        debug!(
            "ui-nav: enter paragraph navigation selected_book={} chapter_index={} label={:?} chapter_start={} chapter_end={} requested_cursor={}",
            selected_book,
            chapter_index.saturating_add(1),
            chapter.label,
            chapter_start,
            chapter_end,
            paragraph_cursor
        );

        self.ui = UiState::NavigateParagraph {
            selected_book,
            chapter_index,
            paragraph_cursor: paragraph_cursor.clamp(chapter_start, chapter_end),
        };
        self.start_transition(AnimationKind::SlideLeft, now_ms, ANIM_NAV_MS);
        self.pending_redraw = true;
    }

    fn apply_navigation_confirm(&mut self, selected_book: u16, target_paragraph: u16, now_ms: u64) {
        debug!(
            "ui-nav: confirm selected_book={} target_paragraph={}",
            selected_book, target_paragraph
        );
        if self.content.seek_paragraph(target_paragraph).is_err() {
            debug!(
                "ui-nav: confirm failed selected_book={} target_paragraph={} status=invalid_paragraph",
                selected_book, target_paragraph
            );
            self.set_status("NAVIGATION ERROR", "PARAGRAPH INVALID", now_ms);
            return;
        }
        debug!(
            "ui-nav: confirm applied selected_book={} target_paragraph={} paragraph_index={} paragraph_total={} chapter={}/{} chapter_label={:?} preview={:?}",
            selected_book,
            target_paragraph,
            self.content.paragraph_index(),
            self.content.paragraph_total(),
            self.current_chapter_index().saturating_add(1),
            self.content.chapter_count().max(1),
            self.content
                .chapter_at(self.current_chapter_index())
                .map(|chapter| chapter.label),
            self.content.paragraph_preview(target_paragraph)
        );

        self.word_buffer.clear();
        self.paragraph_word_index = 0;
        self.paragraph_word_total = 1;
        self.last_ends_clause = false;
        self.last_ends_sentence = false;

        let _ = self.advance_word();

        self.ui = UiState::Reading {
            selected_book,
            paused: true,
            next_word_ms: now_ms,
        };
        self.paused_since_ms = Some(now_ms);
        self.last_pause_anim_slot = None;
        self.start_transition(AnimationKind::SlideRight, now_ms, ANIM_NAV_MS);
        self.pending_redraw = true;
    }

    fn set_status(&mut self, line1: &'static str, line2: &'static str, now_ms: u64) {
        self.last_reading_press_ms = None;
        self.ui = UiState::Status { line1, line2 };
        self.start_transition(AnimationKind::Fade, now_ms, ANIM_SCREEN_MS);
        self.pending_redraw = true;
    }

    fn start_transition(&mut self, kind: AnimationKind, now_ms: u64, duration_ms: u16) {
        self.transition = Some(AnimationSpec::new(kind, now_ms, duration_ms));
    }

    fn transition_frame(&self, now_ms: u64) -> Option<crate::render::AnimationFrame> {
        self.transition.and_then(|anim| anim.frame(now_ms))
    }

    fn total_title_count(&self) -> u16 {
        self.content.title_count()
    }

    fn library_item_count(&self) -> u16 {
        self.total_title_count().saturating_add(1)
    }

    fn settings_item_index(&self) -> u16 {
        self.total_title_count()
    }

    fn chapter_for_paragraph(&self, paragraph_index: u16) -> u16 {
        let chapter_count = self.content.chapter_count().max(1);

        for chapter_idx in 0..chapter_count {
            if let Some(chapter) = self.content.chapter_at(chapter_idx) {
                let start = chapter.start_paragraph;
                let end = start.saturating_add(chapter.paragraph_count.saturating_sub(1));
                if (start..=end).contains(&paragraph_index) {
                    return chapter_idx;
                }
            }
        }

        0
    }

    fn current_chapter_index(&self) -> u16 {
        if let Some(index) = self.content.current_chapter_index() {
            return index.min(self.content.chapter_count().saturating_sub(1));
        }

        let current_paragraph = self.content.paragraph_index().saturating_sub(1);
        self.chapter_for_paragraph(current_paragraph)
    }
}
