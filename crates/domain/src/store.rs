use crate::{
    content::{CollectionKind, ContentState},
    device::{BootState, DeviceState},
    formatter::format_article_document,
    input::InputState,
    network::NetworkState,
    power::PowerStatus,
    reader::ReaderSession,
    runtime::{BootstrapSnapshot, Command, Effect, Event, UiCommand},
    settings::{REFRESH_LOADING_DURATION_MS, RefreshState, SettingsState},
    sleep::{SleepModel, WakeReason},
    storage::StorageHealth,
    ui::{SettingsMode, SettingsRow, TopicRegion, UiRoute, UiState},
};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum DispatchError {
    #[default]
    UnsupportedCommand,
}

pub type DispatchResult = Result<Effect, DispatchError>;

#[derive(Debug, Default)]
pub struct Store {
    pub device: DeviceState,
    pub content: ContentState,
    pub input: InputState,
    pub network: NetworkState,
    pub power: PowerStatus,
    pub reader: ReaderSession,
    pub settings: SettingsState,
    pub sleep: SleepModel,
    pub storage: StorageHealth,
    pub ui: UiState,
}

impl Store {
    pub const fn new() -> Self {
        Self::from_bootstrap(BootstrapSnapshot::new(
            DeviceState::new(),
            0,
            None,
            StorageHealth::new(),
        ))
    }

    pub const fn from_bootstrap(snapshot: BootstrapSnapshot) -> Self {
        let settings = match snapshot.settings {
            Some(settings) => SettingsState::from_persisted(settings),
            None => SettingsState::new(crate::sleep::DEFAULT_INACTIVITY_TIMEOUT_MS),
        };
        let wake_reason = match snapshot.device.boot {
            BootState::DeepSleepWake => WakeReason::ExternalButton,
            BootState::ColdBoot => WakeReason::ColdBoot,
        };

        Self {
            device: snapshot.device,
            content: ContentState::mock(),
            input: InputState::new(),
            network: NetworkState::online(),
            power: PowerStatus::new(82),
            reader: ReaderSession::new(),
            settings,
            sleep: SleepModel {
                config: crate::sleep::SleepConfig::new(settings.inactivity_timeout_ms),
                state: crate::sleep::SleepState::Awake,
                last_activity_ms: snapshot.boot_at_ms,
                last_wake_reason: wake_reason,
            },
            storage: snapshot.storage,
            ui: UiState::new(),
        }
    }

    pub fn dispatch(&mut self, command: Command) -> DispatchResult {
        match command {
            Command::RequestDeepSleep => {
                self.sleep.request_sleep();
                Ok(Effect::EnterDeepSleep)
            }
            Command::Ui(command) => Ok(self.dispatch_ui(command)),
            Command::Noop | Command::Boot => Ok(Effect::Noop),
        }
    }

    pub fn handle_event(&mut self, event: Event, now_ms: u64) -> DispatchResult {
        match event {
            Event::InputGestureReceived(gesture) => {
                self.input.record_gesture(gesture);
                self.sleep.note_activity(now_ms);
            }
            Event::UiTick(tick_ms) => {
                if matches!(self.settings.refresh_state, RefreshState::Refreshing) {
                    let started = self.settings.refresh_started_at_ms.unwrap_or(tick_ms);
                    if tick_ms.saturating_sub(started) >= REFRESH_LOADING_DURATION_MS {
                        self.settings.complete_refresh();
                        self.ui.settings_mode = SettingsMode::Master;
                    }
                }
            }
            Event::ReaderTick(tick_ms) => {
                if matches!(self.ui.route, UiRoute::Reader) {
                    if self.reader.is_active_reading() {
                        self.sleep.note_activity(tick_ms);
                    }
                    self.reader
                        .advance_if_due(tick_ms, self.settings.reading_speed_wpm);
                }
            }
            Event::WokeFromDeepSleep => {
                self.device.boot = BootState::DeepSleepWake;
                self.sleep.mark_woke(WakeReason::ExternalButton, now_ms);
            }
            Event::BootCompleted => {}
            Event::Noop => {}
        }

        Ok(Effect::Noop)
    }

    fn dispatch_ui(&mut self, command: UiCommand) -> Effect {
        match self.ui.route {
            UiRoute::Dashboard => self.dispatch_dashboard(command),
            UiRoute::Collection(kind) => self.dispatch_collection(command, kind),
            UiRoute::Reader => self.dispatch_reader(command),
            UiRoute::Settings => self.dispatch_settings(command),
        }
    }

