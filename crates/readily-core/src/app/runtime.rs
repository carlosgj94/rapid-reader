impl<WS, IN> ReaderApp<WS, IN>
where
    WS: WordSource + TextCatalog + SelectableWordSource + ParagraphNavigator + NavigationCatalog,
    IN: InputProvider,
{
    fn tick_chapter_loading(&mut self, now_ms: u64) -> TickResult {
        let (selected_book, chapter_index) = match self.ui {
            UiState::NavigateChapterLoading {
                selected_book,
                chapter_index,
            } => (selected_book, chapter_index),
            _ => return TickResult::NoRender,
        };

        if self.content.chapter_data_ready(chapter_index) {
            if let Some(pending_restore) = self.pending_wake_restore
                && pending_restore.resume.selected_book == selected_book
                && pending_restore.resume.chapter_index == chapter_index
            {
                self.pending_wake_restore = None;
                if self.apply_resume_context(
                    pending_restore.resume,
                    pending_restore.context,
                    now_ms,
                ) {
                    return TickResult::RenderRequested;
                }
                self.set_status("RESUME ERROR", "LOAD POSITION", now_ms);
                return TickResult::RenderRequested;
            }

            let initial_cursor = self.initial_paragraph_cursor_for_chapter(chapter_index);
            self.enter_paragraph_navigation(selected_book, chapter_index, initial_cursor, now_ms);
            return TickResult::RenderRequested;
        }

        if self.pending_redraw {
            self.pending_redraw = false;
            return TickResult::RenderRequested;
        }

        TickResult::NoRender
    }

    fn tick_countdown(&mut self, now_ms: u64) -> TickResult {
        if self.pending_redraw {
            self.pending_redraw = false;
            return TickResult::RenderRequested;
        }

        let (selected_book, mut remaining, mut next_step_ms) = match self.ui {
            UiState::Countdown {
                selected_book,
                remaining,
                next_step_ms,
            } => (selected_book, remaining, next_step_ms),
            _ => return TickResult::NoRender,
        };

        if now_ms < next_step_ms {
            return TickResult::NoRender;
        }

        if remaining > 1 {
            remaining -= 1;
            next_step_ms += 1_000;
            self.ui = UiState::Countdown {
                selected_book,
                remaining,
                next_step_ms,
            };
            self.start_transition(AnimationKind::Pulse, now_ms, 900);
            return TickResult::RenderRequested;
        }

        self.enter_reading(selected_book, now_ms);
        self.tick_reading(now_ms)
    }

    fn tick_reading(&mut self, now_ms: u64) -> TickResult {
        let (selected_book, paused, next_word_ms) = match self.ui {
            UiState::Reading {
                selected_book,
                paused,
                next_word_ms,
            } => (selected_book, paused, next_word_ms),
            _ => return TickResult::NoRender,
        };

        if paused {
            let slot = (now_ms / PAUSE_ANIM_FRAME_MS) as u32;
            if self.pending_redraw || self.last_pause_anim_slot != Some(slot) {
                self.pending_redraw = false;
                self.last_pause_anim_slot = Some(slot);
                return TickResult::RenderRequested;
            }
            return TickResult::NoRender;
        }
        self.last_pause_anim_slot = None;

        if self.pending_redraw && !self.word_buffer.is_empty() {
            self.pending_redraw = false;
            return TickResult::RenderRequested;
        }

        if self.word_buffer.is_empty() || now_ms >= next_word_ms {
            match self.advance_word() {
                Ok(AdvanceWordResult::Advanced) => {
                    self.ui = UiState::Reading {
                        selected_book,
                        paused: false,
                        next_word_ms: now_ms + self.current_word_delay_ms() as u64,
                    };
                    self.pending_redraw = false;
                    self.words_since_drain = self.words_since_drain.saturating_add(1);
                    return TickResult::RenderRequested;
                }
                Ok(AdvanceWordResult::AwaitingRefill) => {
                    self.ui = UiState::Reading {
                        selected_book,
                        paused: false,
                        next_word_ms: now_ms + 40,
                    };
                    self.pending_redraw = false;
                    return TickResult::NoRender;
                }
                Ok(AdvanceWordResult::EndOfText) => {
                    let paragraph_index = self.content.paragraph_index();
                    let paragraph_total = self.content.paragraph_total().max(1);
                    let waiting_for_refill = self.content.is_waiting_for_refill();
                    info!(
                        "ui-nav: reading end-of-text -> paused selected_book={} paragraph={}/{} waiting_for_refill={}",
                        selected_book,
                        paragraph_index,
                        paragraph_total,
                        waiting_for_refill
                    );

                    // If we reached end before rendering any word, treat it as invalid resume/load
                    // position and request chapter 1 so reopening a finished/invalid state starts over.
                    if self.paragraph_word_index == 0 {
                        match self.content.seek_chapter(0) {
                            Ok(true) => {
                                info!(
                                    "ui-nav: reading end-of-text -> restart request selected_book={} chapter=1 reason=no_word_rendered",
                                    selected_book
                                );
                                self.word_buffer.set("LOAD");
                                self.ui = UiState::Reading {
                                    selected_book,
                                    paused: true,
                                    next_word_ms: now_ms,
                                };
                                self.last_reading_press_ms = None;
                                self.paused_since_ms = Some(now_ms);
                                self.last_pause_anim_slot = None;
                                self.pending_redraw = false;
                                return TickResult::RenderRequested;
                            }
                            Ok(false) => {}
                            Err(_) => {
                                info!(
                                    "ui-nav: reading end-of-text -> restart request failed selected_book={} chapter=1",
                                    selected_book
                                );
                            }
                        }
                    }

                    // Defensive behavior: avoid kicking back to library on transient end-of-chunk
                    // races. Keep the user in reading (paused) so refill/navigation can recover.
                    if self.word_buffer.is_empty() {
                        self.word_buffer.set("END");
                    }
                    self.ui = UiState::Reading {
                        selected_book,
                        paused: true,
                        next_word_ms: now_ms,
                    };
                    self.last_reading_press_ms = None;
                    self.paused_since_ms = Some(now_ms);
                    self.last_pause_anim_slot = None;
                    self.pending_redraw = false;
                    return TickResult::RenderRequested;
                }
                Err(()) => {
                    self.set_status("CONTENT ERROR", "CHECK SOURCE", now_ms);
                    self.pending_redraw = false;
                    return TickResult::RenderRequested;
                }
            }
        }

        TickResult::NoRender
    }

    fn advance_word(&mut self) -> Result<AdvanceWordResult, ()> {
        match self.content.next_word() {
            Ok(Some(token)) => {
                let mut staged_word = WordBuffer::<WORD_BUFFER_BYTES>::new();
                let (ends_sentence, ends_clause) = {
                    staged_word.set(token.text);
                    (token.ends_sentence, token.ends_clause)
                };

                self.word_buffer = staged_word;
                self.last_ends_sentence = ends_sentence;
                self.last_ends_clause = ends_clause;

                let (index, total) = self.content.paragraph_progress();
                self.paragraph_word_index = index;
                self.paragraph_word_total = total.max(1);
                Ok(AdvanceWordResult::Advanced)
            }
            Ok(None) => {
                if self.content.is_waiting_for_refill() {
                    Ok(AdvanceWordResult::AwaitingRefill)
                } else {
                    Ok(AdvanceWordResult::EndOfText)
                }
            }
            Err(_) => Err(()),
        }
    }

    fn current_word_delay_ms(&self) -> u32 {
        let base = 60_000u32 / self.config.wpm.max(1) as u32;
        let punctuation = if self.last_ends_sentence {
            self.config.dot_pause_ms as u32
        } else if self.last_ends_clause {
            self.config.comma_pause_ms as u32
        } else {
            0
        };

        base + punctuation
    }

    fn rotate_setting(&mut self, row: SettingsRow, clockwise: bool) {
        match row {
            SettingsRow::Font => {
                self.style.font_family = match (self.style.font_family, clockwise) {
                    (FontFamily::Serif, _) => FontFamily::Pixel,
                    (FontFamily::Pixel, _) => FontFamily::Serif,
                };
            }
            SettingsRow::Size => {
                self.style.font_size = match (self.style.font_size, clockwise) {
                    (FontSize::Small, true) => FontSize::Medium,
                    (FontSize::Medium, true) => FontSize::Large,
                    (FontSize::Large, true) => FontSize::Small,
                    (FontSize::Small, false) => FontSize::Large,
                    (FontSize::Medium, false) => FontSize::Small,
                    (FontSize::Large, false) => FontSize::Medium,
                };
            }
            SettingsRow::Invert => {
                self.style.inverted = !self.style.inverted;
            }
            SettingsRow::Wpm => {
                let _ = self.adjust_wpm(clockwise);
            }
            SettingsRow::Back => {}
        }
    }

    fn adjust_wpm(&mut self, increase: bool) -> bool {
        let next = if increase {
            self.config
                .wpm
                .saturating_add(WPM_STEP)
                .min(self.config.max_wpm)
        } else {
            self.config
                .wpm
                .saturating_sub(WPM_STEP)
                .max(self.config.min_wpm)
        };

        if next != self.config.wpm {
            self.config.wpm = next;
            true
        } else {
            false
        }
    }


}
