use alloc::boxed::Box;

use crate::{
    content::{
        CollectionKind, CollectionManifestState, ContentState, PackageState, PrepareContentRequest,
        ReadingProgressEntry, ReadingProgressState, RecommendationState,
        RecommendationTopicRequest,
    },
    device::{BootState, DeviceState},
    input::InputState,
    network::{NetworkState, NetworkStatus},
    power::PowerStatus,
    reader::ReaderSession,
    runtime::{
        BootstrapSnapshot, CollectionConfirmIgnoredReason, Command, Effect, Event, UiCommand,
    },
    settings::{REFRESH_LOADING_DURATION_MS, RefreshState, SettingsState},
    sleep::{SleepModel, WakeReason},
    storage::StorageHealth,
    sync::{SyncState, SyncStatus},
    ui::{RecommendationsRegion, SettingsMode, SettingsRow, TopicRegion, UiRoute, UiState},
};

static EMPTY_CONTENT_STATE: ContentState = ContentState::empty();
const RECOMMENDATION_SUBTOPIC_FOCUS_FLASH_TICKS: u8 = 8;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum DispatchError {
    #[default]
    UnsupportedCommand,
}

pub type DispatchResult = Result<Effect, DispatchError>;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct PendingPrepare {
    request: PrepareContentRequest,
    previous_state: PackageState,
    auto_open_reader: bool,
    dispatched: bool,
}

#[derive(Debug)]
pub struct Store {
    pub device: DeviceState,
    content: Option<Box<ContentState>>,
    pub reading_progress: ReadingProgressState,
    pub recommendations: RecommendationState,
    pending_prepare: Option<PendingPrepare>,
    pending_reading_progress_write: Option<ReadingProgressEntry>,
    pub input: InputState,
    pub network: NetworkState,
    pub power: PowerStatus,
    pub reader: ReaderSession,
    pub settings: SettingsState,
    pub sleep: SleepModel,
    pub storage: StorageHealth,
    pub backend_sync: SyncState,
    pub ui: UiState,
}

impl Store {
    pub fn new() -> Self {
        Self {
            device: DeviceState::new(),
            content: None,
            reading_progress: ReadingProgressState::empty(),
            recommendations: RecommendationState::new(),
            pending_prepare: None,
            pending_reading_progress_write: None,
            input: InputState::new(),
            network: NetworkState::disabled(),
            power: PowerStatus::new(82),
            reader: ReaderSession::new(),
            settings: SettingsState::new(crate::sleep::DEFAULT_INACTIVITY_TIMEOUT_MS),
            sleep: SleepModel {
                config: crate::sleep::SleepConfig::new(crate::sleep::DEFAULT_INACTIVITY_TIMEOUT_MS),
                state: crate::sleep::SleepState::Awake,
                last_activity_ms: 0,
                last_wake_reason: WakeReason::ColdBoot,
            },
            storage: StorageHealth::new(),
            backend_sync: SyncState::new(),
            ui: UiState::new(),
        }
    }

    pub fn from_bootstrap(snapshot: BootstrapSnapshot) -> Self {
        let mut store = Self::new();
        store.hydrate_from_bootstrap(snapshot);
        store
    }

