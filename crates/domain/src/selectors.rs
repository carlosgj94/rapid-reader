use crate::{
    content::{CollectionKind, ContentState, script_paragraph},
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
    pub rail_label: &'static str,
    pub items: [DashboardItemModel; VISIBLE_LIST_ROWS],
    pub focused: DashboardFocus,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ContentRowModel {
    pub meta: &'static str,
    pub title: &'static str,
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
    pub title: &'static str,
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
    pub title: &'static str,
    pub current_index: u8,
    pub total: u8,
    pub previous_top: &'static str,
    pub selected_excerpt: &'static str,
    pub previous_bottom: &'static str,
    pub final_excerpt: &'static str,
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
    let rows = select_collection_rows(&store.content, kind, selected_index);

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
    let article = store
        .content
        .article_by_id(store.reader.active_collection, store.reader.active_article);
    let current_unit = store.reader.current_unit();
    let stage_token = current_unit.stage_token();
    let preview = store
        .reader
        .document
        .preview_for_paragraph(store.reader.progress.paragraph_index);

    ReaderScreenModel {
        appearance: store.settings.appearance,
        title: article.reader_title,
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
    let article = store.content.article_at(
        store.reader.active_collection,
        store.ui.collection_index(store.reader.active_collection),
    );
    let current_index = store.reader.progress.paragraph_index as usize;
    let total = store.reader.progress.total_paragraphs;
    let current_zero_based = current_index.saturating_sub(1);
    let previous_top = script_paragraph(article.script, current_zero_based.saturating_sub(1));
    let selected_excerpt = script_paragraph(article.script, current_zero_based);
    let previous_bottom = script_paragraph(
        article.script,
        (current_zero_based + 1).min((total - 1) as usize),
    );
    let final_excerpt = script_paragraph(
        article.script,
        (current_zero_based + 2).min((total - 1) as usize),
    );
    let tick_index = current_zero_based.min(6) as u8;

    ParagraphNavigationModel {
        appearance: store.settings.appearance,
        title: article.reader_title,
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
            value: None,
            selected: matches!(
                store.ui.settings_row,
                crate::ui::SettingsRow::NetworkConnection
            ),
            show_arrow: true,
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
    let len = content.collection(kind).len();
    let previous = content.article_at(kind, (selected_index + len - 1) % len);
    let selected = content.article_at(kind, selected_index % len);
    let next = content.article_at(kind, (selected_index + 1) % len);

    [
        ContentRowModel {
            meta: previous.meta,
            title: previous.title,
            selected: false,
        },
        ContentRowModel {
            meta: selected.meta,
            title: selected.title,
            selected: true,
        },
        ContentRowModel {
            meta: next.meta,
            title: next.title,
            selected: false,
        },
    ]
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
    use crate::store::Store;

    #[test]
    fn dashboard_defaults_to_saved_focus() {
        let store = Store::new();
        let model = select_dashboard(&store);

        assert_eq!(model.items[1].label, "SAVED");
        assert!(model.items[1].selected);
        assert_eq!(model.items[0].label, "INBOX");
        assert_eq!(model.items[2].label, "FOR YOU");
    }

    #[test]
    fn paragraph_navigation_uses_reader_progress() {
        let mut store = Store::new();
        store.reader.progress.paragraph_index = 7;

        let model = select_paragraph_navigation(&store);

        assert_eq!(model.current_index, 7);
        assert_eq!(model.total, 23);
        assert_eq!(
            model.selected_excerpt,
            "Analog objects still teach us what speed tends to erase."
        );
    }

    #[test]
    fn reader_selector_uses_live_rsvp_stage() {
        let mut store = Store::new();
        store
            .dispatch(crate::runtime::Command::Ui(
                crate::runtime::UiCommand::Confirm,
            ))
            .unwrap();
        store
            .dispatch(crate::runtime::Command::Ui(
                crate::runtime::UiCommand::Confirm,
            ))
            .unwrap();

        let model = select_reader(&store);

        assert_eq!(model.title, "THE MACHINE SOUL");
        assert!(!model.right_word.is_empty());
        assert!(!model.preview.is_empty());
    }
}