    fn dispatch_dashboard(&mut self, command: UiCommand) -> Effect {
        match command {
            UiCommand::FocusPrevious => self.ui.move_dashboard_previous(),
            UiCommand::FocusNext => self.ui.move_dashboard_next(),
            UiCommand::Confirm => {
                self.ui.route = UiRoute::Collection(self.ui.dashboard_focus.as_collection());
            }
            UiCommand::Back => {
                self.ui.route = UiRoute::Settings;
                self.ui.settings_mode = SettingsMode::Master;
                self.ui.settings_row = SettingsRow::ReadingSpeed;
            }
            UiCommand::Noop => {}
        }

        Effect::Noop
    }

    fn dispatch_collection(&mut self, command: UiCommand, kind: CollectionKind) -> Effect {
        match command {
            UiCommand::FocusPrevious => self.ui.move_collection_previous(kind),
            UiCommand::FocusNext => self.ui.move_collection_next(kind),
            UiCommand::Confirm => {
                let article = self
                    .content
                    .article_at(kind, self.ui.collection_index(kind));
                self.reader.open_article(
                    kind,
                    article.id,
                    format_article_document(&crate::content::ArticleDocument::new(
                        article.source,
                        article.script,
                    )),
                    article.has_chat,
                );
                self.ui.route = UiRoute::Reader;
            }
            UiCommand::Back => {
                self.ui.route = UiRoute::Dashboard;
                self.ui.dashboard_focus = match kind {
                    CollectionKind::Inbox => crate::ui::DashboardFocus::Inbox,
                    CollectionKind::Saved => crate::ui::DashboardFocus::Saved,
                    CollectionKind::Recommendations => crate::ui::DashboardFocus::Recommendations,
                };
            }
            UiCommand::Noop => {}
        }

        Effect::Noop
    }

    fn dispatch_reader(&mut self, command: UiCommand) -> Effect {
        match self.reader.mode {
            crate::reader::ReaderMode::Normal | crate::reader::ReaderMode::Chat => match command {
                UiCommand::FocusPrevious => self.reader.show_normal(),
                UiCommand::FocusNext => self.reader.show_chat(),
                UiCommand::Confirm => self.reader.pause(),
                UiCommand::Back => {
                    self.ui.route = UiRoute::Collection(self.reader.active_collection);
                    self.reader.mode = crate::reader::ReaderMode::Normal;
                    self.reader.resume_mode = crate::reader::ReaderMode::Normal;
                    self.reader.next_due_at_ms = None;
                }
                UiCommand::Noop => {}
            },
            crate::reader::ReaderMode::Paused => match command {
                UiCommand::FocusPrevious => {
                    self.settings.adjust_reading_speed(true);
                    return self.persist_settings_effect();
                }
                UiCommand::FocusNext => {
                    self.settings.adjust_reading_speed(false);
                    return self.persist_settings_effect();
                }
                UiCommand::Confirm => {
                    self.reader.open_paragraph_navigation();
                }
                UiCommand::Back => {
                    self.reader.resume();
                }
                UiCommand::Noop => {}
            },
            crate::reader::ReaderMode::ParagraphNavigation => match command {
                UiCommand::FocusPrevious => self.reader.move_paragraph(true),
                UiCommand::FocusNext => self.reader.move_paragraph(false),
                UiCommand::Confirm => self.reader.commit_paragraph_navigation(),
                UiCommand::Back => self.reader.close_paragraph_navigation(),
                UiCommand::Noop => {}
            },
        }

        Effect::Noop
    }

    fn dispatch_settings(&mut self, command: UiCommand) -> Effect {
        match self.ui.settings_mode {
            SettingsMode::Master => self.dispatch_settings_master(command),
            SettingsMode::SpeedEdit => self.dispatch_speed_edit(command),
            SettingsMode::AppearanceEdit => self.dispatch_appearance_edit(command),
            SettingsMode::RefreshLoading => {
                if matches!(command, UiCommand::Back) {
                    self.settings.complete_refresh();
                    self.ui.settings_mode = SettingsMode::Master;
                }
                Effect::Noop
            }
            SettingsMode::TopicPreferences => self.dispatch_topic_preferences(command),
        }
    }

