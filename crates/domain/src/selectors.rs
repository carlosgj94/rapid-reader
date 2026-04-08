use crate::{
    content::{
        CONTENT_META_MAX_BYTES, CONTENT_TITLE_MAX_BYTES, CollectionKind, CollectionManifestItem,
        CollectionManifestState, ContentState, PackageState,
        RECOMMENDATION_SUBTOPIC_LABEL_MAX_BYTES, ReadingProgressEntry, ReadingProgressState,
        RecommendationSubtopic,
    },
    formatter::{MAX_PARAGRAPH_PREVIEW_BYTES, MAX_STAGE_SEGMENT_BYTES, StageFont},
    network::NetworkStatus,
    reader::ReaderMode,
    settings::{
        AppearanceMode, TOPIC_CATEGORY_COUNT, TOPIC_CHIP_COUNT, topic_category_label,
        topic_chip_label,
    },
    store::Store,
    text::InlineText,
    ui::{DashboardFocus, RecommendationsRegion, SettingsMode, TopicRegion, UiRoute},
};

pub const VISIBLE_LIST_ROWS: usize = 3;
pub const SETTINGS_ROW_COUNT: usize = 6;
pub const RECOMMENDATION_VISIBLE_TABS: usize = 4;
pub const RECOMMENDATION_TAB_LABEL_MAX_BYTES: usize = RECOMMENDATION_SUBTOPIC_LABEL_MAX_BYTES + 1;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct StatusClusterModel {
    pub battery_percent: u8,
    pub network: NetworkStatus,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct DashboardItemModel {
    pub label: &'static str,
    pub live_dot: bool,
    pub selected: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct DashboardScreenModel {
    pub appearance: AppearanceMode,
    pub status: StatusClusterModel,
    pub sync_indicator: Option<SyncIndicatorModel>,
    pub rail_label: &'static str,
    pub items: [DashboardItemModel; VISIBLE_LIST_ROWS],
    pub focused: DashboardFocus,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct SyncIndicatorModel {
    pub label: &'static str,
    pub spinner_phase: u8,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ContentRowModel {
    pub meta: InlineText<CONTENT_META_MAX_BYTES>,
    pub title: InlineText<CONTENT_TITLE_MAX_BYTES>,
    pub progress_badge: Option<InlineText<8>>,
    pub is_fetching: bool,
    pub selected: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ContentListScreenModel {
    pub appearance: AppearanceMode,
    pub status: StatusClusterModel,
    pub rail_label: &'static str,
    pub recommendations_bar: Option<RecommendationBarModel>,
    pub rows: [ContentRowModel; VISIBLE_LIST_ROWS],
    pub selected_collection: CollectionKind,
    pub selected_index: usize,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct RecommendationTabModel {
    pub label: InlineText<RECOMMENDATION_TAB_LABEL_MAX_BYTES>,
    pub active: bool,
    pub focused: bool,
    pub flash: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct RecommendationBarModel {
    pub tabs: [RecommendationTabModel; RECOMMENDATION_VISIBLE_TABS],
    pub visible_count: usize,
    pub show_left_more: bool,
    pub show_right_more: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PauseActionModel {
    pub label: &'static str,
    pub action: &'static str,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ReaderLoadingModel {
    pub progress_width: u16,
    pub stripe_phase: u8,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ReaderModalModel {
    Pause([PauseActionModel; 3]),
    Loading(ReaderLoadingModel),
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ReaderScreenModel {
    pub appearance: AppearanceMode,
    pub title: InlineText<CONTENT_TITLE_MAX_BYTES>,
    pub wpm: u16,
    pub left_word: InlineText<MAX_STAGE_SEGMENT_BYTES>,
    pub right_word: InlineText<MAX_STAGE_SEGMENT_BYTES>,
    pub preview: InlineText<MAX_PARAGRAPH_PREVIEW_BYTES>,
    pub font: StageFont,
    pub progress_width: u16,
    pub show_chat_badge: bool,
    pub modal: Option<ReaderModalModel>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ParagraphNavigationModel {
    pub appearance: AppearanceMode,
    pub title: InlineText<CONTENT_TITLE_MAX_BYTES>,
    pub current_index: u16,
    pub total: u16,
    pub previous_top: InlineText<MAX_PARAGRAPH_PREVIEW_BYTES>,
    pub selected_excerpt: InlineText<MAX_PARAGRAPH_PREVIEW_BYTES>,
    pub previous_bottom: InlineText<MAX_PARAGRAPH_PREVIEW_BYTES>,
    pub final_excerpt: InlineText<MAX_PARAGRAPH_PREVIEW_BYTES>,
    pub tick_index: u8,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct SettingsRowModel {
    pub label: &'static str,
    pub value: Option<&'static str>,
    pub selected: bool,
    pub show_arrow: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct TopicCategoryModel {
    pub label: &'static str,
    pub selected: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct TopicChipModel {
    pub label: &'static str,
    pub selected: bool,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct TopicPreferencesModel {
    pub title: &'static str,
    pub categories: [TopicCategoryModel; TOPIC_CATEGORY_COUNT],
    pub chips: [TopicChipModel; TOPIC_CHIP_COUNT],
    pub focus_region: TopicRegion,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct SettingsScreenModel {
    pub appearance: AppearanceMode,
    pub title: &'static str,
    pub mode: SettingsMode,
    pub rows: [SettingsRowModel; SETTINGS_ROW_COUNT],
    pub refresh_title: Option<&'static str>,
    pub refresh_body: Option<&'static str>,
    pub topic_preferences: Option<TopicPreferencesModel>,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ActiveScreenModel {
    Dashboard(DashboardScreenModel),
    Collection(ContentListScreenModel),
    Reader(ReaderScreenModel),
    ParagraphNavigation(ParagraphNavigationModel),
    Settings(SettingsScreenModel),
}

pub fn select_active_screen(store: &Store) -> ActiveScreenModel {
    match store.ui.route {
        UiRoute::Dashboard => ActiveScreenModel::Dashboard(select_dashboard(store)),
        UiRoute::Collection(kind) => ActiveScreenModel::Collection(select_collection(store, kind)),
        UiRoute::Reader => match store.reader.mode {
            ReaderMode::ParagraphNavigation => {
                ActiveScreenModel::ParagraphNavigation(select_paragraph_navigation(store))
            }
            _ => ActiveScreenModel::Reader(select_reader(store)),
        },
        UiRoute::Settings => ActiveScreenModel::Settings(select_settings(store)),
    }
}

pub fn select_dashboard(store: &Store) -> DashboardScreenModel {
    let focused = store.ui.dashboard_focus;
    let focused_index = focused.index();

    DashboardScreenModel {
        appearance: store.settings.appearance,
        status: select_status(store),
        sync_indicator: store.backend_sync.shows_dashboard_indicator().then_some(
            SyncIndicatorModel {
                label: "syncing...",
                spinner_phase: store.backend_sync.spinner_phase,
            },
        ),
        rail_label: "M\nO\nT\nI\nF",
        items: [
            focused_index
                .checked_sub(1)
                .map(DashboardFocus::from_index)
                .map(|focus| dashboard_item(focus.as_collection(), false))
                .unwrap_or_else(dashboard_empty_item),
            dashboard_item(focused.as_collection(), true),
            if focused_index + 1 < DashboardFocus::COUNT {
                dashboard_item(
                    DashboardFocus::from_index(focused_index + 1).as_collection(),
                    false,
                )
            } else {
                dashboard_empty_item()
            },
        ],
        focused,
    }
}

pub fn select_collection(store: &Store, kind: CollectionKind) -> ContentListScreenModel {
    let selected_index = store.ui.collection_index(kind);
    let rows = if matches!(kind, CollectionKind::Recommendations) {
        select_recommendation_rows(store)
    } else {
        select_collection_rows(
            store.content(),
            &store.reading_progress,
            kind,
            selected_index,
        )
    };

    ContentListScreenModel {
        appearance: store.settings.appearance,
        status: select_status(store),
        rail_label: kind.rail_label(),
        recommendations_bar: matches!(kind, CollectionKind::Recommendations)
            .then_some(select_recommendation_bar(store)),
        rows,
        selected_collection: kind,
        selected_index,
    }
}

pub fn select_reader(store: &Store) -> ReaderScreenModel {
    let current_unit = store.reader.current_unit();
    let stage_token = current_unit.stage_token();
    let preview = store
        .reader
        .preview_for_paragraph(store.reader.progress.paragraph_index);

    ReaderScreenModel {
        appearance: store.settings.appearance,
        title: store.reader.title,
        // Surface the live cadence, but only at quantized speed steps so reader ticks do not
        // force a screen refresh every 20 ms on the Sharp panel path.
        wpm: store.reader.display_wpm(store.settings.reading_speed_wpm),
        left_word: stage_token.left,
        right_word: stage_token.right,
        preview,
        font: stage_token.font,
        progress_width: store.reader.progress_width_px(),
        show_chat_badge: matches!(store.reader.mode, ReaderMode::Chat),
        modal: reader_modal_model(store),
    }
}

fn reader_modal_model(store: &Store) -> Option<ReaderModalModel> {
    match store.reader.mode {
        ReaderMode::Paused => Some(ReaderModalModel::Pause([
            PauseActionModel {
                label: "LONG PRESS ->",
                action: "PARAGRAPH VIEW",
            },
            PauseActionModel {
                label: "SHORT PRESS ->",
                action: "RESUME RSVP",
            },
            PauseActionModel {
                label: "ROTATE ->",
                action: "ADJUST RSVP SPEED",
            },
        ])),
        ReaderMode::LoadingContent => Some(ReaderModalModel::Loading(loading_modal_model(store))),
        _ => None,
    }
}

fn loading_modal_model(store: &Store) -> ReaderLoadingModel {
    ReaderLoadingModel {
        progress_width: store.reader.prepare_display_progress_width_px(214),
        stripe_phase: store.reader.prepare_stripe_phase(),
    }
}

pub fn select_paragraph_navigation(store: &Store) -> ParagraphNavigationModel {
    let current_index = store.reader.progress.paragraph_index as usize;
    let total = store.reader.progress.total_paragraphs;
    let current_zero_based = current_index.saturating_sub(1);
    let previous_top = if current_zero_based > 0 {
        store
            .reader
            .preview_for_paragraph(current_zero_based as u16)
    } else {
        InlineText::new()
    };
    let selected_excerpt = store
        .reader
        .preview_for_paragraph((current_zero_based + 1) as u16);
    let previous_bottom = if (current_zero_based + 1) < total as usize {
        store
            .reader
            .preview_for_paragraph((current_zero_based + 2) as u16)
    } else {
        InlineText::new()
    };
    let final_excerpt = if (current_zero_based + 2) < total as usize {
        store
            .reader
            .preview_for_paragraph((current_zero_based + 3) as u16)
    } else {
        InlineText::new()
    };
    let tick_index = paragraph_tick_index(store.reader.progress.paragraph_index, total);

    ParagraphNavigationModel {
        appearance: store.settings.appearance,
        title: store.reader.title,
        current_index: store.reader.progress.paragraph_index,
        total,
        previous_top,
        selected_excerpt,
        previous_bottom,
        final_excerpt,
        tick_index,
    }
}

fn paragraph_tick_index(current_index: u16, total: u16) -> u8 {
    if total <= 1 {
        return 0;
    }

    let current_zero_based = current_index.saturating_sub(1).min(total.saturating_sub(1)) as u32;
    let total_zero_based = total.saturating_sub(1) as u32;

    (((current_zero_based * 6) + (total_zero_based / 2)) / total_zero_based).min(6) as u8
}

pub fn select_settings(store: &Store) -> SettingsScreenModel {
    let rows = [
        SettingsRowModel {
            label: "Reading Speed",
            value: Some(store.settings.reading_speed_label()),
            selected: matches!(store.ui.settings_row, crate::ui::SettingsRow::ReadingSpeed),
            show_arrow: false,
        },
        SettingsRowModel {
            label: "Appearance",
            value: Some(store.settings.appearance.label()),
            selected: matches!(store.ui.settings_row, crate::ui::SettingsRow::Appearance),
            show_arrow: false,
        },
        SettingsRowModel {
            label: "Refresh Data",
            value: None,
            selected: matches!(store.ui.settings_row, crate::ui::SettingsRow::RefreshData),
            show_arrow: false,
        },
        SettingsRowModel {
            label: "Topic Preferences",
            value: None,
            selected: matches!(
                store.ui.settings_row,
                crate::ui::SettingsRow::TopicPreferences
            ),
            show_arrow: true,
        },
        SettingsRowModel {
            label: "Network Connection",
            value: Some(store.network.status.label()),
            selected: matches!(
                store.ui.settings_row,
                crate::ui::SettingsRow::NetworkConnection
            ),
            show_arrow: false,
        },
        SettingsRowModel {
            label: "Connect Account",
            value: None,
            selected: matches!(
                store.ui.settings_row,
                crate::ui::SettingsRow::ConnectAccount
            ),
            show_arrow: true,
        },
    ];

    SettingsScreenModel {
        appearance: store.settings.appearance,
        title: match store.ui.settings_mode {
            SettingsMode::TopicPreferences => "TOPIC PREFERENCES",
            _ => "GENERAL SETTINGS",
        },
        mode: store.ui.settings_mode,
        rows,
        refresh_title: matches!(store.ui.settings_mode, SettingsMode::RefreshLoading)
            .then_some("REFRESHING DATA"),
        refresh_body: matches!(store.ui.settings_mode, SettingsMode::RefreshLoading)
            .then_some("This may take a moment."),
        topic_preferences: matches!(store.ui.settings_mode, SettingsMode::TopicPreferences)
            .then_some(select_topic_preferences(store)),
    }
}

fn select_topic_preferences(store: &Store) -> TopicPreferencesModel {
    let mut categories = [TopicCategoryModel {
        label: "",
        selected: false,
    }; TOPIC_CATEGORY_COUNT];
    let mut chips = [TopicChipModel {
        label: "",
        selected: false,
        enabled: false,
    }; TOPIC_CHIP_COUNT];

    let category_index = store
        .ui
        .topic_focus
        .category_index
        .min(TOPIC_CATEGORY_COUNT - 1);

    let mut idx = 0;
    while idx < TOPIC_CATEGORY_COUNT {
        categories[idx] = TopicCategoryModel {
            label: topic_category_label(idx),
            selected: idx == category_index,
        };
        idx += 1;
    }

    let mut chip_index = 0;
    while chip_index < TOPIC_CHIP_COUNT {
        chips[chip_index] = TopicChipModel {
            label: topic_chip_label(category_index, chip_index),
            selected: matches!(store.ui.topic_focus.region, TopicRegion::Chips)
                && chip_index == store.ui.topic_focus.chip_index,
            enabled: store.settings.topics.enabled_by_category[category_index][chip_index],
        };
        chip_index += 1;
    }

    TopicPreferencesModel {
        title: "TOPIC PREFERENCES",
        categories,
        chips,
        focus_region: store.ui.topic_focus.region,
    }
}

fn select_collection_rows(
    content: &ContentState,
    reading_progress: &ReadingProgressState,
    kind: CollectionKind,
    selected_index: usize,
) -> [ContentRowModel; VISIBLE_LIST_ROWS] {
    select_manifest_collection_rows(
        content.collection_state(kind),
        reading_progress,
        kind,
        selected_index,
    )
}

fn select_recommendation_rows(store: &Store) -> [ContentRowModel; VISIBLE_LIST_ROWS] {
    if store.recommendations.subtopics_loading && store.recommendations.subtopics.is_empty() {
        return [
            content_row("", "", false),
            content_row("MOTIF", "Loading topics...", true),
            content_row("NETWORK / SYNC", "Building your recommendations", false),
        ];
    }

    if store.recommendations.topic_loading {
        let topic_label = store
            .recommendations
            .active_subtopic()
            .map(recommendation_topic_meta)
            .unwrap_or_else(|| InlineText::from_slice("FOR YOU"));
        return [
            content_row("", "", false),
            ContentRowModel {
                meta: topic_label,
                title: InlineText::from_slice("Loading articles..."),
                progress_badge: None,
                is_fetching: false,
                selected: true,
            },
            content_row("MOTIF", "This may take a moment", false),
        ];
    }

    let collection = store
        .content()
        .collection_state(CollectionKind::Recommendations);
    if collection.is_empty() {
        if store.recommendations.subtopics.is_empty() {
            return [
                content_row("", "", false),
                content_row("MOTIF", "No recommendations synced yet", true),
                content_row("NETWORK / SYNC", "Refresh data after pairing", false),
            ];
        }

        let topic_label = store
            .recommendations
            .active_subtopic()
            .map(recommendation_topic_meta)
            .unwrap_or_else(|| InlineText::from_slice("FOR YOU"));
        return [
            content_row("", "", false),
            ContentRowModel {
                meta: topic_label,
                title: InlineText::from_slice("No articles for this topic yet"),
                progress_badge: None,
                is_fetching: false,
                selected: true,
            },
            content_row("MOTIF", "Try another subtopic", false),
        ];
    }

    select_manifest_collection_rows(
        collection,
        &store.reading_progress,
        CollectionKind::Recommendations,
        store.ui.recommendations_index,
    )
}

fn select_recommendation_bar(store: &Store) -> RecommendationBarModel {
    let mut tabs = [RecommendationTabModel {
        label: InlineText::new(),
        active: false,
        focused: false,
        flash: false,
    }; RECOMMENDATION_VISIBLE_TABS];

    let len = store.recommendations.subtopics.len();
    if len == 0 {
        return RecommendationBarModel {
            tabs,
            visible_count: 0,
            show_left_more: false,
            show_right_more: false,
        };
    }

    let focus_index = store
        .ui
        .recommendations_subtopic_index
        .min(len.saturating_sub(1));
    let window_start = recommendation_tab_window_start(focus_index, len);
    let visible_count = (len - window_start).min(RECOMMENDATION_VISIBLE_TABS);
    let active_slug = store.recommendations.active_topic_slug;
    let mut index = 0usize;
    while index < visible_count {
        if let Some(subtopic) = store
            .recommendations
            .subtopics
            .item_at(window_start + index)
        {
            let is_active = subtopic.slug == active_slug;
            let is_focused = matches!(
                store.ui.recommendations_region,
                RecommendationsRegion::Subtopics
            ) && window_start + index == focus_index;
            tabs[index] = RecommendationTabModel {
                label: recommendation_tab_label(subtopic),
                active: is_active,
                focused: is_focused,
                flash: is_focused
                    && store.ui.recommendations_focus_flash_ticks > 0
                    && store.ui.recommendations_focus_flash_ticks.is_multiple_of(2),
            };
        }
        index += 1;
    }

    RecommendationBarModel {
        tabs,
        visible_count,
        show_left_more: window_start > 0,
        show_right_more: window_start + visible_count < len,
    }
}

fn recommendation_tab_window_start(focus_index: usize, len: usize) -> usize {
    if len <= RECOMMENDATION_VISIBLE_TABS {
        return 0;
    }

    let preferred = focus_index.saturating_sub(1);
    preferred.min(len.saturating_sub(RECOMMENDATION_VISIBLE_TABS))
}

fn recommendation_tab_label(
    subtopic: RecommendationSubtopic,
) -> InlineText<RECOMMENDATION_TAB_LABEL_MAX_BYTES> {
    let mut label = InlineText::new();
    for ch in subtopic.label.as_str().chars() {
        if !label.try_push_char(ch.to_ascii_uppercase()) {
            break;
        }
    }
    if subtopic.is_recommended() {
        let _ = label.try_push_char('*');
    }
    label
}

fn recommendation_topic_meta(
    subtopic: RecommendationSubtopic,
) -> InlineText<CONTENT_META_MAX_BYTES> {
    let mut meta = InlineText::new();
    if !subtopic.parent_topic_label.is_empty() {
        meta.set_truncated(subtopic.parent_topic_label.as_str());
    } else {
        meta.set_truncated("FOR YOU");
    }
    meta
}

fn select_manifest_collection_rows(
    collection: &CollectionManifestState,
    reading_progress: &ReadingProgressState,
    kind: CollectionKind,
    selected_index: usize,
) -> [ContentRowModel; VISIBLE_LIST_ROWS] {
    let Some(selected) = collection.item_at(selected_index.min(collection.len().saturating_sub(1)))
    else {
        return empty_collection_rows(kind);
    };
    let selected_index = selected_index.min(collection.len().saturating_sub(1));
    let previous = selected_index
        .checked_sub(1)
        .and_then(|index| collection.item_at(index));
    let next = collection.item_at(selected_index.saturating_add(1));

    [
        previous
            .map(|item| content_row_from_manifest(item, reading_progress, kind, false))
            .unwrap_or_else(empty_content_row),
        content_row_from_manifest(selected, reading_progress, kind, true),
        next.map(|item| content_row_from_manifest(item, reading_progress, kind, false))
            .unwrap_or_else(empty_content_row),
    ]
}

fn content_row(meta: &str, title: &str, selected: bool) -> ContentRowModel {
    ContentRowModel {
        meta: InlineText::from_slice(meta),
        title: InlineText::from_slice(title),
        progress_badge: None,
        is_fetching: false,
        selected,
    }
}

fn empty_content_row() -> ContentRowModel {
    content_row("", "", false)
}

fn dashboard_item(collection: CollectionKind, selected: bool) -> DashboardItemModel {
    DashboardItemModel {
        label: collection.dashboard_label(),
        live_dot: collection.has_dashboard_live_dot(),
        selected,
    }
}

fn dashboard_empty_item() -> DashboardItemModel {
    DashboardItemModel {
        label: "",
        live_dot: false,
        selected: false,
    }
}

fn content_row_from_manifest(
    item: CollectionManifestItem,
    reading_progress: &ReadingProgressState,
    kind: CollectionKind,
    selected: bool,
) -> ContentRowModel {
    let is_fetching = matches!(item.package_state, PackageState::Fetching);
    ContentRowModel {
        meta: content_row_meta(kind, item),
        title: item.title,
        progress_badge: row_progress_badge(kind, item, reading_progress),
        is_fetching,
        selected,
    }
}

fn content_row_meta(
    kind: CollectionKind,
    item: CollectionManifestItem,
) -> InlineText<CONTENT_META_MAX_BYTES> {
    let mut meta = collection_row_base_meta(kind, item.meta);
    let Some(label) = package_state_hint(item.package_state) else {
        return meta;
    };

    if !meta.is_empty() {
        let _ = meta.try_push_str(" / ");
    }
    let _ = meta.try_push_str(label);
    meta
}

fn collection_row_base_meta(
    kind: CollectionKind,
    meta: InlineText<CONTENT_META_MAX_BYTES>,
) -> InlineText<CONTENT_META_MAX_BYTES> {
    let Some(suffix) = collection_meta_suffix(kind) else {
        return meta;
    };

    let mut stripped = InlineText::new();
    let base = meta.as_str().strip_suffix(suffix).unwrap_or(meta.as_str());
    stripped.set_truncated(base);
    stripped
}

const fn collection_meta_suffix(kind: CollectionKind) -> Option<&'static str> {
    match kind {
        CollectionKind::Saved => Some(" / SAVED"),
        CollectionKind::Inbox => Some(" / INBOX"),
        CollectionKind::Recommendations => Some(" / FOR YOU"),
    }
}

const fn package_state_hint(state: PackageState) -> Option<&'static str> {
    match state {
        PackageState::Fetching => Some("FETCHING"),
        PackageState::PendingRemote => Some("REMOTE"),
        PackageState::Failed => Some("FAILED"),
        PackageState::Missing | PackageState::Cached | PackageState::Stale => None,
    }
}

fn row_progress_badge(
    _kind: CollectionKind,
    item: CollectionManifestItem,
    reading_progress: &ReadingProgressState,
) -> Option<InlineText<8>> {
    // Progress is article-scoped across collections; any row with a matching
    // content id + revision should show the same badge unless it is still fetching.
    if matches!(item.package_state, PackageState::Fetching) {
        return None;
    }

    progress_badge_label(reading_progress.entry_for_item(item)?)
}

fn progress_badge_label(entry: ReadingProgressEntry) -> Option<InlineText<8>> {
    let percent = entry.completion_percent();
    if percent == 0 {
        return None;
    }

    let mut label = InlineText::new();
    push_decimal(&mut label, percent);
    let _ = label.try_push_char('%');
    Some(label)
}

fn push_decimal(target: &mut InlineText<8>, value: u8) {
    if value >= 100 {
        let _ = target.try_push_char('1');
        let _ = target.try_push_char('0');
        let _ = target.try_push_char('0');
        return;
    }

    if value >= 10 {
        let _ = target.try_push_char((b'0' + (value / 10)) as char);
    }
    let _ = target.try_push_char((b'0' + (value % 10)) as char);
}

fn empty_collection_rows(kind: CollectionKind) -> [ContentRowModel; VISIBLE_LIST_ROWS] {
    match kind {
        CollectionKind::Saved => [
            content_row("", "", false),
            content_row("MOTIF", "No saved items synced yet", true),
            content_row("PHONE / APP", "Save links, then refresh data", false),
        ],
        CollectionKind::Inbox => [
            content_row("", "", false),
            content_row("MOTIF / INBOX", "No inbox items synced yet", true),
            content_row("NETWORK / SYNC", "Refresh data once feeds arrive", false),
        ],
        CollectionKind::Recommendations => [
            content_row("", "", false),
            content_row("MOTIF / FOR YOU", "No recommendations synced yet", true),
            content_row("NETWORK / SYNC", "Refresh data after pairing", false),
        ],
    }
}

fn select_status(store: &Store) -> StatusClusterModel {
    StatusClusterModel {
        battery_percent: store.power.battery_percent,
        network: store.network.status,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{
        CollectionManifestItem, DetailLocator, ReadingProgressEntry, RecommendationSubtopic,
        RecommendationSubtopicsState, RemoteContentStatus,
    };
    use crate::formatter::{article_document_from_script, format_article_document};
    use crate::network::NetworkStatus;
    use crate::store::Store;
    use crate::sync::SyncStatus;

    fn make_recommendation_subtopic(
        slug: &str,
        label: &str,
        from_settings: bool,
        from_behavior: bool,
    ) -> RecommendationSubtopic {
        let mut topic = RecommendationSubtopic::empty();
        topic.slug.set_truncated(slug);
        topic.label.set_truncated(label);
        topic.parent_topic_label.set_truncated("Technology");
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
        let _ = subtopics.try_push(make_recommendation_subtopic("pocket", "Pocket", true, true));
        subtopics
    }

    fn make_recommendation_item(meta: &str, title: &str) -> CollectionManifestItem {
        let mut item = CollectionManifestItem::empty();
        item.remote_item_id.set_truncated(title);
        item.content_id.set_truncated(title);
        item.detail_locator = DetailLocator::Content;
        item.meta.set_truncated(meta);
        item.title.set_truncated(title);
        item.remote_status = RemoteContentStatus::Ready;
        item.package_state = PackageState::Cached;
        item
    }

    #[test]
    fn dashboard_defaults_to_saved_focus() {
        let store = Store::new();
        let model = select_dashboard(&store);

        assert_eq!(model.items[1].label, "SAVED");
        assert!(model.items[1].selected);
        assert_eq!(model.items[0].label, "INBOX");
        assert_eq!(model.items[2].label, "FOR YOU");
        assert_eq!(model.sync_indicator, None);
    }

    #[test]
    fn dashboard_first_focus_uses_empty_top_slot() {
        let mut store = Store::new();
        store.ui.dashboard_focus = DashboardFocus::Inbox;

        let model = select_dashboard(&store);

        assert_eq!(model.items[0].label, "");
        assert_eq!(model.items[1].label, "INBOX");
        assert_eq!(model.items[2].label, "SAVED");
    }

    #[test]
    fn dashboard_last_focus_uses_empty_bottom_slot() {
        let mut store = Store::new();
        store.ui.dashboard_focus = DashboardFocus::Recommendations;

        let model = select_dashboard(&store);

        assert_eq!(model.items[0].label, "SAVED");
        assert_eq!(model.items[1].label, "FOR YOU");
        assert_eq!(model.items[2].label, "");
    }

    #[test]
    fn dashboard_shows_sync_indicator_for_active_backend_sync() {
        let mut store = Store::new();
        store.backend_sync.status = SyncStatus::SyncingContent;
        store.backend_sync.spinner_phase = 2;

        let model = select_dashboard(&store);

        assert_eq!(
            model.sync_indicator,
            Some(SyncIndicatorModel {
                label: "syncing...",
                spinner_phase: 2,
            })
        );
    }

    #[test]
    fn recommendations_selector_builds_topic_bar_with_active_and_focused_tabs() {
        let mut store = Store::new();
        store
            .recommendations
            .set_subtopics(make_recommendation_subtopics());
        store
            .recommendations
            .set_active_topic(crate::text::InlineText::from_slice("e-ink"), false);
        store.ui.recommendations_region = crate::ui::RecommendationsRegion::Subtopics;
        store.ui.recommendations_subtopic_index = 1;

        let model = select_collection(&store, CollectionKind::Recommendations);
        let bar = model.recommendations_bar.expect("recommendation bar");

        assert_eq!(bar.visible_count, 3);
        assert_eq!(bar.tabs[0].label.as_str(), "E INK");
        assert!(bar.tabs[0].active);
        assert!(!bar.tabs[0].focused);
        assert!(!bar.tabs[0].flash);
        assert_eq!(bar.tabs[1].label.as_str(), "SMALL WEB*");
        assert!(!bar.tabs[1].active);
        assert!(bar.tabs[1].focused);
        assert!(!bar.tabs[1].flash);
        assert_eq!(bar.tabs[2].label.as_str(), "POCKET");
    }

    #[test]
    fn recommendations_selector_flashes_active_focused_tab_when_returning_to_subtopics() {
        let mut store = Store::new();
        store
            .recommendations
            .set_subtopics(make_recommendation_subtopics());
        store
            .recommendations
            .set_active_topic(crate::text::InlineText::from_slice("e-ink"), false);
        store.ui.recommendations_region = crate::ui::RecommendationsRegion::Subtopics;
        store.ui.recommendations_subtopic_index = 0;
        store.ui.recommendations_focus_flash_ticks = 8;

        let model = select_collection(&store, CollectionKind::Recommendations);
        let bar = model.recommendations_bar.expect("recommendation bar");

        assert!(bar.tabs[0].active);
        assert!(bar.tabs[0].focused);
        assert!(bar.tabs[0].flash);
    }

    #[test]
    fn recommendations_selector_strips_for_you_meta_suffix() {
        let mut store = Store::new();
        let mut collection = CollectionManifestState::empty();
        let _ = collection.try_push(make_recommendation_item(
            "NOTEBOOKCHECK / FOR YOU",
            "E Ink screens are finally weird enough to be useful",
        ));
        store
            .content_mut()
            .update_collection(CollectionKind::Recommendations, collection);

        let model = select_collection(&store, CollectionKind::Recommendations);

        assert_eq!(model.rows[1].meta.as_str(), "NOTEBOOKCHECK");
    }

    #[test]
    fn paragraph_navigation_uses_reader_progress() {
        let mut store = Store::new();
        store.reader.progress.paragraph_index = 7;

        let model = select_paragraph_navigation(&store);

        assert_eq!(model.current_index, 7);
        assert_eq!(model.total, 23);
        assert_eq!(model.tick_index, 2);
        assert_eq!(
            model.selected_excerpt.as_str(),
            "Analog objects still teach us what speed tends to erase."
        );
    }

    #[test]
    fn paragraph_navigation_uses_empty_edges_at_article_bounds() {
        let mut store = Store::new();

        store.reader.progress.paragraph_index = 1;
        let first = select_paragraph_navigation(&store);
        assert!(first.previous_top.is_empty());
        assert!(!first.previous_bottom.is_empty());

        store.reader.progress.paragraph_index = store.reader.progress.total_paragraphs;
        let last = select_paragraph_navigation(&store);
        assert!(last.previous_bottom.is_empty());
        assert!(last.final_excerpt.is_empty());
    }

    #[test]
    fn reader_selector_uses_live_rsvp_stage() {
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
            InlineText::from_slice(article.reader_title),
            alloc::boxed::Box::new(document),
            article.has_chat,
            store.settings.reading_speed_wpm,
        );
        store.ui.route = UiRoute::Reader;

        let model = select_reader(&store);

        assert_eq!(model.title.as_str(), "THE MACHINE SOUL");
        assert_eq!(model.wpm, 200);
        assert!(!model.right_word.is_empty());
        assert!(!model.preview.is_empty());
    }

    #[test]
    fn reader_selector_shows_quantized_live_ramp_wpm() {
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
            InlineText::from_slice(article.reader_title),
            alloc::boxed::Box::new(document),
            article.has_chat,
            store.settings.reading_speed_wpm,
        );
        store.ui.route = UiRoute::Reader;

        let model = select_reader(&store);

        assert_eq!(model.wpm, 200);
    }

    #[test]
    fn settings_selector_surfaces_network_status_value() {
        let mut store = Store::new();
        store.network.status = NetworkStatus::ProbeFailed;

        let model = select_settings(&store);

        assert_eq!(model.rows[4].value, Some("Probe Failed"));
        assert!(!model.rows[4].show_arrow);
    }

    #[test]
    fn saved_collection_selector_uses_live_saved_manifest() {
        let mut store = Store::new();
        let mut item = CollectionManifestItem::empty();
        item.meta.set_truncated("EXAMPLE / SAVED");
        item.title.set_truncated("Example saved title");
        let _ = store
            .content_mut()
            .collection_state_mut(CollectionKind::Saved)
            .try_push(item);
        store.ui.saved_index = 0;

        let model = select_collection(&store, CollectionKind::Saved);

        assert_eq!(model.rows[1].meta.as_str(), "EXAMPLE");
        assert_eq!(model.rows[1].title.as_str(), "Example saved title");
        assert!(!model.rows[1].is_fetching);
    }

    #[test]
    fn inbox_collection_selector_uses_live_manifest_without_inbox_suffix() {
        let mut store = Store::new();
        let mut item = CollectionManifestItem::empty();
        item.meta.set_truncated("EXAMPLE / INBOX");
        item.title.set_truncated("Example inbox title");
        let _ = store
            .content_mut()
            .collection_state_mut(CollectionKind::Inbox)
            .try_push(item);
        store.ui.inbox_index = 0;

        let model = select_collection(&store, CollectionKind::Inbox);

        assert_eq!(model.rows[1].meta.as_str(), "EXAMPLE");
        assert_eq!(model.rows[1].title.as_str(), "Example inbox title");
        assert!(!model.rows[1].is_fetching);
    }

    #[test]
    fn fetching_saved_collection_selector_shows_fetching_label_without_spinner() {
        let mut store = Store::new();
        let mut item = CollectionManifestItem::empty();
        item.meta.set_truncated("EXAMPLE / SAVED");
        item.title.set_truncated("Example saved title");
        item.package_state = crate::content::PackageState::Fetching;
        let _ = store
            .content_mut()
            .collection_state_mut(CollectionKind::Saved)
            .try_push(item);
        store.ui.saved_index = 0;

        let model = select_collection(&store, CollectionKind::Saved);

        assert_eq!(model.rows[1].meta.as_str(), "EXAMPLE / FETCHING");
        assert!(model.rows[1].is_fetching);
    }

    #[test]
    fn single_saved_item_does_not_repeat_in_adjacent_rows() {
        let mut store = Store::new();
        let mut item = CollectionManifestItem::empty();
        item.meta.set_truncated("EXAMPLE / SAVED");
        item.title.set_truncated("Example saved title");
        let _ = store
            .content_mut()
            .collection_state_mut(CollectionKind::Saved)
            .try_push(item);

        let model = select_collection(&store, CollectionKind::Saved);

        assert!(model.rows[0].meta.is_empty());
        assert!(model.rows[0].title.is_empty());
        assert_eq!(model.rows[1].meta.as_str(), "EXAMPLE");
        assert_eq!(model.rows[1].title.as_str(), "Example saved title");
        assert!(!model.rows[1].is_fetching);
        assert!(model.rows[2].meta.is_empty());
        assert!(model.rows[2].title.is_empty());
    }

    #[test]
    fn saved_collection_selector_stops_at_last_item_and_uses_empty_bottom_row() {
        let mut store = Store::new();
        let mut first = CollectionManifestItem::empty();
        first.meta.set_truncated("EXAMPLE / SAVED");
        first.title.set_truncated("First saved title");
        let mut second = CollectionManifestItem::empty();
        second.meta.set_truncated("EXAMPLE / SAVED");
        second.title.set_truncated("Second saved title");
        let _ = store
            .content_mut()
            .collection_state_mut(CollectionKind::Saved)
            .try_push(first);
        let _ = store
            .content_mut()
            .collection_state_mut(CollectionKind::Saved)
            .try_push(second);
        store.ui.saved_index = 1;

        let model = select_collection(&store, CollectionKind::Saved);

        assert_eq!(model.rows[0].title.as_str(), "First saved title");
        assert_eq!(model.rows[1].title.as_str(), "Second saved title");
        assert!(model.rows[2].title.is_empty());
    }

    #[test]
    fn saved_collection_selector_keeps_first_row_empty_instead_of_wrapping() {
        let mut store = Store::new();
        let mut first = CollectionManifestItem::empty();
        first.meta.set_truncated("EXAMPLE / SAVED");
        first.title.set_truncated("First saved title");
        let mut second = CollectionManifestItem::empty();
        second.meta.set_truncated("EXAMPLE / SAVED");
        second.title.set_truncated("Second saved title");
        let _ = store
            .content_mut()
            .collection_state_mut(CollectionKind::Saved)
            .try_push(first);
        let _ = store
            .content_mut()
            .collection_state_mut(CollectionKind::Saved)
            .try_push(second);
        store.ui.saved_index = 0;

        let model = select_collection(&store, CollectionKind::Saved);

        assert!(model.rows[0].title.is_empty());
        assert_eq!(model.rows[1].title.as_str(), "First saved title");
        assert_eq!(model.rows[2].title.as_str(), "Second saved title");
    }

    #[test]
    fn empty_saved_collection_selector_shows_empty_state() {
        let store = Store::from_bootstrap(crate::runtime::BootstrapSnapshot::new(
            crate::device::DeviceState::new(),
            0,
            None,
            None,
            None,
            None,
            crate::storage::StorageHealth::new(),
            crate::network::NetworkState::disabled(),
        ));

        let model = select_collection(&store, CollectionKind::Saved);

        assert_eq!(model.rows[1].meta.as_str(), "MOTIF");
        assert_eq!(model.rows[1].title.as_str(), "No saved items synced yet");
    }

    #[test]
    fn saved_collection_selector_shows_progress_badge_for_started_article() {
        let mut store = Store::new();
        let mut item = CollectionManifestItem::empty();
        item.content_id.set_truncated("content-1");
        item.remote_revision = 7;
        item.meta.set_truncated("EXAMPLE / SAVED");
        item.title.set_truncated("Example saved title");
        let _ = store
            .content_mut()
            .collection_state_mut(CollectionKind::Saved)
            .try_push(item);
        let _ = store
            .reading_progress
            .record_progress(ReadingProgressEntry {
                content_id: item.content_id,
                remote_revision: 7,
                paragraph_index: 3,
                total_paragraphs: 12,
            });

        let model = select_collection(&store, CollectionKind::Saved);

        assert_eq!(model.rows[1].progress_badge.unwrap().as_str(), "25%");
    }

    #[test]
    fn inbox_collection_selector_shows_progress_badge_for_started_article() {
        let mut store = Store::new();
        let mut item = CollectionManifestItem::empty();
        item.content_id.set_truncated("content-1");
        item.remote_revision = 7;
        item.meta.set_truncated("EXAMPLE / INBOX");
        item.title.set_truncated("Example inbox title");
        let _ = store
            .content_mut()
            .collection_state_mut(CollectionKind::Inbox)
            .try_push(item);
        let _ = store
            .reading_progress
            .record_progress(ReadingProgressEntry {
                content_id: item.content_id,
                remote_revision: 7,
                paragraph_index: 3,
                total_paragraphs: 12,
            });

        let model = select_collection(&store, CollectionKind::Inbox);

        assert_eq!(model.rows[1].progress_badge.unwrap().as_str(), "25%");
    }

    #[test]
    fn recommendations_collection_selector_shows_progress_badge_for_started_article() {
        let mut store = Store::new();
        let mut item = CollectionManifestItem::empty();
        item.content_id.set_truncated("content-1");
        item.remote_item_id.set_truncated("content-1");
        item.remote_revision = 7;
        item.detail_locator = DetailLocator::Content;
        item.meta.set_truncated("EXAMPLE / FOR YOU");
        item.title.set_truncated("Example recommendation title");
        let _ = store
            .content_mut()
            .collection_state_mut(CollectionKind::Recommendations)
            .try_push(item);
        let _ = store
            .reading_progress
            .record_progress(ReadingProgressEntry {
                content_id: item.content_id,
                remote_revision: 7,
                paragraph_index: 3,
                total_paragraphs: 12,
            });

        let model = select_collection(&store, CollectionKind::Recommendations);

        assert_eq!(model.rows[1].progress_badge.unwrap().as_str(), "25%");
    }

    #[test]
    fn saved_collection_selector_hides_progress_badge_for_fetching_row() {
        let mut store = Store::new();
        let mut item = CollectionManifestItem::empty();
        item.content_id.set_truncated("content-1");
        item.remote_revision = 7;
        item.meta.set_truncated("EXAMPLE / SAVED");
        item.title.set_truncated("Example saved title");
        item.package_state = crate::content::PackageState::Fetching;
        let _ = store
            .content_mut()
            .collection_state_mut(CollectionKind::Saved)
            .try_push(item);
        let _ = store
            .reading_progress
            .record_progress(ReadingProgressEntry {
                content_id: item.content_id,
                remote_revision: 7,
                paragraph_index: 3,
                total_paragraphs: 12,
            });

        let model = select_collection(&store, CollectionKind::Saved);

        assert!(model.rows[1].is_fetching);
        assert_eq!(model.rows[1].meta.as_str(), "EXAMPLE / FETCHING");
        assert_eq!(model.rows[1].progress_badge, None);
    }

    #[test]
    fn inbox_collection_selector_hides_progress_badge_for_fetching_row() {
        let mut store = Store::new();
        let mut item = CollectionManifestItem::empty();
        item.content_id.set_truncated("content-1");
        item.remote_revision = 7;
        item.meta.set_truncated("EXAMPLE / INBOX");
        item.title.set_truncated("Example inbox title");
        item.package_state = crate::content::PackageState::Fetching;
        let _ = store
            .content_mut()
            .collection_state_mut(CollectionKind::Inbox)
            .try_push(item);
        let _ = store
            .reading_progress
            .record_progress(ReadingProgressEntry {
                content_id: item.content_id,
                remote_revision: 7,
                paragraph_index: 3,
                total_paragraphs: 12,
            });

        let model = select_collection(&store, CollectionKind::Inbox);

        assert!(model.rows[1].is_fetching);
        assert_eq!(model.rows[1].meta.as_str(), "EXAMPLE / FETCHING");
        assert_eq!(model.rows[1].progress_badge, None);
    }

    #[test]
    fn recommendations_collection_selector_hides_progress_badge_for_fetching_row() {
        let mut store = Store::new();
        let mut item = CollectionManifestItem::empty();
        item.content_id.set_truncated("content-1");
        item.remote_item_id.set_truncated("content-1");
        item.remote_revision = 7;
        item.detail_locator = DetailLocator::Content;
        item.meta.set_truncated("EXAMPLE / FOR YOU");
        item.title.set_truncated("Example recommendation title");
        item.package_state = crate::content::PackageState::Fetching;
        let _ = store
            .content_mut()
            .collection_state_mut(CollectionKind::Recommendations)
            .try_push(item);
        let _ = store
            .reading_progress
            .record_progress(ReadingProgressEntry {
                content_id: item.content_id,
                remote_revision: 7,
                paragraph_index: 3,
                total_paragraphs: 12,
            });

        let model = select_collection(&store, CollectionKind::Recommendations);

        assert!(model.rows[1].is_fetching);
        assert_eq!(model.rows[1].meta.as_str(), "EXAMPLE / FETCHING");
        assert_eq!(model.rows[1].progress_badge, None);
    }

    #[test]
    fn collection_selector_hides_progress_badge_for_revision_mismatch_across_collections() {
        let mut store = Store::new();
        let mut saved = CollectionManifestItem::empty();
        saved.content_id.set_truncated("content-1");
        saved.remote_revision = 8;
        saved.meta.set_truncated("EXAMPLE / SAVED");
        saved.title.set_truncated("Example saved title");
        let mut inbox = CollectionManifestItem::empty();
        inbox.content_id = saved.content_id;
        inbox.remote_revision = 8;
        inbox.meta.set_truncated("EXAMPLE / INBOX");
        inbox.title.set_truncated("Example inbox title");
        let mut recommendation = CollectionManifestItem::empty();
        recommendation.content_id = saved.content_id;
        recommendation.remote_item_id = saved.content_id;
        recommendation.remote_revision = 8;
        recommendation.detail_locator = DetailLocator::Content;
        recommendation.meta.set_truncated("EXAMPLE / FOR YOU");
        recommendation
            .title
            .set_truncated("Example recommendation title");
        let _ = store
            .content_mut()
            .collection_state_mut(CollectionKind::Saved)
            .try_push(saved);
        let _ = store
            .content_mut()
            .collection_state_mut(CollectionKind::Inbox)
            .try_push(inbox);
        let _ = store
            .content_mut()
            .collection_state_mut(CollectionKind::Recommendations)
            .try_push(recommendation);
        let _ = store
            .reading_progress
            .record_progress(ReadingProgressEntry {
                content_id: saved.content_id,
                remote_revision: 7,
                paragraph_index: 3,
                total_paragraphs: 12,
            });

        assert_eq!(
            select_collection(&store, CollectionKind::Saved).rows[1].progress_badge,
            None
        );
        assert_eq!(
            select_collection(&store, CollectionKind::Inbox).rows[1].progress_badge,
            None
        );
        assert_eq!(
            select_collection(&store, CollectionKind::Recommendations).rows[1].progress_badge,
            None
        );
    }

    #[test]
    fn matching_progress_entry_shows_badge_across_all_collections_for_same_article() {
        let mut store = Store::new();
        let mut saved = CollectionManifestItem::empty();
        saved.content_id.set_truncated("content-1");
        saved.remote_revision = 7;
        saved.meta.set_truncated("EXAMPLE / SAVED");
        saved.title.set_truncated("Example saved title");
        let mut inbox = CollectionManifestItem::empty();
        inbox.content_id = saved.content_id;
        inbox.remote_revision = 7;
        inbox.meta.set_truncated("EXAMPLE / INBOX");
        inbox.title.set_truncated("Example inbox title");
        let mut recommendation = CollectionManifestItem::empty();
        recommendation.content_id = saved.content_id;
        recommendation.remote_item_id = saved.content_id;
        recommendation.remote_revision = 7;
        recommendation.detail_locator = DetailLocator::Content;
        recommendation.meta.set_truncated("EXAMPLE / FOR YOU");
        recommendation
            .title
            .set_truncated("Example recommendation title");
        let _ = store
            .content_mut()
            .collection_state_mut(CollectionKind::Saved)
            .try_push(saved);
        let _ = store
            .content_mut()
            .collection_state_mut(CollectionKind::Inbox)
            .try_push(inbox);
        let _ = store
            .content_mut()
            .collection_state_mut(CollectionKind::Recommendations)
            .try_push(recommendation);
        let _ = store
            .reading_progress
            .record_progress(ReadingProgressEntry {
                content_id: saved.content_id,
                remote_revision: 7,
                paragraph_index: 3,
                total_paragraphs: 12,
            });

        assert_eq!(
            select_collection(&store, CollectionKind::Saved).rows[1]
                .progress_badge
                .unwrap()
                .as_str(),
            "25%"
        );
        assert_eq!(
            select_collection(&store, CollectionKind::Inbox).rows[1]
                .progress_badge
                .unwrap()
                .as_str(),
            "25%"
        );
        assert_eq!(
            select_collection(&store, CollectionKind::Recommendations).rows[1]
                .progress_badge
                .unwrap()
                .as_str(),
            "25%"
        );
    }
}