    pub fn hydrate_from_bootstrap(&mut self, snapshot: BootstrapSnapshot) {
        let settings = match snapshot.settings {
            Some(settings) => SettingsState::from_persisted(settings),
            None => SettingsState::new(crate::sleep::DEFAULT_INACTIVITY_TIMEOUT_MS),
        };
        let wake_reason = match snapshot.device.boot {
            BootState::DeepSleepWake => WakeReason::ExternalButton,
            BootState::ColdBoot => WakeReason::ColdBoot,
        };
        self.device = snapshot.device;
        self.content = snapshot.content;
        self.reading_progress = snapshot
            .reading_progress
            .map(|progress| *progress)
            .unwrap_or_else(ReadingProgressState::empty);
        self.recommendations = RecommendationState::new();
        if let Some(subtopics) = snapshot.recommendation_subtopics {
            self.recommendations.set_subtopics(*subtopics);
        }
        self.pending_prepare = None;
        self.pending_reading_progress_write = None;
        self.input = InputState::new();
        self.network = snapshot.network;
        self.power = PowerStatus::new(82);
        self.reader = ReaderSession::new();
        self.settings = settings;
        self.sleep = SleepModel {
            config: crate::sleep::SleepConfig::new(self.settings.inactivity_timeout_ms),
            state: crate::sleep::SleepState::Awake,
            last_activity_ms: snapshot.boot_at_ms,
            last_wake_reason: wake_reason,
        };
        self.storage = snapshot.storage;
        self.backend_sync = SyncState::new();
        self.ui = UiState::new();
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
            Event::NetworkStatusChanged(status) => {
                self.network.status = status;
                if let Some(request) = self.dispatchable_pending_prepare_request() {
                    return Ok(Effect::PrepareContent(request));
                }
            }
            Event::BackendSyncStatusChanged(status) => {
                self.backend_sync.set_status(status);
                if matches!(status, SyncStatus::AuthFailed | SyncStatus::Disabled) {
                    self.restore_pending_prepare();
                } else if let Some(request) = self.dispatchable_pending_prepare_request() {
                    return Ok(Effect::PrepareContent(request));
                }
            }
            Event::CollectionContentUpdated(kind, mut collection) => {
                if let Some(pending) = self.pending_prepare
                    && pending.request.collection == kind
                    && collection_package_state_for_remote_item(
                        &collection,
                        &pending.request.remote_item_id,
                    ) != Some(PackageState::Cached)
                {
                    let _ = collection.update_package_state(
                        &pending.request.remote_item_id,
                        PackageState::Fetching,
                    );
                }
                if self.content.is_some() || !collection.is_empty() {
                    self.content_mut().update_boxed_collection(kind, collection);
                }
                if let Some(pending) = self.pending_prepare
                    && pending.request.collection == kind
                    && let Some(index) =
                        self.collection_index_for_remote_item(kind, &pending.request.remote_item_id)
                {
                    self.set_collection_index(kind, index);
                } else {
                    self.set_collection_index(kind, 0);
                }
            }
            Event::RecommendationSubtopicsUpdated(subtopics) => {
                let previous_active_topic = self.recommendations.active_topic_slug;
                self.recommendations.finish_subtopics_loading();
                self.recommendations.set_subtopics(*subtopics);
                self.ui.recommendations_subtopic_index =
                    self.recommendations.active_topic_index().unwrap_or(0);

                if self.recommendations.subtopics.is_empty() {
                    self.recommendations.topic_loading = false;
                    self.focus_recommendation_subtopics(false);
                    self.ui.recommendations_index = 0;
                    self.content_mut().update_collection(
                        CollectionKind::Recommendations,
                        CollectionManifestState::empty(),
                    );
                    return Ok(Effect::Noop);
                }

                let active_topic = self.recommendations.active_topic_slug;
                let needs_topic_fetch = previous_active_topic != active_topic
                    || self
                        .content()
                        .collection_state(CollectionKind::Recommendations)
                        .is_empty();
                let should_prefetch_active_topic = matches!(
                    self.ui.route,
                    UiRoute::Collection(CollectionKind::Recommendations)
                ) && !self.recommendations.topic_loading;

                if needs_topic_fetch && should_prefetch_active_topic {
                    self.recommendations.set_active_topic(active_topic, true);
                    self.ui.recommendations_index = 0;
                    self.content_mut().update_collection(
                        CollectionKind::Recommendations,
                        CollectionManifestState::empty(),
                    );
                    return Ok(Effect::LoadRecommendationTopic(
                        RecommendationTopicRequest::new(active_topic),
                    ));
                }
            }
            Event::RecommendationTopicContentUpdated {
                topic_slug,
                collection,
            } => {
                if topic_slug != self.recommendations.active_topic_slug {
                    return Ok(Effect::Noop);
                }

                let collection_len = collection.len();
                self.recommendations.topic_loading = false;
                if self.content.is_some() || collection_len > 0 {
                    self.content_mut()
                        .update_boxed_collection(CollectionKind::Recommendations, collection);
                }
                if collection_len == 0 || self.ui.recommendations_index >= collection_len {
                    self.ui.recommendations_index = 0;
                }
            }
            Event::ReaderContentOpened {
                collection,
                content_id,
                title,
                total_units,
                paragraphs,
                window,
            } => {
                let should_open = self.pending_prepare.as_ref().is_some_and(|pending| {
                    pending.request.collection == collection
                        && pending.request.content_id == content_id
                        && pending.auto_open_reader
                });

                if should_open {
                    let remote_revision = self
                        .pending_prepare
                        .as_ref()
                        .filter(|pending| {
                            pending.request.collection == collection
                                && pending.request.content_id == content_id
                        })
                        .map(|pending| pending.request.remote_revision)
                        .unwrap_or(0);
                    self.open_cached_content(
                        collection,
                        content_id,
                        remote_revision,
                        title,
                        total_units,
                        paragraphs,
                        window,
                    );
                } else if self.pending_prepare.as_ref().is_some_and(|pending| {
                    pending.request.collection == collection
                        && pending.request.content_id == content_id
                }) {
                    self.pending_prepare = None;
                }
            }
            Event::ContentPackageStateChanged {
                collection,
                remote_item_id,
                package_state,
            } => {
                let _ = self.content_mut().update_package_state(
                    collection,
                    &remote_item_id,
                    package_state,
                );
                if package_state != PackageState::Fetching
                    && self.pending_prepare.is_some_and(|pending| {
                        pending.request.collection == collection
                            && pending.request.remote_item_id == remote_item_id
                    })
                {
                    if matches!(self.reader.mode, crate::reader::ReaderMode::LoadingContent)
                        && self
                            .pending_prepare
                            .is_some_and(|pending| pending.auto_open_reader)
                    {
                        self.ui.route = UiRoute::Collection(collection);
                        self.reader.unload_document();
                    }
                    self.pending_prepare = None;
                }
            }
            Event::ContentPrepareProgress {
                content_id,
                progress,
            } => {
                if self.pending_prepare.as_ref().is_some_and(|pending| {
                    pending.request.content_id == content_id && pending.auto_open_reader
                }) {
                    self.reader.update_prepare_progress(progress);
                }
            }
            Event::UiTick(tick_ms) => {
                if matches!(self.ui.route, UiRoute::Dashboard | UiRoute::Collection(_)) {
                    self.backend_sync.advance_spinner();
                }
                if self.ui.recommendations_focus_flash_ticks > 0 {
                    self.ui.recommendations_focus_flash_ticks -= 1;
                }
                if matches!(self.ui.route, UiRoute::Reader)
                    && matches!(self.reader.mode, crate::reader::ReaderMode::LoadingContent)
                {
                    self.reader.advance_prepare_animation();
                }
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
                    let previous_paragraph = self.reader.progress.paragraph_index;
                    let outcome = self
                        .reader
                        .advance_if_due(tick_ms, self.settings.reading_speed_wpm);
                    if self.reader.progress.paragraph_index != previous_paragraph {
                        self.track_reader_progress();
                    }
                    if let Some(request) = outcome.load_request {
                        return Ok(Effect::LoadReaderWindow(request));
                    }
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

    #[allow(clippy::too_many_arguments)]
    pub fn open_cached_content(
        &mut self,
        collection: CollectionKind,
        content_id: crate::text::InlineText<{ crate::content::CONTENT_ID_MAX_BYTES }>,
        remote_revision: u64,
        title: crate::text::InlineText<{ crate::content::CONTENT_TITLE_MAX_BYTES }>,
        total_units: u32,
        paragraphs: Box<[crate::reader::ReaderParagraphInfo]>,
        window: Box<crate::reader::ReaderWindow>,
    ) {
        if self.pending_prepare.is_some_and(|pending| {
            pending.request.collection == collection && pending.request.content_id == content_id
        }) {
            self.pending_prepare = None;
        }
        self.reader.open_cached_reader_content(
            collection,
            crate::content::ArticleId(0),
            content_id,
            remote_revision,
            title,
            total_units,
            paragraphs,
            window,
            false,
            self.settings.reading_speed_wpm,
        );
        if matches!(collection, CollectionKind::Recommendations) {
            self.show_recommendation_articles();
        }
        self.ui.route = UiRoute::Reader;
        self.track_reader_progress();
    }

    pub fn load_reader_window(&mut self, window: Box<crate::reader::ReaderWindow>) {
        self.reader.apply_loaded_window(window);
        self.track_reader_progress();
    }

    pub fn take_pending_reading_progress_write(&mut self) -> Option<ReadingProgressEntry> {
        self.pending_reading_progress_write.take()
    }

    pub fn content(&self) -> &ContentState {
        self.content.as_deref().unwrap_or(&EMPTY_CONTENT_STATE)
    }

    pub fn content_mut(&mut self) -> &mut ContentState {
        self.content
            .get_or_insert_with(ContentState::boxed_empty)
            .as_mut()
    }

    fn focus_recommendation_subtopics(&mut self, flash: bool) {
        self.ui.recommendations_region = RecommendationsRegion::Subtopics;
        self.ui.recommendations_focus_flash_ticks = if flash {
            RECOMMENDATION_SUBTOPIC_FOCUS_FLASH_TICKS
        } else {
            0
        };
    }

    fn show_recommendation_articles(&mut self) {
        self.ui.recommendations_region = RecommendationsRegion::Articles;
        self.ui.recommendations_focus_flash_ticks = 0;
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
                let collection = self.ui.dashboard_focus.as_collection();
                self.ui.route = UiRoute::Collection(collection);
                if matches!(collection, CollectionKind::Recommendations) {
                    return self.enter_recommendations();
                }
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
        if matches!(kind, CollectionKind::Recommendations) {
            return self.dispatch_recommendations(command);
        }

        let collection_len = self.content().collection_len(kind);

        match command {
            UiCommand::FocusPrevious => self.ui.move_collection_previous(kind, collection_len),
            UiCommand::FocusNext => self.ui.move_collection_next(kind, collection_len),
            UiCommand::Confirm => return self.confirm_collection_item(kind),
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

    fn enter_recommendations(&mut self) -> Effect {
        if self.recommendations.subtopics.is_empty() {
            self.focus_recommendation_subtopics(false);
            self.ui.recommendations_subtopic_index = 0;
            self.ui.recommendations_index = 0;

            if !self.recommendations.subtopics_loading {
                self.recommendations.begin_subtopics_loading();
                return Effect::LoadRecommendationSubtopics;
            }

            return Effect::Noop;
        }

        self.ui.recommendations_subtopic_index =
            self.recommendations.active_topic_index().unwrap_or(0);
        let has_active_topic = !self.recommendations.active_topic_slug.is_empty();
        let has_articles = !self
            .content()
            .collection_state(CollectionKind::Recommendations)
            .is_empty();

        if has_active_topic && (has_articles || self.recommendations.topic_loading) {
            self.show_recommendation_articles();
        } else {
            self.focus_recommendation_subtopics(false);
        }

        if has_active_topic && !has_articles && !self.recommendations.topic_loading {
            self.recommendations
                .set_active_topic(self.recommendations.active_topic_slug, true);
            self.ui.recommendations_index = 0;
            self.show_recommendation_articles();
            return Effect::LoadRecommendationTopic(RecommendationTopicRequest::new(
                self.recommendations.active_topic_slug,
            ));
        }

        Effect::Noop
    }

    fn dispatch_recommendations(&mut self, command: UiCommand) -> Effect {
        match self.ui.recommendations_region {
            RecommendationsRegion::Subtopics => self.dispatch_recommendation_subtopics(command),
            RecommendationsRegion::Articles => self.dispatch_recommendation_articles(command),
        }
    }

    fn dispatch_recommendation_subtopics(&mut self, command: UiCommand) -> Effect {
        let subtopic_len = self.recommendations.subtopics.len();

        match command {
            UiCommand::FocusPrevious => {
                if subtopic_len == 0 {
                    self.ui.recommendations_subtopic_index = 0;
                } else {
                    self.ui.recommendations_subtopic_index = self
                        .ui
                        .recommendations_subtopic_index
                        .min(subtopic_len.saturating_sub(1))
                        .saturating_sub(1);
                }
                self.ui.recommendations_focus_flash_ticks = 0;
            }
            UiCommand::FocusNext => {
                if subtopic_len == 0 {
                    self.ui.recommendations_subtopic_index = 0;
                } else {
                    self.ui.recommendations_subtopic_index = self
                        .ui
                        .recommendations_subtopic_index
                        .min(subtopic_len.saturating_sub(1))
                        .saturating_add(1)
                        .min(subtopic_len.saturating_sub(1));
                }
                self.ui.recommendations_focus_flash_ticks = 0;
            }
            UiCommand::Confirm => {
                if subtopic_len == 0 {
                    if !self.recommendations.subtopics_loading {
                        self.recommendations.begin_subtopics_loading();
                        return Effect::LoadRecommendationSubtopics;
                    }
                    return Effect::Noop;
                }

                let Some(subtopic) = self
                    .recommendations
                    .subtopics
                    .item_at(self.ui.recommendations_subtopic_index)
                else {
                    return Effect::Noop;
                };

                if self.recommendations.active_topic_slug != subtopic.slug {
                    self.recommendations.set_active_topic(subtopic.slug, true);
                    self.ui.recommendations_index = 0;
                    self.show_recommendation_articles();
                    self.content_mut().update_collection(
                        CollectionKind::Recommendations,
                        CollectionManifestState::empty(),
                    );
                    return Effect::LoadRecommendationTopic(RecommendationTopicRequest::new(
                        subtopic.slug,
                    ));
                }

                self.show_recommendation_articles();
                if self
                    .content()
                    .collection_state(CollectionKind::Recommendations)
                    .is_empty()
                    && !self.recommendations.topic_loading
                {
                    self.recommendations.set_active_topic(subtopic.slug, true);
                    return Effect::LoadRecommendationTopic(RecommendationTopicRequest::new(
                        subtopic.slug,
                    ));
                }
            }
            UiCommand::Back => {
                self.ui.route = UiRoute::Dashboard;
                self.ui.dashboard_focus = crate::ui::DashboardFocus::Recommendations;
                self.ui.recommendations_focus_flash_ticks = 0;
            }
            UiCommand::Noop => {}
        }

        Effect::Noop
    }

    fn dispatch_recommendation_articles(&mut self, command: UiCommand) -> Effect {
        let collection_len = self
            .content()
            .collection_len(CollectionKind::Recommendations);

        match command {
            UiCommand::FocusPrevious => {
                if collection_len == 0 || self.ui.recommendations_index == 0 {
                    self.focus_recommendation_subtopics(true);
                } else {
                    self.ui.recommendations_index -= 1;
                }
            }
            UiCommand::FocusNext => {
                self.ui
                    .move_collection_next(CollectionKind::Recommendations, collection_len);
            }
            UiCommand::Confirm => {
                return self.confirm_collection_item(CollectionKind::Recommendations);
            }
            UiCommand::Back => self.focus_recommendation_subtopics(true),
            UiCommand::Noop => {}
        }

        Effect::Noop
    }

    fn confirm_collection_item(&mut self, kind: CollectionKind) -> Effect {
        let Some(item) = self
            .content()
            .manifest_item_at(kind, self.ui.collection_index(kind))
        else {
            return self
                .collection_confirm_ignored(kind, CollectionConfirmIgnoredReason::EmptyCollection);
        };

        if matches!(item.package_state, PackageState::Cached) {
            if !self.storage.sd_card_ready {
                return self.collection_confirm_ignored(
                    kind,
                    CollectionConfirmIgnoredReason::StorageUnavailable,
                );
            }
            return Effect::OpenCachedContent(PrepareContentRequest::from_manifest(kind, item));
        }

        if matches!(item.package_state, PackageState::Fetching) {
            return self
                .collection_confirm_ignored(kind, CollectionConfirmIgnoredReason::AlreadyFetching);
        }

        if self.pending_prepare.is_some() {
            return self
                .collection_confirm_ignored(kind, CollectionConfirmIgnoredReason::AlreadyFetching);
        }

        if !self.storage.sd_card_ready {
            return self.collection_confirm_ignored(
                kind,
                CollectionConfirmIgnoredReason::StorageUnavailable,
            );
        }

        if !item.can_prepare() {
            return self.collection_confirm_ignored(
                kind,
                ignored_reason_for_manifest_item(item.package_state),
            );
        }

        let request = PrepareContentRequest::from_manifest(kind, item);
        let _ = self.content_mut().update_package_state(
            kind,
            &request.remote_item_id,
            PackageState::Fetching,
        );
        if matches!(
            self.backend_sync.status,
            SyncStatus::AuthFailed | SyncStatus::Disabled
        ) {
            let _ = self.content_mut().update_package_state(
                kind,
                &request.remote_item_id,
                item.package_state,
            );
            return self.collection_confirm_ignored(
                kind,
                CollectionConfirmIgnoredReason::BackendUnavailable,
            );
        }
        let dispatch_now = self.can_dispatch_prepare_now();
        self.pending_prepare = Some(PendingPrepare {
            request,
            previous_state: item.package_state,
            auto_open_reader: true,
            dispatched: dispatch_now,
        });
        self.reader
            .begin_content_loading(kind, request.content_id, item.title);
        self.ui.route = UiRoute::Reader;
        if dispatch_now {
            return Effect::PrepareContent(request);
        }
        Effect::Noop
    }

    fn collection_confirm_ignored(
        &self,
        collection: CollectionKind,
        reason: CollectionConfirmIgnoredReason,
    ) -> Effect {
        Effect::CollectionConfirmIgnored { collection, reason }
    }

    fn can_dispatch_prepare_now(&self) -> bool {
        matches!(self.backend_sync.status, SyncStatus::Ready)
            && self.network.status == NetworkStatus::Online
    }

    fn dispatchable_pending_prepare_request(&mut self) -> Option<PrepareContentRequest> {
        if !self.can_dispatch_prepare_now() {
            return None;
        }

        let pending = self.pending_prepare.as_mut()?;
        if pending.dispatched {
            return None;
        }

        pending.dispatched = true;
        Some(pending.request)
    }

    fn restore_pending_prepare(&mut self) {
        if let Some(pending) = self.pending_prepare.take() {
            let _ = self.content_mut().update_package_state(
                pending.request.collection,
                &pending.request.remote_item_id,
                pending.previous_state,
            );
            if pending.auto_open_reader
                && matches!(self.reader.mode, crate::reader::ReaderMode::LoadingContent)
                && self.reader.active_content_id == pending.request.content_id
            {
                self.ui.route = UiRoute::Collection(pending.request.collection);
                self.reader.unload_document();
            }
        }
    }

    fn collection_index_for_remote_item(
        &self,
        kind: CollectionKind,
        remote_item_id: &crate::text::InlineText<{ crate::content::REMOTE_ITEM_ID_MAX_BYTES }>,
    ) -> Option<usize> {
        let collection = self.content().collection_state(kind);
        let mut index = 0usize;
        while index < collection.len() {
            if collection.items[index].remote_item_id == *remote_item_id {
                return Some(index);
            }
            index += 1;
        }
        None
    }

    fn set_collection_index(&mut self, kind: CollectionKind, index: usize) {
        match kind {
            CollectionKind::Saved => self.ui.saved_index = index,
            CollectionKind::Inbox => self.ui.inbox_index = index,
            CollectionKind::Recommendations => self.ui.recommendations_index = index,
        }
    }

    fn track_reader_progress(&mut self) {
        let Some(checkpoint) = self.reader.reading_progress_checkpoint() else {
            return;
        };
        let Some(entry) = self.reading_progress.record_progress(checkpoint) else {
            return;
        };
        self.queue_reading_progress_write(entry);
    }

    fn queue_reading_progress_write(&mut self, entry: ReadingProgressEntry) {
        match self.pending_reading_progress_write {
            Some(queued)
                if queued.content_id == entry.content_id
                    && queued.remote_revision == entry.remote_revision =>
            {
                self.pending_reading_progress_write = Some(ReadingProgressEntry {
                    content_id: entry.content_id,
                    remote_revision: entry.remote_revision,
                    paragraph_index: queued.paragraph_index.max(entry.paragraph_index),
                    total_paragraphs: queued
                        .total_paragraphs
                        .max(entry.total_paragraphs)
                        .max(entry.paragraph_index),
                });
            }
            _ => self.pending_reading_progress_write = Some(entry),
        }
    }

    fn dispatch_reader(&mut self, command: UiCommand) -> Effect {
        match self.reader.mode {
            crate::reader::ReaderMode::Normal | crate::reader::ReaderMode::Chat => match command {
                UiCommand::FocusPrevious => {
                    let request = self
                        .reader
                        .jump_live_previous_paragraph(self.settings.reading_speed_wpm);
                    self.track_reader_progress();
                    if let Some(request) = request {
                        return Effect::LoadReaderWindow(request);
                    }
                }
                UiCommand::FocusNext => {
                    let request = self
                        .reader
                        .jump_live_next_paragraph(self.settings.reading_speed_wpm);
                    self.track_reader_progress();
                    if let Some(request) = request {
                        return Effect::LoadReaderWindow(request);
                    }
                }
                UiCommand::Confirm => self.reader.pause(),
                UiCommand::Back => {
                    self.ui.route = UiRoute::Collection(self.reader.active_collection);
                    self.reader.unload_document();
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
                    self.reader.resume(self.settings.reading_speed_wpm);
                }
                UiCommand::Back => {
                    self.reader.open_paragraph_navigation();
                }
                UiCommand::Noop => {}
            },
            crate::reader::ReaderMode::ParagraphNavigation => match command {
                UiCommand::FocusPrevious => self.reader.move_paragraph(true),
                UiCommand::FocusNext => self.reader.move_paragraph(false),
                UiCommand::Confirm => {
                    let request = self
                        .reader
                        .commit_paragraph_navigation(self.settings.reading_speed_wpm);
                    self.track_reader_progress();
                    if let Some(request) = request {
                        return Effect::LoadReaderWindow(request);
                    }
                }
                UiCommand::Back => self.reader.close_paragraph_navigation(),
                UiCommand::Noop => {}
            },
            crate::reader::ReaderMode::LoadingContent => match command {
                UiCommand::Back => {
                    if let Some(pending) = self.pending_prepare.as_mut() {
                        pending.auto_open_reader = false;
                    }
                    self.ui.route = UiRoute::Collection(self.reader.active_collection);
                    self.reader.unload_document();
                }
                UiCommand::FocusPrevious
                | UiCommand::FocusNext
                | UiCommand::Confirm
                | UiCommand::Noop => {}
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

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}

const fn ignored_reason_for_manifest_item(
    package_state: PackageState,
) -> CollectionConfirmIgnoredReason {
    match package_state {
        PackageState::Fetching => CollectionConfirmIgnoredReason::AlreadyFetching,
        PackageState::PendingRemote => CollectionConfirmIgnoredReason::PendingRemote,
        PackageState::Failed => CollectionConfirmIgnoredReason::Failed,
        PackageState::Missing | PackageState::Stale | PackageState::Cached => {
            CollectionConfirmIgnoredReason::NotReady
        }
    }
}

fn collection_package_state_for_remote_item(
    collection: &crate::content::CollectionManifestState,
    remote_item_id: &crate::text::InlineText<{ crate::content::REMOTE_ITEM_ID_MAX_BYTES }>,
) -> Option<PackageState> {
    let mut index = 0usize;
    while index < collection.len() {
        if collection.items[index].remote_item_id == *remote_item_id {
            return Some(collection.items[index].package_state);
        }
        index += 1;
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        content::{
            CollectionManifestItem, CollectionManifestState, DetailLocator, PackageState,
            RECOMMENDATION_SUBTOPIC_SLUG_MAX_BYTES, RecommendationSubtopic,
            RecommendationSubtopicsState, RemoteContentStatus,
        },
        device::{BootState, DeviceState},
        formatter::{article_document_from_script, format_article_document},
        network::{NetworkState, NetworkStatus},
        reader::{ReaderParagraphInfo, ReaderWindow},
        runtime::CollectionConfirmIgnoredReason,
        settings::{AppearanceMode, PersistedSettings},
        storage::{StorageHealth, StorageRecoveryStatus},
        sync::SyncStatus,
    };

    fn make_ready_saved_item(package_state: PackageState) -> CollectionManifestItem {
        let mut item = CollectionManifestItem::empty();
        item.remote_item_id.set_truncated("saved-item-1");
        item.content_id.set_truncated("content-1");
        item.detail_locator = DetailLocator::Saved;
        item.meta.set_truncated("EXAMPLE / SAVED");
        item.title.set_truncated("Example saved title");
        item.remote_status = RemoteContentStatus::Ready;
        item.package_state = package_state;
        item
    }

    fn make_manifest_item(remote_item_id: &str, title: &str) -> CollectionManifestItem {
        let mut item = CollectionManifestItem::empty();
        item.remote_item_id.set_truncated(remote_item_id);
        item.content_id.set_truncated(remote_item_id);
        item.detail_locator = DetailLocator::Saved;
        item.meta.set_truncated("EXAMPLE / SAVED");
        item.title.set_truncated(title);
        item.remote_status = RemoteContentStatus::Ready;
        item.package_state = PackageState::Cached;
        item
    }

    fn make_storage_with_sd() -> StorageHealth {
        StorageHealth::available(1024, 1024, StorageRecoveryStatus::Clean).with_sd_card(
            true,
            4 * 1024 * 1024,
            3 * 1024 * 1024,
        )
    }

    fn make_reader_window(start_unit_index: u32, unit_count: u16) -> Box<ReaderWindow> {
        let mut window = Box::new(ReaderWindow::empty());
        window.start_unit_index = start_unit_index;
        window.unit_count = unit_count;
        window
    }

    fn make_recommendation_subtopic(
        slug: &str,
        label: &str,
        from_settings: bool,
        from_behavior: bool,
    ) -> RecommendationSubtopic {
        let mut topic = RecommendationSubtopic::empty();
        topic.slug.set_truncated(slug);
        topic.label.set_truncated(label);
        topic.parent_topic_label.set_truncated("TECHNOLOGY");
        topic.is_from_settings = from_settings;
        topic.is_from_behavior = from_behavior;
        topic
    }

    fn make_recommendation_subtopics() -> RecommendationSubtopicsState {
        let mut subtopics = RecommendationSubtopicsState::empty();
        let _ = subtopics.try_push(make_recommendation_subtopic("e-ink", "E Ink", true, false));
        let _ = subtopics.try_push(make_recommendation_subtopic(
            "small-web",
            "Small Web",
            false,
            true,
        ));
        subtopics
    }

    fn make_recommendation_manifest_item(content_id: &str, title: &str) -> CollectionManifestItem {
        let mut item = CollectionManifestItem::empty();
        item.remote_item_id.set_truncated(content_id);
        item.content_id.set_truncated(content_id);
        item.detail_locator = DetailLocator::Content;
        item.meta.set_truncated("A24 / FOR YOU");
        item.title.set_truncated(title);
        item.remote_status = RemoteContentStatus::Ready;
        item.package_state = PackageState::Cached;
        item
    }

    #[test]
    fn deep_sleep_bootstrap_hydrates_sleep_and_storage() {
        let snapshot = BootstrapSnapshot::new(
            DeviceState::with_boot(BootState::DeepSleepWake),
            42,
            None,
            None,
            None,
            Some(PersistedSettings::with_preferences(
                45_000,
                320,
                AppearanceMode::Dark,
                crate::settings::TopicPreferences::new(),
            )),
            StorageHealth::available(100, 200, StorageRecoveryStatus::Recovered),
            NetworkState::connecting(),
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
            None,
            None,
            None,
            StorageHealth::new(),
            NetworkState::disabled(),
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
    fn bootstrap_hydrates_recommendation_subtopics() {
        let snapshot = BootstrapSnapshot::new(
            DeviceState::with_boot(BootState::ColdBoot),
            7,
            None,
            None,
            Some(Box::new(make_recommendation_subtopics())),
            None,
            StorageHealth::new(),
            NetworkState::disabled(),
        );

        let store = Store::from_bootstrap(snapshot);

        assert_eq!(store.recommendations.subtopics.len(), 3);
        assert_eq!(store.recommendations.active_topic_slug.as_str(), "e-ink");
        assert_eq!(store.recommendations.active_topic_index(), Some(0));
    }

    #[test]
    fn network_events_update_store_state() {
        let mut store = Store::new();

        store
            .handle_event(Event::NetworkStatusChanged(NetworkStatus::Online), 0)
            .unwrap();

        assert_eq!(store.network.status, NetworkStatus::Online);
    }

    #[test]
    fn backend_sync_events_update_store_state() {
        let mut store = Store::new();

        store
            .handle_event(
                Event::BackendSyncStatusChanged(SyncStatus::SyncingContent),
                0,
            )
            .unwrap();

        assert_eq!(store.backend_sync.status, SyncStatus::SyncingContent);
    }

    #[test]
    fn dashboard_ui_tick_advances_sync_spinner_only_while_active() {
        let mut store = Store::new();
        store.ui.route = UiRoute::Dashboard;
        store
            .handle_event(
                Event::BackendSyncStatusChanged(SyncStatus::RefreshingSession),
                0,
            )
            .unwrap();

        store.handle_event(Event::UiTick(160), 0).unwrap();

        assert_eq!(store.backend_sync.spinner_phase, 1);

        store
            .handle_event(Event::BackendSyncStatusChanged(SyncStatus::Ready), 0)
            .unwrap();
        store.handle_event(Event::UiTick(320), 0).unwrap();

        assert_eq!(store.backend_sync.spinner_phase, 0);
    }

    #[test]
    fn collection_ui_tick_advances_sync_spinner_only_while_active() {
        let mut store = Store::new();
        store.ui.route = UiRoute::Collection(CollectionKind::Saved);
        store
            .handle_event(
                Event::BackendSyncStatusChanged(SyncStatus::SyncingContent),
                0,
            )
            .unwrap();

        store.handle_event(Event::UiTick(160), 0).unwrap();

        assert_eq!(store.backend_sync.spinner_phase, 1);

        store
            .handle_event(Event::BackendSyncStatusChanged(SyncStatus::Ready), 0)
            .unwrap();
        store.handle_event(Event::UiTick(320), 0).unwrap();

        assert_eq!(store.backend_sync.spinner_phase, 0);
    }

    #[test]
    fn dashboard_confirm_opens_selected_collection() {
        let mut store = Store::new();

        store.dispatch(Command::Ui(UiCommand::Confirm)).unwrap();

        assert_eq!(store.ui.route, UiRoute::Collection(CollectionKind::Saved));
    }

    #[test]
    fn saved_confirm_ignores_empty_collection() {
        let mut store = Store::new();
        store.ui.route = UiRoute::Collection(CollectionKind::Saved);

        let effect = store.dispatch(Command::Ui(UiCommand::Confirm)).unwrap();

        assert_eq!(
            effect,
            Effect::CollectionConfirmIgnored {
                collection: CollectionKind::Saved,
                reason: CollectionConfirmIgnoredReason::EmptyCollection,
            }
        );
    }

    #[test]
    fn saved_confirm_queues_when_backend_not_ready() {
        let mut store = Store::new();
        store.ui.route = UiRoute::Collection(CollectionKind::Saved);
        store.storage = make_storage_with_sd();
        store
            .handle_event(Event::NetworkStatusChanged(NetworkStatus::Online), 0)
            .unwrap();
        let mut manifest = CollectionManifestState::empty();
        let item = make_ready_saved_item(PackageState::Missing);
        let _ = manifest.try_push(item);
        store
            .content_mut()
            .update_collection(CollectionKind::Saved, manifest);

        let effect = store.dispatch(Command::Ui(UiCommand::Confirm)).unwrap();

        assert_eq!(effect, Effect::Noop);
        assert_eq!(
            store
                .content()
                .collection_state(CollectionKind::Saved)
                .item_at(0)
                .unwrap()
                .package_state,
            PackageState::Fetching
        );
        assert_eq!(
            store
                .handle_event(Event::BackendSyncStatusChanged(SyncStatus::Ready), 0)
                .unwrap(),
            Effect::PrepareContent(PrepareContentRequest::from_manifest(
                CollectionKind::Saved,
                item,
            ))
        );
    }

    #[test]
    fn saved_confirm_prepares_ready_missing_item() {
        let mut store = Store::new();
        store.ui.route = UiRoute::Collection(CollectionKind::Saved);
        store.storage = make_storage_with_sd();
        store
            .handle_event(Event::NetworkStatusChanged(NetworkStatus::Online), 0)
            .unwrap();
        store
            .handle_event(Event::BackendSyncStatusChanged(SyncStatus::Ready), 0)
            .unwrap();
        let mut manifest = CollectionManifestState::empty();
        let item = make_ready_saved_item(PackageState::Missing);
        let _ = manifest.try_push(item);
        store
            .content_mut()
            .update_collection(CollectionKind::Saved, manifest);

        let effect = store.dispatch(Command::Ui(UiCommand::Confirm)).unwrap();

        assert_eq!(
            effect,
            Effect::PrepareContent(PrepareContentRequest::from_manifest(
                CollectionKind::Saved,
                item,
            ))
        );
        assert_eq!(
            store
                .content()
                .collection_state(CollectionKind::Saved)
                .item_at(0)
                .unwrap()
                .package_state,
            PackageState::Fetching
        );
    }

    #[test]
    fn saved_confirm_retries_failed_item() {
        let mut store = Store::new();
        store.ui.route = UiRoute::Collection(CollectionKind::Saved);
        store.storage = make_storage_with_sd();
        store
            .handle_event(Event::NetworkStatusChanged(NetworkStatus::Online), 0)
            .unwrap();
        store
            .handle_event(Event::BackendSyncStatusChanged(SyncStatus::Ready), 0)
            .unwrap();
        let mut manifest = CollectionManifestState::empty();
        let item = make_ready_saved_item(PackageState::Failed);
        let _ = manifest.try_push(item);
        store
            .content_mut()
            .update_collection(CollectionKind::Saved, manifest);

        let effect = store.dispatch(Command::Ui(UiCommand::Confirm)).unwrap();

        assert_eq!(
            effect,
            Effect::PrepareContent(PrepareContentRequest::from_manifest(
                CollectionKind::Saved,
                item,
            ))
        );
        assert_eq!(
            store
                .content()
                .collection_state(CollectionKind::Saved)
                .item_at(0)
                .unwrap()
                .package_state,
            PackageState::Fetching
        );
    }

    #[test]
    fn saved_confirm_queues_until_network_recovers() {
        let mut store = Store::new();
        store.ui.route = UiRoute::Collection(CollectionKind::Saved);
        store.storage = make_storage_with_sd();
        store
            .handle_event(Event::BackendSyncStatusChanged(SyncStatus::Ready), 0)
            .unwrap();
        let mut manifest = CollectionManifestState::empty();
        let item = make_ready_saved_item(PackageState::Missing);
        let _ = manifest.try_push(item);
        store
            .content_mut()
            .update_collection(CollectionKind::Saved, manifest);

        let effect = store.dispatch(Command::Ui(UiCommand::Confirm)).unwrap();

        assert_eq!(effect, Effect::Noop);
        assert_eq!(
            store
                .handle_event(Event::NetworkStatusChanged(NetworkStatus::Online), 0)
                .unwrap(),
            Effect::PrepareContent(PrepareContentRequest::from_manifest(
                CollectionKind::Saved,
                item,
            ))
        );
    }

    #[test]
    fn collection_update_keeps_pending_prepare_item_selected() {
        let mut store = Store::new();
        store.ui.route = UiRoute::Collection(CollectionKind::Saved);
        store.storage = make_storage_with_sd();
        store
            .handle_event(Event::BackendSyncStatusChanged(SyncStatus::Ready), 0)
            .unwrap();

        let mut initial_manifest = CollectionManifestState::empty();
        let first = make_ready_saved_item(PackageState::Cached);
        let mut pending_item = make_ready_saved_item(PackageState::Missing);
        pending_item.remote_item_id.set_truncated("pending-item");
        let third = make_ready_saved_item(PackageState::Cached);
        let _ = initial_manifest.try_push(first);
        let _ = initial_manifest.try_push(pending_item);
        let _ = initial_manifest.try_push(third);
        store
            .content_mut()
            .update_collection(CollectionKind::Saved, initial_manifest);
        store.ui.saved_index = 1;

        assert_eq!(
            store.dispatch(Command::Ui(UiCommand::Confirm)).unwrap(),
            Effect::Noop
        );

        let mut refreshed_manifest = CollectionManifestState::empty();
        let refreshed_first = make_ready_saved_item(PackageState::Cached);
        let refreshed_second = make_ready_saved_item(PackageState::Cached);
        let mut refreshed_pending = pending_item;
        refreshed_pending.package_state = PackageState::Missing;
        let _ = refreshed_manifest.try_push(refreshed_first);
        let _ = refreshed_manifest.try_push(refreshed_second);
        let _ = refreshed_manifest.try_push(refreshed_pending);

        assert_eq!(
            store
                .handle_event(
                    Event::CollectionContentUpdated(
                        CollectionKind::Saved,
                        Box::new(refreshed_manifest),
                    ),
                    0,
                )
                .unwrap(),
            Effect::Noop
        );
        assert_eq!(store.ui.saved_index, 2);
        assert_eq!(
            store
                .content()
                .collection_state(CollectionKind::Saved)
                .item_at(2)
                .unwrap()
                .package_state,
            PackageState::Fetching
        );
    }

    #[test]
    fn auth_failed_restores_queued_prepare_state() {
        let mut store = Store::new();
        store.ui.route = UiRoute::Collection(CollectionKind::Saved);
        store.storage = make_storage_with_sd();
        store
            .handle_event(Event::NetworkStatusChanged(NetworkStatus::Online), 0)
            .unwrap();
        let mut manifest = CollectionManifestState::empty();
        let item = make_ready_saved_item(PackageState::Missing);
        let _ = manifest.try_push(item);
        store
            .content_mut()
            .update_collection(CollectionKind::Saved, manifest);

        assert_eq!(
            store.dispatch(Command::Ui(UiCommand::Confirm)).unwrap(),
            Effect::Noop
        );
        assert_eq!(
            store
                .handle_event(Event::BackendSyncStatusChanged(SyncStatus::AuthFailed), 0)
                .unwrap(),
            Effect::Noop
        );
        assert_eq!(
            store
                .content()
                .collection_state(CollectionKind::Saved)
                .item_at(0)
                .unwrap()
                .package_state,
            PackageState::Missing
        );
    }

    #[test]
    fn saved_confirm_ignores_when_storage_unavailable_even_if_backend_ready() {
        let mut store = Store::new();
        store.ui.route = UiRoute::Collection(CollectionKind::Saved);
        store
            .handle_event(Event::NetworkStatusChanged(NetworkStatus::Online), 0)
            .unwrap();
        store
            .handle_event(Event::BackendSyncStatusChanged(SyncStatus::Ready), 0)
            .unwrap();
        let mut manifest = CollectionManifestState::empty();
        let item = make_ready_saved_item(PackageState::Missing);
        let _ = manifest.try_push(item);
        store
            .content_mut()
            .update_collection(CollectionKind::Saved, manifest);

        let effect = store.dispatch(Command::Ui(UiCommand::Confirm)).unwrap();

        assert_eq!(
            effect,
            Effect::CollectionConfirmIgnored {
                collection: CollectionKind::Saved,
                reason: CollectionConfirmIgnoredReason::StorageUnavailable,
            }
        );
        assert_eq!(
            store
                .content()
                .collection_state(CollectionKind::Saved)
                .item_at(0)
                .unwrap()
                .package_state,
            PackageState::Missing
        );
    }

    #[test]
    fn saved_confirm_ignores_pending_remote_item() {
        let mut store = Store::new();
        store.ui.route = UiRoute::Collection(CollectionKind::Saved);
        store.storage = make_storage_with_sd();
        store
            .handle_event(Event::NetworkStatusChanged(NetworkStatus::Online), 0)
            .unwrap();
        store
            .handle_event(Event::BackendSyncStatusChanged(SyncStatus::Ready), 0)
            .unwrap();
        let mut manifest = CollectionManifestState::empty();
        let mut item = make_ready_saved_item(PackageState::PendingRemote);
        item.remote_status = RemoteContentStatus::Pending;
        let _ = manifest.try_push(item);
        store
            .content_mut()
            .update_collection(CollectionKind::Saved, manifest);

        let effect = store.dispatch(Command::Ui(UiCommand::Confirm)).unwrap();

        assert_eq!(
            effect,
            Effect::CollectionConfirmIgnored {
                collection: CollectionKind::Saved,
                reason: CollectionConfirmIgnoredReason::PendingRemote,
            }
        );
    }

    #[test]
    fn paused_reader_confirm_resumes_live_session() {
        let mut store = Store::new();
        store.settings.reading_speed_wpm = 300;
        store.ui.route = UiRoute::Reader;
        store.reader.pause();

        store.dispatch(Command::Ui(UiCommand::Confirm)).unwrap();

        assert!(matches!(
            store.reader.mode,
            crate::reader::ReaderMode::Normal
        ));
        assert!(
            store.reader.display_wpm(store.settings.reading_speed_wpm)
                < store.settings.reading_speed_wpm
        );
    }

    #[test]
    fn paused_reader_back_opens_paragraph_navigation() {
        let mut store = Store::new();
        store.ui.route = UiRoute::Reader;
        store.reader.pause();

        store.dispatch(Command::Ui(UiCommand::Back)).unwrap();

        assert!(matches!(
            store.reader.mode,
            crate::reader::ReaderMode::ParagraphNavigation
        ));
    }

    #[test]
    fn reader_back_unloads_document_before_returning_to_collection() {
        let mut store = Store::new();
        store.settings.reading_speed_wpm = 300;
        let article = store.content().article_at(CollectionKind::Saved, 0);
        let document = format_article_document(&article_document_from_script(
            article.source,
            article.script,
        ));
        store.reader.open_article(
            CollectionKind::Saved,
            article.id,
            crate::text::InlineText::from_slice(article.reader_title),
            alloc::boxed::Box::new(document),
            article.has_chat,
            store.settings.reading_speed_wpm,
        );
        store.ui.route = UiRoute::Reader;

        store.dispatch(Command::Ui(UiCommand::Back)).unwrap();

        assert_eq!(store.ui.route, UiRoute::Collection(CollectionKind::Saved));
        assert!(store.reader.is_empty());
        assert_eq!(store.reader.progress.total_paragraphs, 1);
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
        store.settings.reading_speed_wpm = 300;
        let article = store.content().article_at(CollectionKind::Inbox, 0);
        let document = format_article_document(&article_document_from_script(
            article.source,
            article.script,
        ));
        store.reader.open_article(
            CollectionKind::Inbox,
            article.id,
            crate::text::InlineText::from_slice(article.reader_title),
            alloc::boxed::Box::new(document),
            article.has_chat,
            store.settings.reading_speed_wpm,
        );
        store.ui.route = UiRoute::Reader;
        let before = store.reader.progress.unit_index;

        store.handle_event(Event::ReaderTick(0), 0).unwrap();
        store.handle_event(Event::ReaderTick(1_000), 0).unwrap();

        assert!(store.reader.progress.unit_index > before);
        assert_eq!(store.ui.route, UiRoute::Reader);
    }

    #[test]
    fn active_reader_tick_keeps_sleep_awake() {
        let mut store = Store::new();
        store.settings.reading_speed_wpm = 300;
        let article = store.content().article_at(CollectionKind::Inbox, 0);
        let document = format_article_document(&article_document_from_script(
            article.source,
            article.script,
        ));
        store.reader.open_article(
            CollectionKind::Inbox,
            article.id,
            crate::text::InlineText::from_slice(article.reader_title),
            alloc::boxed::Box::new(document),
            article.has_chat,
            store.settings.reading_speed_wpm,
        );
        store.ui.route = UiRoute::Reader;
        store.sleep.last_activity_ms = 10;

        store.handle_event(Event::ReaderTick(250), 0).unwrap();

        assert_eq!(store.sleep.last_activity_ms, 250);
    }

    #[test]
    fn paused_reader_tick_does_not_refresh_sleep_timer() {
        let mut store = Store::new();
        store.settings.reading_speed_wpm = 300;
        let article = store.content().article_at(CollectionKind::Inbox, 0);
        let document = format_article_document(&article_document_from_script(
            article.source,
            article.script,
        ));
        store.reader.open_article(
            CollectionKind::Inbox,
            article.id,
            crate::text::InlineText::from_slice(article.reader_title),
            alloc::boxed::Box::new(document),
            article.has_chat,
            store.settings.reading_speed_wpm,
        );
        store.ui.route = UiRoute::Reader;
        store.reader.pause();
        store.sleep.last_activity_ms = 10;

        store.handle_event(Event::ReaderTick(250), 0).unwrap();

        assert_eq!(store.sleep.last_activity_ms, 10);
    }

    #[test]
    fn live_reader_scroll_back_jumps_to_current_paragraph_start() {
        let mut store = Store::new();
        store.settings.reading_speed_wpm = 300;
        store.open_cached_content(
            CollectionKind::Inbox,
            crate::text::InlineText::from_slice("content-1"),
            crate::text::InlineText::from_slice("Example"),
            120,
            alloc::vec![
                ReaderParagraphInfo {
                    start_unit_index: 0,
                    preview: crate::text::InlineText::new(),
                },
                ReaderParagraphInfo {
                    start_unit_index: 10,
                    preview: crate::text::InlineText::new(),
                },
                ReaderParagraphInfo {
                    start_unit_index: 20,
                    preview: crate::text::InlineText::new(),
                },
            ]
            .into_boxed_slice(),
            make_reader_window(0, 64),
        );
        store.reader.progress.unit_index = 14;
        store.reader.progress.paragraph_index = 2;
        store.reader.progress.total_paragraphs = 3;
        store.reader.next_due_at_ms = Some(1_000);

        let effect = store
            .dispatch(Command::Ui(UiCommand::FocusPrevious))
            .unwrap();

        assert_eq!(effect, Effect::Noop);
        assert_eq!(store.reader.progress.unit_index, 10);
        assert_eq!(store.reader.progress.paragraph_index, 2);
        assert_eq!(store.reader.next_due_at_ms, None);
        assert!(
            store.reader.display_wpm(store.settings.reading_speed_wpm)
                < store.settings.reading_speed_wpm
        );
    }

    #[test]
    fn live_reader_scroll_forward_requests_reader_window_for_next_paragraph() {
        let mut store = Store::new();
        store.settings.reading_speed_wpm = 300;
        store.open_cached_content(
            CollectionKind::Inbox,
            crate::text::InlineText::from_slice("content-1"),
            crate::text::InlineText::from_slice("Example"),
            200,
            alloc::vec![
                ReaderParagraphInfo {
                    start_unit_index: 0,
                    preview: crate::text::InlineText::new(),
                },
                ReaderParagraphInfo {
                    start_unit_index: 64,
                    preview: crate::text::InlineText::new(),
                },
            ]
            .into_boxed_slice(),
            make_reader_window(0, 32),
        );

        let effect = store.dispatch(Command::Ui(UiCommand::FocusNext)).unwrap();

        assert_eq!(
            effect,
            Effect::LoadReaderWindow(crate::reader::ReaderWindowLoadRequest {
                content_id: crate::text::InlineText::from_slice("content-1"),
                window_start_unit_index: 32,
            })
        );
        assert_eq!(store.reader.progress.unit_index, 0);
        assert!(
            store.reader.display_wpm(store.settings.reading_speed_wpm)
                < store.settings.reading_speed_wpm
        );
    }

    #[test]
    fn paragraph_navigation_scroll_still_moves_selected_paragraph() {
        let mut store = Store::new();
        store.ui.route = UiRoute::Reader;
        store.reader.mode = crate::reader::ReaderMode::ParagraphNavigation;
        store.reader.progress.paragraph_index = 2;
        store.reader.progress.total_paragraphs = 4;

        let effect = store.dispatch(Command::Ui(UiCommand::FocusNext)).unwrap();

        assert_eq!(effect, Effect::Noop);
        assert_eq!(store.reader.progress.paragraph_index, 3);
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

    #[test]
    fn saved_content_events_replace_live_saved_manifest() {
        let mut store = Store::new();
        let mut saved_manifest = CollectionManifestState::empty();
        let mut item = CollectionManifestItem::empty();
        item.remote_item_id.set_truncated("saved-item-1");
        item.content_id.set_truncated("content-1");
        item.detail_locator = DetailLocator::Saved;
        item.meta.set_truncated("EXAMPLE / SAVED");
        item.title.set_truncated("Example saved title");
        item.remote_status = RemoteContentStatus::Ready;
        item.package_state = PackageState::Missing;
        let _ = saved_manifest.try_push(item);
        store.ui.saved_index = 4;

        store
            .handle_event(
                Event::CollectionContentUpdated(
                    CollectionKind::Saved,
                    alloc::boxed::Box::new(saved_manifest),
                ),
                0,
            )
            .unwrap();

        assert_eq!(
            store.content().collection_state(CollectionKind::Saved),
            &saved_manifest
        );
        assert_eq!(store.ui.saved_index, 0);
    }

    #[test]
    fn empty_collection_update_does_not_allocate_live_content_state() {
        let mut store = Store::new();

        store
            .handle_event(
                Event::CollectionContentUpdated(
                    CollectionKind::Saved,
                    alloc::boxed::Box::new(CollectionManifestState::empty()),
                ),
                0,
            )
            .unwrap();

        assert!(store.content.is_none());
        assert!(
            store
                .content()
                .collection_state(CollectionKind::Saved)
                .is_empty()
        );
        assert_eq!(store.ui.saved_index, 0);
    }

    #[test]
    fn dashboard_focus_previous_stops_at_first_item() {
        let mut store = Store::new();
        store.ui.route = UiRoute::Dashboard;
        store.ui.dashboard_focus = crate::ui::DashboardFocus::Inbox;

        let effect = store
            .dispatch(Command::Ui(UiCommand::FocusPrevious))
            .unwrap();

        assert_eq!(effect, Effect::Noop);
        assert_eq!(store.ui.dashboard_focus, crate::ui::DashboardFocus::Inbox);
    }

    #[test]
    fn collection_focus_next_stops_at_last_item() {
        let mut store = Store::new();
        store.ui.route = UiRoute::Collection(CollectionKind::Saved);
        let mut collection = CollectionManifestState::empty();
        let _ = collection.try_push(make_manifest_item("saved-1", "First"));
        let _ = collection.try_push(make_manifest_item("saved-2", "Second"));
        store
            .content_mut()
            .update_collection(CollectionKind::Saved, collection);
        store.ui.saved_index = 1;

        let effect = store.dispatch(Command::Ui(UiCommand::FocusNext)).unwrap();

        assert_eq!(effect, Effect::Noop);
        assert_eq!(store.ui.saved_index, 1);
    }

    #[test]
    fn recommendations_subtopic_focus_next_stops_at_last_item() {
        let mut store = Store::new();
        store.ui.route = UiRoute::Collection(CollectionKind::Recommendations);
        store.ui.recommendations_region = RecommendationsRegion::Subtopics;
        store
            .recommendations
            .set_subtopics(make_recommendation_subtopics());
        store.ui.recommendations_subtopic_index = 2;

        let effect = store.dispatch(Command::Ui(UiCommand::FocusNext)).unwrap();

        assert_eq!(effect, Effect::Noop);
        assert_eq!(store.ui.recommendations_subtopic_index, 2);
    }

    #[test]
    fn settings_focus_previous_stops_at_first_row() {
        let mut store = Store::new();
        store.ui.route = UiRoute::Settings;
        store.ui.settings_mode = SettingsMode::Master;
        store.ui.settings_row = SettingsRow::ReadingSpeed;

        let effect = store
            .dispatch(Command::Ui(UiCommand::FocusPrevious))
            .unwrap();

        assert_eq!(effect, Effect::Noop);
        assert_eq!(store.ui.settings_row, SettingsRow::ReadingSpeed);
    }

    #[test]
    fn topic_preferences_focus_next_stops_at_last_chip() {
        let mut store = Store::new();
        store.ui.route = UiRoute::Settings;
        store.ui.settings_mode = SettingsMode::TopicPreferences;
        store.ui.topic_focus.region = TopicRegion::Chips;
        store.ui.topic_focus.chip_index = crate::settings::TOPIC_CHIP_COUNT - 1;

        let effect = store.dispatch(Command::Ui(UiCommand::FocusNext)).unwrap();

        assert_eq!(effect, Effect::Noop);
        assert_eq!(
            store.ui.topic_focus.chip_index,
            crate::settings::TOPIC_CHIP_COUNT - 1
        );
    }

    #[test]
    fn dashboard_enter_recommendations_uses_article_region_when_topic_is_ready() {
        let mut store = Store::new();
        store.ui.dashboard_focus = crate::ui::DashboardFocus::Recommendations;
        store
            .recommendations
            .set_subtopics(make_recommendation_subtopics());
        store.recommendations.set_active_topic(
            crate::text::InlineText::<RECOMMENDATION_SUBTOPIC_SLUG_MAX_BYTES>::from_slice("e-ink"),
            false,
        );
        let mut collection = CollectionManifestState::empty();
        let _ = collection.try_push(make_recommendation_manifest_item(
            "rec-1",
            "Why single-purpose readers feel radical again",
        ));
        store
            .content_mut()
            .update_collection(CollectionKind::Recommendations, collection);

        let effect = store.dispatch(Command::Ui(UiCommand::Confirm)).unwrap();

        assert_eq!(effect, Effect::Noop);
        assert_eq!(
            store.ui.route,
            UiRoute::Collection(CollectionKind::Recommendations)
        );
        assert_eq!(
            store.ui.recommendations_region,
            RecommendationsRegion::Articles
        );
        assert_eq!(store.ui.recommendations_subtopic_index, 0);
    }

    #[test]
    fn dashboard_warm_subtopics_do_not_prefetch_articles() {
        let mut store = Store::new();

        let effect = store
            .handle_event(
                Event::RecommendationSubtopicsUpdated(Box::new(make_recommendation_subtopics())),
                0,
            )
            .unwrap();

        assert_eq!(effect, Effect::Noop);
        assert_eq!(store.recommendations.subtopics.len(), 3);
        assert_eq!(store.recommendations.active_topic_slug.as_str(), "e-ink");
        assert!(
            store
                .content()
                .collection_state(CollectionKind::Recommendations)
                .is_empty()
        );
        assert!(!store.recommendations.topic_loading);
    }

    #[test]
    fn recommendations_back_moves_from_articles_to_subtopics_then_dashboard() {
        let mut store = Store::new();
        store.ui.route = UiRoute::Collection(CollectionKind::Recommendations);
        store.ui.recommendations_region = RecommendationsRegion::Articles;
        store.ui.dashboard_focus = crate::ui::DashboardFocus::Saved;
        store
            .recommendations
            .set_subtopics(make_recommendation_subtopics());
        store.recommendations.set_active_topic(
            crate::text::InlineText::<RECOMMENDATION_SUBTOPIC_SLUG_MAX_BYTES>::from_slice("e-ink"),
            false,
        );

        let first = store.dispatch(Command::Ui(UiCommand::Back)).unwrap();

        assert_eq!(first, Effect::Noop);
        assert_eq!(
            store.ui.recommendations_region,
            RecommendationsRegion::Subtopics
        );
        assert_eq!(
            store.ui.route,
            UiRoute::Collection(CollectionKind::Recommendations)
        );
        assert_eq!(store.ui.recommendations_focus_flash_ticks, 8);

        let second = store.dispatch(Command::Ui(UiCommand::Back)).unwrap();

        assert_eq!(second, Effect::Noop);
        assert_eq!(store.ui.route, UiRoute::Dashboard);
        assert_eq!(
            store.ui.dashboard_focus,
            crate::ui::DashboardFocus::Recommendations
        );
        assert_eq!(store.ui.recommendations_focus_flash_ticks, 0);
    }

    #[test]
    fn recommendations_confirm_on_new_subtopic_loads_that_topic() {
        let mut store = Store::new();
        store.ui.route = UiRoute::Collection(CollectionKind::Recommendations);
        store.ui.recommendations_region = RecommendationsRegion::Subtopics;
        store.ui.recommendations_subtopic_index = 1;
        store
            .recommendations
            .set_subtopics(make_recommendation_subtopics());
        store.recommendations.set_active_topic(
            crate::text::InlineText::<RECOMMENDATION_SUBTOPIC_SLUG_MAX_BYTES>::from_slice("e-ink"),
            false,
        );
        let mut existing = CollectionManifestState::empty();
        let _ = existing.try_push(make_recommendation_manifest_item("rec-1", "Existing topic"));
        store
            .content_mut()
            .update_collection(CollectionKind::Recommendations, existing);

        let effect = store.dispatch(Command::Ui(UiCommand::Confirm)).unwrap();

        assert_eq!(
            effect,
            Effect::LoadRecommendationTopic(RecommendationTopicRequest::new(
                crate::text::InlineText::<RECOMMENDATION_SUBTOPIC_SLUG_MAX_BYTES>::from_slice(
                    "small-web",
                ),
            ))
        );
        assert_eq!(
            store.recommendations.active_topic_slug.as_str(),
            "small-web"
        );
        assert!(store.recommendations.topic_loading);
        assert_eq!(
            store.ui.recommendations_region,
            RecommendationsRegion::Articles
        );
        assert!(
            store
                .content()
                .collection_state(CollectionKind::Recommendations)
                .is_empty()
        );
    }

    #[test]
    fn recommendations_previous_from_first_article_returns_to_subtopics() {
        let mut store = Store::new();
        store.ui.route = UiRoute::Collection(CollectionKind::Recommendations);
        store.ui.recommendations_region = RecommendationsRegion::Articles;
        store.ui.recommendations_index = 0;
        store
            .recommendations
            .set_subtopics(make_recommendation_subtopics());
        store.recommendations.set_active_topic(
            crate::text::InlineText::<RECOMMENDATION_SUBTOPIC_SLUG_MAX_BYTES>::from_slice("e-ink"),
            false,
        );
        let mut collection = CollectionManifestState::empty();
        let _ = collection.try_push(make_recommendation_manifest_item("rec-1", "First"));
        let _ = collection.try_push(make_recommendation_manifest_item("rec-2", "Second"));
        store
            .content_mut()
            .update_collection(CollectionKind::Recommendations, collection);

        let effect = store
            .dispatch(Command::Ui(UiCommand::FocusPrevious))
            .unwrap();

        assert_eq!(effect, Effect::Noop);
        assert_eq!(
            store.ui.recommendations_region,
            RecommendationsRegion::Subtopics
        );
        assert_eq!(store.ui.recommendations_index, 0);
        assert_eq!(store.ui.recommendations_focus_flash_ticks, 8);
    }
}