    fn dispatch_settings_master(&mut self, command: UiCommand) -> Effect {
        match command {
            UiCommand::FocusPrevious => self.ui.move_settings_previous(),
            UiCommand::FocusNext => self.ui.move_settings_next(),
            UiCommand::Confirm => match self.ui.settings_row {
                SettingsRow::ReadingSpeed => self.ui.settings_mode = SettingsMode::SpeedEdit,
                SettingsRow::Appearance => self.ui.settings_mode = SettingsMode::AppearanceEdit,
                SettingsRow::RefreshData => {
                    self.ui.settings_mode = SettingsMode::RefreshLoading;
                    self.settings.start_refresh(self.sleep.last_activity_ms);
                }
                SettingsRow::TopicPreferences => {
                    self.ui.settings_mode = SettingsMode::TopicPreferences;
                    self.ui.topic_focus.region = TopicRegion::Categories;
                }
                SettingsRow::NetworkConnection | SettingsRow::ConnectAccount => {}
            },
            UiCommand::Back => self.ui.route = UiRoute::Dashboard,
            UiCommand::Noop => {}
        }

        Effect::Noop
    }

    fn dispatch_speed_edit(&mut self, command: UiCommand) -> Effect {
        match command {
            UiCommand::FocusPrevious => {
                self.settings.adjust_reading_speed(true);
                return self.persist_settings_effect();
            }
            UiCommand::FocusNext => {
                self.settings.adjust_reading_speed(false);
                return self.persist_settings_effect();
            }
            UiCommand::Confirm | UiCommand::Back => {
                self.ui.settings_mode = SettingsMode::Master;
                self.ui.settings_row = SettingsRow::ReadingSpeed;
            }
            UiCommand::Noop => {}
        }

        Effect::Noop
    }

    fn dispatch_appearance_edit(&mut self, command: UiCommand) -> Effect {
        match command {
            UiCommand::FocusPrevious | UiCommand::FocusNext => {
                self.settings.toggle_appearance();
                return self.persist_settings_effect();
            }
            UiCommand::Confirm | UiCommand::Back => {
                self.ui.settings_mode = SettingsMode::Master;
                self.ui.settings_row = SettingsRow::Appearance;
            }
            UiCommand::Noop => {}
        }

        Effect::Noop
    }

    fn dispatch_topic_preferences(&mut self, command: UiCommand) -> Effect {
        match command {
            UiCommand::FocusPrevious => self
                .ui
                .move_topic_previous(crate::settings::TOPIC_CHIP_COUNT),
            UiCommand::FocusNext => self.ui.move_topic_next(crate::settings::TOPIC_CHIP_COUNT),
            UiCommand::Confirm => match self.ui.topic_focus.region {
                TopicRegion::Categories => {
                    self.ui.topic_focus.region = TopicRegion::Chips;
                    self.ui.topic_focus.chip_index = 0;
                }
                TopicRegion::Chips => {
                    self.settings.topics.toggle_chip(
                        self.ui.topic_focus.category_index,
                        self.ui.topic_focus.chip_index,
                    );
                    return self.persist_settings_effect();
                }
            },
            UiCommand::Back => {
                if matches!(self.ui.topic_focus.region, TopicRegion::Chips) {
                    self.ui.topic_focus.region = TopicRegion::Categories;
                } else {
                    self.ui.settings_mode = SettingsMode::Master;
                    self.ui.settings_row = SettingsRow::TopicPreferences;
                }
            }
            UiCommand::Noop => {}
        }

        Effect::Noop
    }

    fn persist_settings_effect(&self) -> Effect {
        Effect::PersistSettings(self.settings.to_persisted())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        device::{BootState, DeviceState},
        settings::{AppearanceMode, PersistedSettings},
        storage::{StorageHealth, StorageRecoveryStatus},
    };

    #[test]
    fn deep_sleep_bootstrap_hydrates_sleep_and_storage() {
        let snapshot = BootstrapSnapshot::new(
            DeviceState::with_boot(BootState::DeepSleepWake),
            42,
            Some(PersistedSettings::with_preferences(
                45_000,
                320,
                AppearanceMode::Dark,
                crate::settings::TopicPreferences::new(),
            )),
            StorageHealth::available(100, 200, StorageRecoveryStatus::Recovered),
        );

        let store = Store::from_bootstrap(snapshot);

        assert_eq!(store.device.boot, BootState::DeepSleepWake);
        assert_eq!(store.settings.inactivity_timeout_ms, 45_000);
        assert_eq!(store.settings.reading_speed_wpm, 320);
        assert_eq!(store.settings.appearance, AppearanceMode::Dark);
        assert_eq!(store.sleep.config.inactivity_timeout_ms, 45_000);
        assert_eq!(store.sleep.last_activity_ms, 42);
        assert_eq!(store.sleep.last_wake_reason, WakeReason::ExternalButton);
        assert_eq!(store.storage.state_free_bytes, 100);
        assert_eq!(store.storage.outbox_free_bytes, 200);
    }

