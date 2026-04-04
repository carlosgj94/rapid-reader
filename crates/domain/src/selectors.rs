use crate::{
    content::{
        CONTENT_META_MAX_BYTES, CONTENT_TITLE_MAX_BYTES, CollectionKind, CollectionManifestItem,
        CollectionManifestState, ContentState, PackageState,
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
    ui::{DashboardFocus, SettingsMode, TopicRegion, UiRoute},
};

pub const VISIBLE_LIST_ROWS: usize = 3;
pub const SETTINGS_ROW_COUNT: usize = 6;

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
    pub selected: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ContentListScreenModel {
    pub appearance: AppearanceMode,
    pub status: StatusClusterModel,
    pub rail_label: &'static str,
    pub rows: [ContentRowModel; VISIBLE_LIST_ROWS],
    pub selected_collection: CollectionKind,
    pub selected_index: usize,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PauseActionModel {
    pub label: &'static str,
    pub action: &'static str,
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
    pub pause_actions: Option<[PauseActionModel; 3]>,
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
    let previous = focused.previous().as_collection();
    let next = focused.next().as_collection();
    let current = focused.as_collection();

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
            DashboardItemModel {
                label: previous.dashboard_label(),
                live_dot: previous.has_dashboard_live_dot(),
                selected: false,
            },
            DashboardItemModel {
                label: current.dashboard_label(),
                live_dot: current.has_dashboard_live_dot(),
                selected: true,
            },
            DashboardItemModel {
                label: next.dashboard_label(),
                live_dot: next.has_dashboard_live_dot(),
                selected: false,
            },
        ],
        focused,
    }
}

pub fn select_collection(store: &Store, kind: CollectionKind) -> ContentListScreenModel {
    let selected_index = store.ui.collection_index(kind);
    let rows = select_collection_rows(store.content(), kind, selected_index);

    ContentListScreenModel {
        appearance: store.settings.appearance,
        status: select_status(store),
        rail_label: kind.rail_label(),
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
        wpm: store.settings.reading_speed_wpm,
        left_word: stage_token.left,
        right_word: stage_token.right,
        preview,
        font: stage_token.font,
        progress_width: store.reader.progress_width_px(),
        show_chat_badge: matches!(store.reader.mode, ReaderMode::Chat),
        pause_actions: matches!(store.reader.mode, ReaderMode::Paused).then_some([
            PauseActionModel {
                label: "LONG PRESS ->",
                action: "GO BACK",
            },
            PauseActionModel {
                label: "SHORT PRESS ->",
                action: "PARAGRAPH VIEW",
            },
            PauseActionModel {
                label: "ROTATE ->",
                action: "ADJUST RSVP SPEED",
            },
        ]),
    }
}

pub fn select_paragraph_navigation(store: &Store) -> ParagraphNavigationModel {
    let current_index = store.reader.progress.paragraph_index as usize;
    let total = store.reader.progress.total_paragraphs;
    let current_zero_based = current_index.saturating_sub(1);
    let previous_top = store
        .reader
        .preview_for_paragraph((current_zero_based.saturating_sub(1) + 1) as u16);
    let selected_excerpt = store
        .reader
        .preview_for_paragraph((current_zero_based + 1) as u16);
    let previous_bottom = store.reader.preview_for_paragraph(
        ((current_zero_based + 1).min(total.saturating_sub(1) as usize) + 1) as u16,
    );
    let final_excerpt = store.reader.preview_for_paragraph(
        ((current_zero_based + 2).min(total.saturating_sub(1) as usize) + 1) as u16,
    );
    let tick_index = current_zero_based.min(6) as u8;

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
    kind: CollectionKind,
    selected_index: usize,
) -> [ContentRowModel; VISIBLE_LIST_ROWS] {
    select_manifest_collection_rows(content.collection_state(kind), kind, selected_index)
}

