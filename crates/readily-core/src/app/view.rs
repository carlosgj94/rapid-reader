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
        }
    }

    pub fn tick(&mut self, now_ms: u64) -> TickResult {
        self.process_inputs(now_ms);

        let rendered = match self.ui {
            UiState::Countdown { .. } => self.tick_countdown(now_ms),
            UiState::Reading { .. } => self.tick_reading(now_ms),
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


}