    #[test]
    fn missing_persisted_settings_fall_back_to_default_timeout() {
        let snapshot = BootstrapSnapshot::new(
            DeviceState::with_boot(BootState::ColdBoot),
            7,
            None,
            StorageHealth::new(),
        );

        let store = Store::from_bootstrap(snapshot);

        assert_eq!(
            store.settings.inactivity_timeout_ms,
            crate::sleep::DEFAULT_INACTIVITY_TIMEOUT_MS
        );
        assert_eq!(
            store.sleep.config.inactivity_timeout_ms,
            crate::sleep::DEFAULT_INACTIVITY_TIMEOUT_MS
        );
        assert_eq!(store.sleep.last_wake_reason, WakeReason::ColdBoot);
        assert_eq!(store.ui.route, UiRoute::Dashboard);
    }

    #[test]
    fn dashboard_confirm_opens_selected_collection() {
        let mut store = Store::new();

        store.dispatch(Command::Ui(UiCommand::Confirm)).unwrap();

        assert_eq!(store.ui.route, UiRoute::Collection(CollectionKind::Saved));
    }

    #[test]
    fn paused_reader_confirm_opens_paragraph_navigation() {
        let mut store = Store::new();
        store.ui.route = UiRoute::Reader;
        store.reader.pause();

        store.dispatch(Command::Ui(UiCommand::Confirm)).unwrap();

        assert!(matches!(
            store.reader.mode,
            crate::reader::ReaderMode::ParagraphNavigation
        ));
    }

    #[test]
    fn refresh_loading_completes_on_tick() {
        let mut store = Store::new();
        store.ui.route = UiRoute::Settings;
        store.ui.settings_mode = SettingsMode::RefreshLoading;
        store.settings.start_refresh(10);

        store
            .handle_event(Event::UiTick(REFRESH_LOADING_DURATION_MS + 10), 0)
            .unwrap();

        assert!(matches!(store.settings.refresh_state, RefreshState::Idle));
        assert!(matches!(store.ui.settings_mode, SettingsMode::Master));
    }

    #[test]
    fn reader_tick_advances_live_rsvp_session() {
        let mut store = Store::new();

        store.dispatch(Command::Ui(UiCommand::Confirm)).unwrap();
        store.dispatch(Command::Ui(UiCommand::Confirm)).unwrap();
        let before = store.reader.progress.unit_index;

        store.handle_event(Event::ReaderTick(0), 0).unwrap();
        store.handle_event(Event::ReaderTick(1_000), 0).unwrap();

        assert!(store.reader.progress.unit_index > before);
        assert_eq!(store.ui.route, UiRoute::Reader);
    }

    #[test]
    fn active_reader_tick_keeps_sleep_awake() {
        let mut store = Store::new();

        store.dispatch(Command::Ui(UiCommand::Confirm)).unwrap();
        store.dispatch(Command::Ui(UiCommand::Confirm)).unwrap();
        store.sleep.last_activity_ms = 10;

        store.handle_event(Event::ReaderTick(250), 0).unwrap();

        assert_eq!(store.sleep.last_activity_ms, 250);
    }

    #[test]
    fn paused_reader_tick_does_not_refresh_sleep_timer() {
        let mut store = Store::new();

        store.dispatch(Command::Ui(UiCommand::Confirm)).unwrap();
        store.dispatch(Command::Ui(UiCommand::Confirm)).unwrap();
        store.reader.pause();
        store.sleep.last_activity_ms = 10;

        store.handle_event(Event::ReaderTick(250), 0).unwrap();

        assert_eq!(store.sleep.last_activity_ms, 10);
    }

    #[test]
    fn appearance_edit_toggles_theme_setting() {
        let mut store = Store::new();
        store.ui.route = UiRoute::Settings;
        store.ui.settings_mode = SettingsMode::AppearanceEdit;
        store.settings.appearance = AppearanceMode::Light;

        let effect = store.dispatch(Command::Ui(UiCommand::FocusNext)).unwrap();

        assert_eq!(store.settings.appearance, AppearanceMode::Dark);
        assert_eq!(
            effect,
            Effect::PersistSettings(store.settings.to_persisted())
        );
    }

    #[test]
    fn paused_reader_speed_adjust_persists_settings() {
        let mut store = Store::new();
        store.ui.route = UiRoute::Reader;
        store.reader.pause();

        let effect = store
            .dispatch(Command::Ui(UiCommand::FocusPrevious))
            .unwrap();

        assert_eq!(
            effect,
            Effect::PersistSettings(store.settings.to_persisted())
        );
    }
}