fn select_manifest_collection_rows(
    collection: &CollectionManifestState,
    kind: CollectionKind,
    selected_index: usize,
) -> [ContentRowModel; VISIBLE_LIST_ROWS] {
    let Some(selected) = collection.item_at(selected_index) else {
        return empty_collection_rows(kind);
    };
    if collection.len() == 1 {
        return [
            content_row("", "", false),
            content_row_from_manifest(selected, true),
            content_row("", "", false),
        ];
    }
    let previous = collection
        .item_at((selected_index + collection.len() - 1) % collection.len())
        .unwrap_or(selected);
    let next = collection
        .item_at((selected_index + 1) % collection.len())
        .unwrap_or(selected);

    [
        content_row_from_manifest(previous, false),
        content_row_from_manifest(selected, true),
        content_row_from_manifest(next, false),
    ]
}

fn content_row(meta: &str, title: &str, selected: bool) -> ContentRowModel {
    ContentRowModel {
        meta: InlineText::from_slice(meta),
        title: InlineText::from_slice(title),
        selected,
    }
}

fn content_row_from_manifest(item: CollectionManifestItem, selected: bool) -> ContentRowModel {
    ContentRowModel {
        meta: content_row_meta(item),
        title: item.title,
        selected,
    }
}

fn content_row_meta(item: CollectionManifestItem) -> InlineText<CONTENT_META_MAX_BYTES> {
    let mut meta = item.meta;
    let Some(label) = package_state_hint(item.package_state) else {
        return meta;
    };

    let _ = meta.try_push_str(" / ");
    let _ = meta.try_push_str(label);
    meta
}

const fn package_state_hint(state: PackageState) -> Option<&'static str> {
    match state {
        PackageState::Fetching => Some("FETCHING"),
        PackageState::PendingRemote => Some("REMOTE"),
        PackageState::Failed => Some("FAILED"),
        PackageState::Missing | PackageState::Cached | PackageState::Stale => None,
    }
}

fn empty_collection_rows(kind: CollectionKind) -> [ContentRowModel; VISIBLE_LIST_ROWS] {
    match kind {
        CollectionKind::Saved => [
            content_row("", "", false),
            content_row("MOTIF / SAVED", "No saved items synced yet", true),
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
    use crate::content::CollectionManifestItem;
    use crate::formatter::{article_document_from_script, format_article_document};
    use crate::network::NetworkStatus;
    use crate::store::Store;
    use crate::sync::SyncStatus;

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
    fn paragraph_navigation_uses_reader_progress() {
        let mut store = Store::new();
        store.reader.progress.paragraph_index = 7;

        let model = select_paragraph_navigation(&store);

        assert_eq!(model.current_index, 7);
        assert_eq!(model.total, 23);
        assert_eq!(
            model.selected_excerpt.as_str(),
            "Analog objects still teach us what speed tends to erase."
        );
    }

    #[test]
    fn reader_selector_uses_live_rsvp_stage() {
        let mut store = Store::new();
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
        );
        store.ui.route = UiRoute::Reader;

        let model = select_reader(&store);

        assert_eq!(model.title.as_str(), "THE MACHINE SOUL");
        assert!(!model.right_word.is_empty());
        assert!(!model.preview.is_empty());
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

        assert_eq!(model.rows[1].meta.as_str(), "EXAMPLE / SAVED");
        assert_eq!(model.rows[1].title.as_str(), "Example saved title");
    }

    #[test]
    fn fetching_saved_collection_selector_surfaces_fetching_state() {
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

        assert_eq!(model.rows[1].meta.as_str(), "EXAMPLE / SAVED / FETCHING");
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
        assert_eq!(model.rows[1].meta.as_str(), "EXAMPLE / SAVED");
        assert_eq!(model.rows[1].title.as_str(), "Example saved title");
        assert!(model.rows[2].meta.is_empty());
        assert!(model.rows[2].title.is_empty());
    }

    #[test]
    fn empty_saved_collection_selector_shows_empty_state() {
        let store = Store::from_bootstrap(crate::runtime::BootstrapSnapshot::new(
            crate::device::DeviceState::new(),
            0,
            None,
            None,
            crate::storage::StorageHealth::new(),
            crate::network::NetworkState::disabled(),
        ));

        let model = select_collection(&store, CollectionKind::Saved);

        assert_eq!(model.rows[1].meta.as_str(), "MOTIF / SAVED");
        assert_eq!(model.rows[1].title.as_str(), "No saved items synced yet");
    }
}
