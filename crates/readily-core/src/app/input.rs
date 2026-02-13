impl<WS, IN> ReaderApp<WS, IN>
where
    WS: WordSource + TextCatalog + SelectableWordSource + ParagraphNavigator + NavigationCatalog,
    IN: InputProvider,
{
    fn process_inputs(&mut self, now_ms: u64) {
        loop {
            match self.input.poll_event() {
                Ok(Some(event)) => self.apply_input_event(event, now_ms),
                Ok(None) => break,
                Err(_) => {
                    self.set_status("INPUT ERROR", "CHECK PROVIDER", now_ms);
                    break;
                }
            }
        }
    }

    fn apply_input_event(&mut self, event: InputEvent, now_ms: u64) {
        match self.ui {
            UiState::Library { cursor } => self.apply_library_input(cursor, event, now_ms),
            UiState::Settings { cursor, editing } => {
                self.apply_settings_input(cursor, editing, event, now_ms)
            }
            UiState::Countdown {
                selected_book,
                remaining,
                next_step_ms,
            } => self.apply_countdown_input(selected_book, remaining, next_step_ms, event, now_ms),
            UiState::Reading {
                selected_book,
                paused,
                next_word_ms,
            } => self.apply_reading_input(selected_book, paused, next_word_ms, event, now_ms),
            UiState::NavigateChapter {
                selected_book,
                chapter_cursor,
            } => self.apply_chapter_navigation_input(selected_book, chapter_cursor, event, now_ms),
            UiState::NavigateChapterLoading {
                selected_book,
                chapter_index,
            } => self.apply_chapter_loading_input(selected_book, chapter_index, event, now_ms),
            UiState::NavigateParagraph {
                selected_book,
                chapter_index,
                paragraph_cursor,
            } => self.apply_paragraph_navigation_input(
                selected_book,
                chapter_index,
                paragraph_cursor,
                event,
                now_ms,
            ),
            UiState::Status { .. } => {
                if matches!(event, InputEvent::Press) {
                    self.enter_library(self.content.selected_index(), now_ms);
                }
            }
        }
    }

    fn apply_library_input(&mut self, cursor: u16, event: InputEvent, now_ms: u64) {
        let total_items = self.library_item_count().max(1);

        match event {
            InputEvent::RotateCw => {
                self.ui = UiState::Library {
                    cursor: rotate_cw(cursor, total_items),
                };
                self.start_transition(AnimationKind::SlideLeft, now_ms, 120);
                self.pending_redraw = true;
            }
            InputEvent::RotateCcw => {
                self.ui = UiState::Library {
                    cursor: rotate_ccw(cursor, total_items),
                };
                self.start_transition(AnimationKind::SlideRight, now_ms, 120);
                self.pending_redraw = true;
            }
            InputEvent::Press => {
                let settings_index = self.settings_item_index();
                if cursor == settings_index {
                    self.enter_settings(0, false, now_ms);
                    return;
                }

                if self.content.select_text(cursor).is_err() {
                    self.set_status("CONTENT ERROR", "INVALID TITLE", now_ms);
                    return;
                }

                self.enter_countdown(cursor, now_ms);
            }
        }
    }

    fn apply_settings_input(&mut self, cursor: u8, editing: bool, event: InputEvent, now_ms: u64) {
        if editing {
            match event {
                InputEvent::Press => self.enter_settings(cursor, false, now_ms),
                InputEvent::RotateCw => {
                    self.rotate_setting(SettingsRow::from_index(cursor), true);
                    self.pending_redraw = true;
                }
                InputEvent::RotateCcw => {
                    self.rotate_setting(SettingsRow::from_index(cursor), false);
                    self.pending_redraw = true;
                }
            }
            return;
        }

        match event {
            InputEvent::RotateCw => {
                let next = rotate_cw(cursor as u16, SettingsRow::COUNT as u16) as u8;
                self.enter_settings(next, false, now_ms);
            }
            InputEvent::RotateCcw => {
                let next = rotate_ccw(cursor as u16, SettingsRow::COUNT as u16) as u8;
                self.enter_settings(next, false, now_ms);
            }
            InputEvent::Press => {
                let row = SettingsRow::from_index(cursor);
                if matches!(row, SettingsRow::Back) {
                    self.enter_library(self.settings_item_index(), now_ms);
                } else {
                    self.enter_settings(cursor, true, now_ms);
                }
            }
        }
    }

    fn apply_countdown_input(
        &mut self,
        selected_book: u16,
        remaining: u8,
        next_step_ms: u64,
        event: InputEvent,
        now_ms: u64,
    ) {
        match event {
            InputEvent::Press => self.enter_reading(selected_book, now_ms),
            InputEvent::RotateCw => {
                if self.adjust_wpm(true) {
                    self.ui = UiState::Countdown {
                        selected_book,
                        remaining,
                        next_step_ms,
                    };
                    self.pending_redraw = true;
                }
            }
            InputEvent::RotateCcw => {
                if self.adjust_wpm(false) {
                    self.ui = UiState::Countdown {
                        selected_book,
                        remaining,
                        next_step_ms,
                    };
                    self.pending_redraw = true;
                }
            }
        }
    }

    fn apply_reading_input(
        &mut self,
        selected_book: u16,
        paused: bool,
        next_word_ms: u64,
        event: InputEvent,
        now_ms: u64,
    ) {
        match event {
            InputEvent::Press => {
                let double_press = self
                    .last_reading_press_ms
                    .is_some_and(|last| now_ms.saturating_sub(last) <= EXIT_DOUBLE_PRESS_MS);
                self.last_reading_press_ms = Some(now_ms);

                if double_press {
                    self.last_reading_press_ms = None;
                    self.enter_library(selected_book, now_ms);
                    return;
                }

                self.ui = UiState::Reading {
                    selected_book,
                    paused: !paused,
                    next_word_ms,
                };
                if paused {
                    self.paused_since_ms = None;
                } else {
                    self.paused_since_ms = Some(now_ms);
                }
                self.last_pause_anim_slot = None;
                self.pending_redraw = true;
            }
            InputEvent::RotateCw => {
                if paused {
                    let chapter_total = self.content.chapter_count().max(1);
                    let current_chapter = self.current_chapter_index();
                    let next_chapter = rotate_cw(current_chapter, chapter_total);
                    debug!(
                        "ui-nav: paused rotate_cw selected_book={} current_chapter={}/{} next_chapter={}/{}",
                        selected_book,
                        current_chapter.saturating_add(1),
                        chapter_total,
                        next_chapter.saturating_add(1),
                        chapter_total
                    );
                    self.enter_chapter_navigation(selected_book, next_chapter, now_ms);
                    return;
                }

                if self.adjust_wpm(true) {
                    self.ui = UiState::Reading {
                        selected_book,
                        paused,
                        next_word_ms: if paused { next_word_ms } else { now_ms },
                    };
                    self.pending_redraw = true;
                }
            }
            InputEvent::RotateCcw => {
                if paused {
                    let chapter_total = self.content.chapter_count().max(1);
                    let current_chapter = self.current_chapter_index();
                    let next_chapter = rotate_ccw(current_chapter, chapter_total);
                    debug!(
                        "ui-nav: paused rotate_ccw selected_book={} current_chapter={}/{} next_chapter={}/{}",
                        selected_book,
                        current_chapter.saturating_add(1),
                        chapter_total,
                        next_chapter.saturating_add(1),
                        chapter_total
                    );
                    self.enter_chapter_navigation(selected_book, next_chapter, now_ms);
                    return;
                }

                if self.adjust_wpm(false) {
                    self.ui = UiState::Reading {
                        selected_book,
                        paused,
                        next_word_ms: if paused { next_word_ms } else { now_ms },
                    };
                    self.pending_redraw = true;
                }
            }
        }
    }

    fn apply_chapter_navigation_input(
        &mut self,
        selected_book: u16,
        chapter_cursor: u16,
        event: InputEvent,
        now_ms: u64,
    ) {
        let chapter_total = self.content.chapter_count().max(1);
        let chapter_cursor = chapter_cursor.min(chapter_total.saturating_sub(1));

        match event {
            InputEvent::RotateCw => {
                let next = rotate_cw(chapter_cursor, chapter_total);
                debug!(
                    "ui-nav: chapter rotate_cw selected_book={} chapter_cursor={}/{} -> {}/{}",
                    selected_book,
                    chapter_cursor.saturating_add(1),
                    chapter_total,
                    next.saturating_add(1),
                    chapter_total
                );
                self.ui = UiState::NavigateChapter {
                    selected_book,
                    chapter_cursor: next,
                };
                self.start_transition(AnimationKind::SlideLeft, now_ms, ANIM_NAV_ROTATE_MS);
                self.pending_redraw = true;
            }
            InputEvent::RotateCcw => {
                let next = rotate_ccw(chapter_cursor, chapter_total);
                debug!(
                    "ui-nav: chapter rotate_ccw selected_book={} chapter_cursor={}/{} -> {}/{}",
                    selected_book,
                    chapter_cursor.saturating_add(1),
                    chapter_total,
                    next.saturating_add(1),
                    chapter_total
                );
                self.ui = UiState::NavigateChapter {
                    selected_book,
                    chapter_cursor: next,
                };
                self.start_transition(AnimationKind::SlideRight, now_ms, ANIM_NAV_ROTATE_MS);
                self.pending_redraw = true;
            }
            InputEvent::Press => {
                let Some(chapter_info) = self.content.chapter_at(chapter_cursor) else {
                    self.set_status("NAVIGATION ERROR", "CHAPTER INVALID", now_ms);
                    return;
                };

                debug!(
                    "ui-nav: chapter press selected_book={} chapter_cursor={}/{} label={:?}",
                    selected_book,
                    chapter_cursor.saturating_add(1),
                    chapter_total,
                    chapter_info.label,
                );

                match self.content.seek_chapter(chapter_cursor) {
                    Ok(true) => {
                        debug!(
                            "ui-nav: chapter press seek accepted selected_book={} chapter_cursor={}/{}",
                            selected_book,
                            chapter_cursor.saturating_add(1),
                            chapter_total
                        );
                    }
                    Ok(false) => {
                        debug!(
                            "ui-nav: chapter press seek unsupported selected_book={} chapter_cursor={}/{}",
                            selected_book,
                            chapter_cursor.saturating_add(1),
                            chapter_total
                        );
                    }
                    Err(_) => {
                        debug!(
                            "ui-nav: chapter press seek failed selected_book={} chapter_cursor={}/{}",
                            selected_book,
                            chapter_cursor.saturating_add(1),
                            chapter_total
                        );
                        self.set_status("NAVIGATION ERROR", "CHAPTER SEEK FAILED", now_ms);
                        return;
                    }
                }

                if self.content.chapter_data_ready(chapter_cursor) {
                    let initial_cursor = self.initial_paragraph_cursor_for_chapter(chapter_cursor);
                    self.enter_paragraph_navigation(
                        selected_book,
                        chapter_cursor,
                        initial_cursor,
                        now_ms,
                    );
                } else {
                    self.enter_chapter_loading(selected_book, chapter_cursor, now_ms);
                }
            }
        }
    }

    fn apply_chapter_loading_input(
        &mut self,
        selected_book: u16,
        chapter_index: u16,
        event: InputEvent,
        _now_ms: u64,
    ) {
        debug!(
            "ui-nav: chapter loading ignored input selected_book={} chapter_index={} event={:?}",
            selected_book,
            chapter_index.saturating_add(1),
            event
        );
    }

    fn apply_paragraph_navigation_input(
        &mut self,
        selected_book: u16,
        chapter_index: u16,
        paragraph_cursor: u16,
        event: InputEvent,
        now_ms: u64,
    ) {
        let Some(chapter) = self.content.chapter_at(chapter_index) else {
            self.set_status("NAVIGATION ERROR", "CHAPTER INVALID", now_ms);
            return;
        };

        let chapter_start = chapter.start_paragraph;
        let chapter_total = chapter.paragraph_count.max(1);
        let chapter_end = chapter_start.saturating_add(chapter_total.saturating_sub(1));
        let paragraph_cursor = paragraph_cursor.clamp(chapter_start, chapter_end);

        match event {
            InputEvent::RotateCw => {
                let rel = paragraph_cursor.saturating_sub(chapter_start);
                let next_rel = rotate_cw(rel, chapter_total);
                debug!(
                    "ui-nav: paragraph rotate_cw selected_book={} chapter_index={} cursor={} rel={}/{} -> rel={}",
                    selected_book,
                    chapter_index.saturating_add(1),
                    paragraph_cursor,
                    rel.saturating_add(1),
                    chapter_total,
                    next_rel.saturating_add(1)
                );
                self.ui = UiState::NavigateParagraph {
                    selected_book,
                    chapter_index,
                    paragraph_cursor: chapter_start.saturating_add(next_rel),
                };
                self.start_transition(AnimationKind::SlideLeft, now_ms, ANIM_NAV_ROTATE_MS);
                self.pending_redraw = true;
            }
            InputEvent::RotateCcw => {
                let rel = paragraph_cursor.saturating_sub(chapter_start);
                let next_rel = rotate_ccw(rel, chapter_total);
                debug!(
                    "ui-nav: paragraph rotate_ccw selected_book={} chapter_index={} cursor={} rel={}/{} -> rel={}",
                    selected_book,
                    chapter_index.saturating_add(1),
                    paragraph_cursor,
                    rel.saturating_add(1),
                    chapter_total,
                    next_rel.saturating_add(1)
                );
                self.ui = UiState::NavigateParagraph {
                    selected_book,
                    chapter_index,
                    paragraph_cursor: chapter_start.saturating_add(next_rel),
                };
                self.start_transition(AnimationKind::SlideRight, now_ms, ANIM_NAV_ROTATE_MS);
                self.pending_redraw = true;
            }
            InputEvent::Press => {
                debug!(
                    "ui-nav: paragraph press selected_book={} chapter_index={} target_paragraph={}",
                    selected_book,
                    chapter_index.saturating_add(1),
                    paragraph_cursor
                );
                self.apply_navigation_confirm(selected_book, paragraph_cursor, now_ms)
            }
        }
    }


}
