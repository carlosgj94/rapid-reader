impl<WS, IN> ReaderApp<WS, IN>
where
    WS: WordSource + TextCatalog + SelectableWordSource + ParagraphNavigator + NavigationCatalog,
    IN: InputProvider,
{
    pub fn new(
        mut content: WS,
        input: IN,
        mut config: ReaderConfig,
        app_title: &'static str,
        countdown_seconds: u8,
    ) -> Self {
        if config.max_wpm < config.min_wpm {
            core::mem::swap(&mut config.max_wpm, &mut config.min_wpm);
        }
        config.wpm = config.wpm.clamp(config.min_wpm, config.max_wpm);

        let initial_index = content
            .selected_index()
            .min(content.title_count().saturating_sub(1));
        let _ = content.select_text(initial_index);

        Self {
            content,
            input,
            config,
            app_title,
            countdown_seconds: countdown_seconds.max(1),
            style: VisualStyle::default(),
            ui: UiState::Library {
                cursor: initial_index,
            },
            pending_redraw: true,
            transition: None,
            word_buffer: WordBuffer::new(),
            paragraph_word_index: 0,
            paragraph_word_total: 1,
            last_ends_sentence: false,
            last_ends_clause: false,
            words_since_drain: 0,
            last_reading_press_ms: None,
            paused_since_ms: None,
            last_pause_anim_slot: None,
            last_input_activity_ms: 0,
            pending_wake_restore: None,
        }
    }

    pub fn tick(&mut self, now_ms: u64) -> TickResult {
        self.process_inputs(now_ms);

        let rendered = match self.ui {
            UiState::Countdown { .. } => self.tick_countdown(now_ms),
            UiState::Reading { .. } => self.tick_reading(now_ms),
            UiState::NavigateChapterLoading { .. } => self.tick_chapter_loading(now_ms),
            UiState::Library { .. }
            | UiState::Settings { .. }
            | UiState::NavigateChapter { .. }
            | UiState::NavigateParagraph { .. }
            | UiState::Status { .. } => {
                if self.pending_redraw {
                    self.pending_redraw = false;
                    TickResult::RenderRequested
                } else {
                    TickResult::NoRender
                }
            }
        };

        if self.transition_frame(now_ms).is_some() {
            TickResult::RenderRequested
        } else {
            rendered
        }
    }

    pub fn with_screen<F>(&self, now_ms: u64, f: F)
    where
        F: FnOnce(Screen<'_>),
    {
        let animation = self.transition_frame(now_ms);

        match self.ui {
            UiState::Library { cursor } => {
                let mut items = [MenuItemView::default(); MAX_LIBRARY_ITEMS];
                let mut count = 0usize;

                let total_titles = self.total_title_count() as usize;
                let visible_slots = MAX_LIBRARY_ITEMS.saturating_sub(1);
                let selected_cursor = cursor as usize;
                let selected_book = selected_cursor.min(total_titles.saturating_sub(1));
                let window_start = if total_titles <= visible_slots {
                    0
                } else {
                    selected_book
                        .saturating_sub(visible_slots / 2)
                        .min(total_titles.saturating_sub(visible_slots))
                };
                let window_end = core::cmp::min(total_titles, window_start + visible_slots);

                for idx in window_start..window_end {
                    let label = self.content.title_at(idx as u16).unwrap_or("Untitled");
                    items[count] = MenuItemView {
                        label,
                        kind: MenuItemKind::Text,
                    };
                    count += 1;
                }

                items[count] = MenuItemView {
                    label: "Settings",
                    kind: MenuItemKind::Settings,
                };
                count += 1;

                let settings_cursor = self.settings_item_index();
                let cursor = if cursor == settings_cursor {
                    count.saturating_sub(1)
                } else {
                    selected_book
                        .saturating_sub(window_start)
                        .min(count.saturating_sub(1))
                };
                f(Screen::Library {
                    title: self.app_title,
                    subtitle: "Library",
                    items: &items[..count],
                    cursor,
                    style: self.style,
                    animation,
                });
            }
            UiState::Settings { cursor, editing } => {
                let rows = [
                    SettingRowView {
                        key: "Font",
                        value: SettingValue::Label(font_family_label(self.style.font_family)),
                    },
                    SettingRowView {
                        key: "Size",
                        value: SettingValue::Label(font_size_label(self.style.font_size)),
                    },
                    SettingRowView {
                        key: "Invert",
                        value: SettingValue::Toggle(self.style.inverted),
                    },
                    SettingRowView {
                        key: "WPM",
                        value: SettingValue::Number(self.config.wpm),
                    },
                    SettingRowView {
                        key: "Back",
                        value: SettingValue::Action("Return"),
                    },
                ];

                f(Screen::Settings {
                    title: self.app_title,
                    subtitle: "Settings",
                    rows: &rows,
                    cursor: (cursor as usize).min(rows.len() - 1),
                    editing,
                    style: self.style,
                    animation,
                });
            }
            UiState::Countdown {
                selected_book,
                remaining,
                ..
            } => {
                let title = self.content.title_at(selected_book).unwrap_or("Untitled");
                f(Screen::Countdown {
                    title,
                    cover_slot: selected_book,
                    has_cover: self.content.has_cover_at(selected_book),
                    wpm: self.config.wpm,
                    remaining,
                    style: self.style,
                    animation,
                });
            }
            UiState::Reading {
                selected_book,
                paused,
                ..
            } => {
                let book_title = self.content.title_at(selected_book).unwrap_or("Untitled");
                let paused_elapsed_ms = if paused {
                    now_ms.saturating_sub(self.paused_since_ms.unwrap_or(now_ms)) as u32
                } else {
                    0
                };
                let current_paragraph = self.content.paragraph_index().saturating_sub(1);
                let current_chapter = self.current_chapter_index();
                let chapter_label_raw = self
                    .content
                    .chapter_at(current_chapter)
                    .and_then(|chapter| {
                        if chapter.label.trim().is_empty() {
                            self.content.paragraph_preview(chapter.start_paragraph)
                        } else {
                            Some(chapter.label)
                        }
                    })
                    .or_else(|| self.content.paragraph_preview(current_paragraph))
                    .unwrap_or("Section");
                let mut header_title_buf = [0u8; NAV_LABEL_BYTES];
                let mut pause_chapter_label_buf = [0u8; NAV_LABEL_BYTES];
                let title = preview_limited(chapter_label_raw, &mut header_title_buf, 4, 36);
                let pause_chapter_label = preview_compact(book_title, &mut pause_chapter_label_buf);
                f(Screen::Reading {
                    title,
                    wpm: self.config.wpm,
                    word: self.word_buffer.as_str(),
                    paragraph_word_index: self.paragraph_word_index,
                    paragraph_word_total: self.paragraph_word_total,
                    paused,
                    paused_elapsed_ms,
                    pause_chapter_label,
                    style: self.style,
                    animation,
                });
            }
            UiState::NavigateChapter {
                selected_book,
                chapter_cursor,
            } => {
                let title = self.content.title_at(selected_book).unwrap_or("Untitled");
                let chapter_total = self.content.chapter_count().max(1);
                let chapter_cursor = chapter_cursor.min(chapter_total.saturating_sub(1));
                let current_chapter = self.current_chapter_index();

                let current_label_raw = self
                    .content
                    .chapter_at(current_chapter)
                    .map(|c| c.label)
                    .unwrap_or("Current");
                let target_label_raw = self
                    .content
                    .chapter_at(chapter_cursor)
                    .map(|c| c.label)
                    .unwrap_or("Target");

                let mut current_label_buf = [0u8; NAV_LABEL_BYTES];
                let mut target_label_buf = [0u8; NAV_LABEL_BYTES];
                let mut current_secondary_buf = [0u8; NAV_LABEL_BYTES];
                let mut target_secondary_buf = [0u8; NAV_LABEL_BYTES];
                let current_label = preview_compact(current_label_raw, &mut current_label_buf);
                let target_label = preview_compact(target_label_raw, &mut target_label_buf);
                let current_secondary = section_secondary_label(
                    current_chapter.saturating_add(1),
                    chapter_total,
                    "Current",
                    &mut current_secondary_buf,
                );
                let target_secondary = section_secondary_label(
                    chapter_cursor.saturating_add(1),
                    chapter_total,
                    "Target  Press for paragraphs",
                    &mut target_secondary_buf,
                );

                f(Screen::NavigateChapters {
                    title,
                    wpm: self.config.wpm,
                    current_chapter: current_chapter.saturating_add(1),
                    target_chapter: chapter_cursor.saturating_add(1),
                    chapter_total,
                    current_label,
                    target_label,
                    current_secondary,
                    target_secondary,
                    style: self.style,
                    animation,
                });
            }
            UiState::NavigateChapterLoading {
                selected_book,
                chapter_index,
            } => {
                let title = self.content.title_at(selected_book).unwrap_or("Untitled");
                let chapter_total = self.content.chapter_count().max(1);
                let chapter_index = chapter_index.min(chapter_total.saturating_sub(1));
                let current_chapter = self.current_chapter_index();
                let current_label_raw = self
                    .content
                    .chapter_at(current_chapter)
                    .map(|c| c.label)
                    .unwrap_or("Current");
                let target_label_raw = self
                    .content
                    .chapter_at(chapter_index)
                    .map(|c| c.label)
                    .unwrap_or("Target");

                let mut current_label_buf = [0u8; NAV_LABEL_BYTES];
                let mut target_label_buf = [0u8; NAV_LABEL_BYTES];
                let mut current_secondary_buf = [0u8; NAV_LABEL_BYTES];
                let mut target_secondary_buf = [0u8; NAV_LABEL_BYTES];
                let current_label = preview_compact(current_label_raw, &mut current_label_buf);
                let target_label = preview_compact(target_label_raw, &mut target_label_buf);
                let current_secondary = section_secondary_label(
                    current_chapter.saturating_add(1),
                    chapter_total,
                    "Current",
                    &mut current_secondary_buf,
                );
                let target_secondary = section_secondary_label(
                    chapter_index.saturating_add(1),
                    chapter_total,
                    "Loading chapter...",
                    &mut target_secondary_buf,
                );

                f(Screen::NavigateChapters {
                    title,
                    wpm: self.config.wpm,
                    current_chapter: current_chapter.saturating_add(1),
                    target_chapter: chapter_index.saturating_add(1),
                    chapter_total,
                    current_label,
                    target_label,
                    current_secondary,
                    target_secondary,
                    style: self.style,
                    animation,
                });
            }
            UiState::NavigateParagraph {
                selected_book,
                chapter_index,
                paragraph_cursor,
            } => {
                let title = self.content.title_at(selected_book).unwrap_or("Untitled");
                let chapter = self.content.chapter_at(chapter_index);

                let (chapter_label_raw, chapter_start, chapter_count) = match chapter {
                    Some(info) => (
                        info.label,
                        info.start_paragraph,
                        info.paragraph_count.max(1),
                    ),
                    None => ("Chapter", 0, 1),
                };

                let max_cursor = chapter_start.saturating_add(chapter_count.saturating_sub(1));
                let paragraph_cursor = paragraph_cursor.clamp(chapter_start, max_cursor);
                let current_paragraph = self.content.paragraph_index().saturating_sub(1);

                let target_preview_raw = self
                    .content
                    .paragraph_preview(paragraph_cursor)
                    .unwrap_or("Target paragraph");

                let mut chapter_label_buf = [0u8; NAV_LABEL_BYTES];
                let mut chapter_number_buf = [0u8; 10];
                let mut target_preview_buf = [0u8; NAV_PREVIEW_BYTES];
                let chapter_label = preview_compact(chapter_label_raw, &mut chapter_label_buf);
                let current_preview =
                    chapter_number_label(chapter_index.saturating_add(1), &mut chapter_number_buf);
                let target_preview =
                    preview_limited(target_preview_raw, &mut target_preview_buf, 48, 220);

                let current_index_in_chapter =
                    if current_paragraph >= chapter_start && current_paragraph <= max_cursor {
                        current_paragraph
                            .saturating_sub(chapter_start)
                            .saturating_add(1)
                    } else {
                        1
                    };

                let target_index_in_chapter = paragraph_cursor
                    .saturating_sub(chapter_start)
                    .saturating_add(1);

                let mut current_secondary_buf = [0u8; NAV_LABEL_BYTES];
                let mut target_secondary_buf = [0u8; NAV_LABEL_BYTES];
                let current_secondary = section_secondary_label(
                    current_index_in_chapter,
                    chapter_count,
                    "Current",
                    &mut current_secondary_buf,
                );
                let target_secondary = section_secondary_label(
                    target_index_in_chapter,
                    chapter_count,
                    "Target  Press to jump",
                    &mut target_secondary_buf,
                );

                f(Screen::NavigateParagraphs {
                    title,
                    wpm: self.config.wpm,
                    chapter_label,
                    current_preview,
                    target_preview,
                    current_secondary,
                    target_secondary,
                    target_index_in_chapter,
                    paragraph_total_in_chapter: chapter_count,
                    style: self.style,
                    animation,
                });
            }
            UiState::Status { line1, line2 } => {
                f(Screen::Status {
                    title: self.app_title,
                    wpm: self.config.wpm,
                    line1,
                    line2,
                    style: self.style,
                    animation,
                });
            }
        }
    }

    pub fn with_content_mut<R, F>(&mut self, f: F) -> R
    where
        F: FnOnce(&mut WS) -> R,
    {
        f(&mut self.content)
    }

    pub fn drain_word_updates(&mut self) -> u32 {
        let count = self.words_since_drain;
        self.words_since_drain = 0;
        count
    }

    pub fn persisted_settings(&self) -> PersistedSettings {
        PersistedSettings::new(self.config.wpm, self.style)
    }

    pub fn apply_persisted_settings(&mut self, settings: PersistedSettings) {
        self.style = settings.style;
        self.config.wpm = settings.wpm.clamp(self.config.min_wpm, self.config.max_wpm);
        self.pending_redraw = true;
    }

    pub fn sleep_eligible(&self) -> bool {
        !matches!(
            self.ui,
            UiState::Reading {
                paused: false,
                ..
            }
        )
    }

    pub fn inactivity_sleep_due(&self, now_ms: u64, timeout_ms: u64) -> bool {
        self.sleep_eligible()
            && now_ms.saturating_sub(self.last_input_activity_ms) >= timeout_ms
    }

    pub fn export_resume_state(&self) -> Option<ResumeState> {
        let title_total = self.total_title_count();
        if title_total == 0 {
            return None;
        }

        let chapter_index = self.current_chapter_index();
        let paragraph_global = self.content.paragraph_index().saturating_sub(1);
        let paragraph_in_chapter = self
            .content
            .chapter_at(chapter_index)
            .map(|chapter| {
                let start = chapter.start_paragraph;
                let end = start.saturating_add(chapter.paragraph_count.saturating_sub(1));
                if (start..=end).contains(&paragraph_global) {
                    paragraph_global.saturating_sub(start)
                } else {
                    0
                }
            })
            .unwrap_or(0);

        let (word_index, _) = self.content.paragraph_progress();
        Some(ResumeState {
            selected_book: self.content.selected_index().min(title_total.saturating_sub(1)),
            chapter_index,
            paragraph_in_chapter,
            word_index: word_index.max(1),
        })
    }

    pub fn export_wake_snapshot(&self) -> Option<WakeSnapshot> {
        let resume = self.export_resume_state()?;
        let ui_context = match self.ui {
            UiState::Library { cursor } => SleepUiContext::Library { cursor },
            UiState::Settings { cursor, editing } => SleepUiContext::Settings { cursor, editing },
            UiState::Reading { paused: true, .. } => SleepUiContext::ReadingPaused,
            UiState::Reading { paused: false, .. } => return None,
            UiState::NavigateChapter { chapter_cursor, .. } => {
                SleepUiContext::NavigateChapter { chapter_cursor }
            }
            UiState::NavigateChapterLoading { chapter_index, .. } => {
                SleepUiContext::NavigateChapter {
                    chapter_cursor: chapter_index,
                }
            }
            UiState::NavigateParagraph {
                chapter_index,
                paragraph_cursor,
                ..
            } => SleepUiContext::NavigateParagraph {
                chapter_index,
                paragraph_in_chapter: self.paragraph_in_chapter_from_cursor(
                    chapter_index,
                    paragraph_cursor,
                ),
            },
            UiState::Countdown { selected_book, .. } => SleepUiContext::Library {
                cursor: selected_book.min(self.settings_item_index()),
            },
            UiState::Status { .. } => SleepUiContext::Library {
                cursor: self.content.selected_index().min(self.settings_item_index()),
            },
        };

        Some(WakeSnapshot { ui_context, resume })
    }

    pub fn import_wake_snapshot(&mut self, snapshot: WakeSnapshot, now_ms: u64) -> bool {
        self.import_resume_for_context(snapshot.resume, snapshot.ui_context, now_ms)
    }

    pub fn import_resume_state(&mut self, resume: ResumeState, now_ms: u64) -> bool {
        self.import_resume_for_context(resume, SleepUiContext::ReadingPaused, now_ms)
    }

    fn import_resume_for_context(
        &mut self,
        mut resume: ResumeState,
        context: SleepUiContext,
        now_ms: u64,
    ) -> bool {
        let title_total = self.total_title_count();
        if title_total == 0 {
            return false;
        }

        resume.selected_book = resume.selected_book.min(title_total.saturating_sub(1));
        if self.content.select_text(resume.selected_book).is_err() {
            return false;
        }

        let chapter_total = self.content.chapter_count().max(1);
        resume.chapter_index = resume.chapter_index.min(chapter_total.saturating_sub(1));
        self.pending_wake_restore = None;

        let requires_chapter_data = matches!(
            context,
            SleepUiContext::ReadingPaused
                | SleepUiContext::NavigateChapter { .. }
                | SleepUiContext::NavigateParagraph { .. }
        );

        if requires_chapter_data {
            match self.content.seek_chapter(resume.chapter_index) {
                Ok(true) => {
                    if self.content.chapter_data_ready(resume.chapter_index) {
                        if !self.apply_resume_context(resume, context, now_ms) {
                            return false;
                        }
                    } else {
                        self.pending_wake_restore = Some(PendingWakeRestore { resume, context });
                        self.enter_chapter_loading(resume.selected_book, resume.chapter_index, now_ms);
                    }
                }
                Ok(false) => {
                    if !self.apply_resume_context(resume, context, now_ms) {
                        return false;
                    }
                }
                Err(_) => return false,
            }
        } else if !self.apply_resume_context(resume, context, now_ms) {
            return false;
        }

        self.last_input_activity_ms = now_ms;
        self.pending_redraw = true;
        true
    }

    fn apply_resume_context(
        &mut self,
        resume: ResumeState,
        context: SleepUiContext,
        now_ms: u64,
    ) -> bool {
        match context {
            SleepUiContext::ReadingPaused => self.apply_resume_location(resume, now_ms),
            SleepUiContext::Library { cursor } => {
                self.enter_library(cursor.min(self.settings_item_index()), now_ms);
                true
            }
            SleepUiContext::Settings { cursor, editing } => {
                let cursor = cursor.min(SettingsRow::COUNT.saturating_sub(1));
                self.enter_settings(cursor, editing, now_ms);
                true
            }
            SleepUiContext::NavigateChapter { chapter_cursor } => {
                if !self.apply_resume_cursor(resume) {
                    return false;
                }
                self.enter_chapter_navigation(resume.selected_book, chapter_cursor, now_ms);
                true
            }
            SleepUiContext::NavigateParagraph {
                chapter_index,
                paragraph_in_chapter,
            } => {
                if !self.apply_resume_cursor(resume) {
                    return false;
                }

                let chapter_total = self.content.chapter_count().max(1);
                let chapter_index = chapter_index.min(chapter_total.saturating_sub(1));
                let paragraph_cursor = self
                    .content
                    .chapter_at(chapter_index)
                    .map(|chapter| {
                        let chapter_start = chapter.start_paragraph;
                        let paragraph_rel =
                            paragraph_in_chapter.min(chapter.paragraph_count.saturating_sub(1));
                        chapter_start.saturating_add(paragraph_rel)
                    })
                    .unwrap_or_else(|| self.content.paragraph_index().saturating_sub(1));

                self.enter_paragraph_navigation(
                    resume.selected_book,
                    chapter_index,
                    paragraph_cursor,
                    now_ms,
                );
                true
            }
        }
    }

    fn apply_resume_location(&mut self, resume: ResumeState, now_ms: u64) -> bool {
        if !self.apply_resume_cursor(resume) {
            return false;
        }

        self.last_reading_press_ms = None;
        self.ui = UiState::Reading {
            selected_book: resume.selected_book,
            paused: true,
            next_word_ms: now_ms,
        };
        self.paused_since_ms = Some(now_ms);
        self.last_pause_anim_slot = None;
        self.start_transition(AnimationKind::Fade, now_ms, ANIM_SCREEN_MS);
        self.pending_redraw = true;
        true
    }

    fn apply_resume_cursor(&mut self, resume: ResumeState) -> bool {
        let (chapter_start, chapter_count) = self
            .content
            .chapter_at(resume.chapter_index)
            .map(|chapter| (chapter.start_paragraph, chapter.paragraph_count.max(1)))
            .unwrap_or((0, self.content.paragraph_total().max(1)));
        let paragraph_rel = resume
            .paragraph_in_chapter
            .min(chapter_count.saturating_sub(1));
        let target_paragraph = chapter_start.saturating_add(paragraph_rel);

        if self.content.seek_paragraph(target_paragraph).is_err() {
            return false;
        }

        self.reset_read_word_state();

        let target_word = resume.word_index.clamp(1, 512);
        let mut advanced_any = false;
        let mut advanced_count = 0u16;
        for _ in 0..target_word {
            match self.advance_word() {
                Ok(AdvanceWordResult::Advanced) => {
                    advanced_any = true;
                    advanced_count = advanced_count.saturating_add(1);
                }
                _ => break,
            }
        }
        if !advanced_any {
            let _ = self.advance_word();
        }

        let resume_unreadable = advanced_count == 0 && !self.content.is_waiting_for_refill();
        if resume_unreadable {
            info!(
                "resume: unreadable position selected_book={} chapter={} paragraph={} word={}",
                resume.selected_book.saturating_add(1),
                resume.chapter_index.saturating_add(1),
                resume.paragraph_in_chapter.saturating_add(1),
                resume.word_index.max(1)
            );

            if let Ok(true) = self.content.seek_chapter(resume.chapter_index) {
                self.reset_read_word_state();
                self.word_buffer.set("LOAD");
                return true;
            }

            info!(
                "resume: fallback to book start selected_book={} reason=chapter_requeue_failed",
                resume.selected_book.saturating_add(1)
            );
            match self.content.seek_chapter(0) {
                Ok(true) => {}
                Ok(false) | Err(_) => {
                    if self.content.seek_paragraph(0).is_err() {
                        return false;
                    }
                }
            }
            self.reset_read_word_state();
            let _ = self.advance_word();
        }

        self.last_reading_press_ms = None;
        true
    }

    fn paragraph_in_chapter_from_cursor(&self, chapter_index: u16, paragraph_cursor: u16) -> u16 {
        self.content
            .chapter_at(chapter_index)
            .map(|chapter| {
                let chapter_start = chapter.start_paragraph;
                let chapter_end =
                    chapter_start.saturating_add(chapter.paragraph_count.saturating_sub(1));
                paragraph_cursor
                    .clamp(chapter_start, chapter_end)
                    .saturating_sub(chapter_start)
            })
            .unwrap_or(0)
    }

    fn reset_read_word_state(&mut self) {
        self.word_buffer.clear();
        self.paragraph_word_index = 0;
        self.paragraph_word_total = 1;
        self.last_ends_clause = false;
        self.last_ends_sentence = false;
    }


}
