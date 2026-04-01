use crate::screens::Screen;
use domain::{
    formatter::{MAX_PARAGRAPH_PREVIEW_BYTES, MAX_STAGE_SEGMENT_BYTES, StageFont},
    selectors::{
        ActiveScreenModel, ContentListScreenModel, DashboardScreenModel, ParagraphNavigationModel,
        ReaderScreenModel, SettingsScreenModel,
    },
    settings::AppearanceMode,
    ui::{SettingsMode, TopicRegion},
};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub struct ComponentId(pub u16);

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct StatusCluster {
    pub battery_percent: u8,
    pub wifi_online: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct VerticalRail {
    pub text: &'static str,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct DashboardItem {
    pub label: &'static str,
    pub live_dot: bool,
    pub selected: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct SelectionBand {
    pub y: i32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct DashboardShell {
    pub appearance: AppearanceMode,
    pub status: StatusCluster,
    pub rail: VerticalRail,
    pub items: [DashboardItem; 3],
    pub band: SelectionBand,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ContentRow {
    pub meta: &'static str,
    pub title: &'static str,
    pub selected: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct HelpHint {
    pub text: &'static str,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ContentListShell {
    pub appearance: AppearanceMode,
    pub status: StatusCluster,
    pub rail: VerticalRail,
    pub rows: [ContentRow; 3],
    pub band: SelectionBand,
    pub help: HelpHint,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ModeBadge {
    pub label: &'static str,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PauseModalRow {
    pub label: &'static str,
    pub action: &'static str,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PauseModal {
    pub title: &'static str,
    pub rows: [PauseModalRow; 3],
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct RsvpStage {
    pub title: &'static str,
    pub wpm: u16,
    pub left_word: domain::text::InlineText<MAX_STAGE_SEGMENT_BYTES>,
    pub right_word: domain::text::InlineText<MAX_STAGE_SEGMENT_BYTES>,
    pub preview: domain::text::InlineText<MAX_PARAGRAPH_PREVIEW_BYTES>,
    pub font: StageFont,
    pub progress_width: u16,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ReaderShell {
    pub appearance: AppearanceMode,
    pub stage: RsvpStage,
    pub badge: Option<ModeBadge>,
    pub pause_modal: Option<PauseModal>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ParagraphMapRail {
    pub selected_index: u8,
    pub total_ticks: u8,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ParagraphNavigationShell {
    pub appearance: AppearanceMode,
    pub title: &'static str,
    pub counter: &'static str,
    pub previous_top: &'static str,
    pub selected_label: &'static str,
    pub selected_excerpt: &'static str,
    pub previous_bottom: &'static str,
    pub final_excerpt: &'static str,
    pub rail: ParagraphMapRail,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct SettingsRow {
    pub label: &'static str,
    pub value: Option<&'static str>,
    pub selected: bool,
    pub show_arrow: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct TopicCategory {
    pub label: &'static str,
    pub selected: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct TopicChip {
    pub label: &'static str,
    pub selected: bool,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct TopicPreferenceGrid {
    pub title: &'static str,
    pub categories: [TopicCategory; 4],
    pub chips: [TopicChip; 7],
    pub focus_region: TopicRegion,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct SettingsShell {
    pub appearance: AppearanceMode,
    pub title: &'static str,
    pub mode: SettingsMode,
    pub rows: [SettingsRow; 6],
    pub refresh_title: Option<&'static str>,
    pub refresh_body: Option<&'static str>,
    pub topic_preferences: Option<TopicPreferenceGrid>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PreparedScreen {
    Dashboard(DashboardShell),
    Collection(ContentListShell),
    Reader(ReaderShell),
    ParagraphNavigation(ParagraphNavigationShell),
    Settings(SettingsShell),
}

pub fn compose(model: ActiveScreenModel) -> (Screen, PreparedScreen) {
    match model {
        ActiveScreenModel::Dashboard(model) => (
            Screen::Dashboard,
            PreparedScreen::Dashboard(compose_dashboard(model)),
        ),
        ActiveScreenModel::Collection(model) => (
            match model.selected_collection {
                domain::content::CollectionKind::Saved => Screen::Saved,
                domain::content::CollectionKind::Inbox => Screen::Inbox,
                domain::content::CollectionKind::Recommendations => Screen::Recommendations,
            },
            PreparedScreen::Collection(compose_collection(model)),
        ),
        ActiveScreenModel::Reader(model) => (
            Screen::Reader,
            PreparedScreen::Reader(compose_reader(model)),
        ),
        ActiveScreenModel::ParagraphNavigation(model) => (
            Screen::ParagraphNavigation,
            PreparedScreen::ParagraphNavigation(compose_paragraph_navigation(model)),
        ),
        ActiveScreenModel::Settings(model) => (
            Screen::Settings,
            PreparedScreen::Settings(compose_settings(model)),
        ),
    }
}

fn compose_dashboard(model: DashboardScreenModel) -> DashboardShell {
    DashboardShell {
        appearance: model.appearance,
        status: StatusCluster {
            battery_percent: model.status.battery_percent,
            wifi_online: model.status.network == domain::network::NetworkStatus::Online,
        },
        rail: VerticalRail {
            text: model.rail_label,
        },
        items: [
            DashboardItem {
                label: model.items[0].label,
                live_dot: model.items[0].live_dot,
                selected: false,
            },
            DashboardItem {
                label: model.items[1].label,
                live_dot: model.items[1].live_dot,
                selected: true,
            },
            DashboardItem {
                label: model.items[2].label,
                live_dot: model.items[2].live_dot,
                selected: false,
            },
        ],
        band: SelectionBand { y: 82, height: 60 },
    }
}

fn compose_collection(model: ContentListScreenModel) -> ContentListShell {
    ContentListShell {
        appearance: model.appearance,
        status: StatusCluster {
            battery_percent: model.status.battery_percent,
            wifi_online: model.status.network == domain::network::NetworkStatus::Online,
        },
        rail: VerticalRail {
            text: model.rail_label,
        },
        rows: [
            ContentRow {
                meta: model.rows[0].meta,
                title: model.rows[0].title,
                selected: false,
            },
            ContentRow {
                meta: model.rows[1].meta,
                title: model.rows[1].title,
                selected: true,
            },
            ContentRow {
                meta: model.rows[2].meta,
                title: model.rows[2].title,
                selected: false,
            },
        ],
        band: SelectionBand { y: 98, height: 68 },
        help: HelpHint {
            text: "long press_",
        },
    }
}

fn compose_reader(model: ReaderScreenModel) -> ReaderShell {
    ReaderShell {
        appearance: model.appearance,
        stage: RsvpStage {
            title: model.title,
            wpm: model.wpm,
            left_word: model.left_word,
            right_word: model.right_word,
            preview: model.preview,
            font: model.font,
            progress_width: model.progress_width,
        },
        badge: model.show_chat_badge.then_some(ModeBadge { label: "CHAT" }),
        pause_modal: model.pause_actions.map(|actions| PauseModal {
            title: "PAUSED",
            rows: [
                PauseModalRow {
                    label: actions[0].label,
                    action: actions[0].action,
                },
                PauseModalRow {
                    label: actions[1].label,
                    action: actions[1].action,
                },
                PauseModalRow {
                    label: actions[2].label,
                    action: actions[2].action,
                },
            ],
        }),
    }
}

fn compose_paragraph_navigation(model: ParagraphNavigationModel) -> ParagraphNavigationShell {
    ParagraphNavigationShell {
        appearance: model.appearance,
        title: model.title,
        counter: counter_label(model.current_index, model.total),
        previous_top: model.previous_top,
        selected_label: paragraph_label(model.current_index),
        selected_excerpt: model.selected_excerpt,
        previous_bottom: model.previous_bottom,
        final_excerpt: model.final_excerpt,
        rail: ParagraphMapRail {
            selected_index: model.tick_index,
            total_ticks: 7,
        },
    }
}

fn compose_settings(model: SettingsScreenModel) -> SettingsShell {
    let topic_preferences = model.topic_preferences.map(|topic| TopicPreferenceGrid {
        title: topic.title,
        categories: [
            TopicCategory {
                label: topic.categories[0].label,
                selected: topic.categories[0].selected,
            },
            TopicCategory {
                label: topic.categories[1].label,
                selected: topic.categories[1].selected,
            },
            TopicCategory {
                label: topic.categories[2].label,
                selected: topic.categories[2].selected,
            },
            TopicCategory {
                label: topic.categories[3].label,
                selected: topic.categories[3].selected,
            },
        ],
        chips: [
            TopicChip {
                label: topic.chips[0].label,
                selected: topic.chips[0].selected,
                enabled: topic.chips[0].enabled,
            },
            TopicChip {
                label: topic.chips[1].label,
                selected: topic.chips[1].selected,
                enabled: topic.chips[1].enabled,
            },
            TopicChip {
                label: topic.chips[2].label,
                selected: topic.chips[2].selected,
                enabled: topic.chips[2].enabled,
            },
            TopicChip {
                label: topic.chips[3].label,
                selected: topic.chips[3].selected,
                enabled: topic.chips[3].enabled,
            },
            TopicChip {
                label: topic.chips[4].label,
                selected: topic.chips[4].selected,
                enabled: topic.chips[4].enabled,
            },
            TopicChip {
                label: topic.chips[5].label,
                selected: topic.chips[5].selected,
                enabled: topic.chips[5].enabled,
            },
            TopicChip {
                label: topic.chips[6].label,
                selected: topic.chips[6].selected,
                enabled: topic.chips[6].enabled,
            },
        ],
        focus_region: topic.focus_region,
    });

    SettingsShell {
        appearance: model.appearance,
        title: model.title,
        mode: model.mode,
        rows: [
            SettingsRow {
                label: model.rows[0].label,
                value: model.rows[0].value,
                selected: model.rows[0].selected,
                show_arrow: model.rows[0].show_arrow,
            },
            SettingsRow {
                label: model.rows[1].label,
                value: model.rows[1].value,
                selected: model.rows[1].selected,
                show_arrow: model.rows[1].show_arrow,
            },
            SettingsRow {
                label: model.rows[2].label,
                value: model.rows[2].value,
                selected: model.rows[2].selected,
                show_arrow: model.rows[2].show_arrow,
            },
            SettingsRow {
                label: model.rows[3].label,
                value: model.rows[3].value,
                selected: model.rows[3].selected,
                show_arrow: model.rows[3].show_arrow,
            },
            SettingsRow {
                label: model.rows[4].label,
                value: model.rows[4].value,
                selected: model.rows[4].selected,
                show_arrow: model.rows[4].show_arrow,
            },
            SettingsRow {
                label: model.rows[5].label,
                value: model.rows[5].value,
                selected: model.rows[5].selected,
                show_arrow: model.rows[5].show_arrow,
            },
        ],
        refresh_title: model.refresh_title,
        refresh_body: model.refresh_body,
        topic_preferences,
    }
}

impl PreparedScreen {
    pub const fn appearance(self) -> AppearanceMode {
        match self {
            PreparedScreen::Dashboard(shell) => shell.appearance,
            PreparedScreen::Collection(shell) => shell.appearance,
            PreparedScreen::Reader(shell) => shell.appearance,
            PreparedScreen::ParagraphNavigation(shell) => shell.appearance,
            PreparedScreen::Settings(shell) => shell.appearance,
        }
    }
}

const fn counter_label(current_index: u8, total: u8) -> &'static str {
    match (current_index, total) {
        (1, 23) => "01 / 23",
        (2, 23) => "02 / 23",
        (3, 23) => "03 / 23",
        (4, 23) => "04 / 23",
        (5, 23) => "05 / 23",
        (6, 23) => "06 / 23",
        (7, 23) => "07 / 23",
        (8, 23) => "08 / 23",
        (9, 23) => "09 / 23",
        (10, 23) => "10 / 23",
        (11, 23) => "11 / 23",
        (12, 23) => "12 / 23",
        (13, 23) => "13 / 23",
        (14, 23) => "14 / 23",
        (15, 23) => "15 / 23",
        (16, 23) => "16 / 23",
        (17, 23) => "17 / 23",
        (18, 23) => "18 / 23",
        (19, 23) => "19 / 23",
        (20, 23) => "20 / 23",
        (21, 23) => "21 / 23",
        (22, 23) => "22 / 23",
        (23, 23) => "23 / 23",
        _ => "07 / 23",
    }
}

const fn paragraph_label(current_index: u8) -> &'static str {
    match current_index {
        1 => "PARAGRAPH 01",
        2 => "PARAGRAPH 02",
        3 => "PARAGRAPH 03",
        4 => "PARAGRAPH 04",
        5 => "PARAGRAPH 05",
        6 => "PARAGRAPH 06",
        7 => "PARAGRAPH 07",
        8 => "PARAGRAPH 08",
        9 => "PARAGRAPH 09",
        10 => "PARAGRAPH 10",
        11 => "PARAGRAPH 11",
        12 => "PARAGRAPH 12",
        13 => "PARAGRAPH 13",
        14 => "PARAGRAPH 14",
        15 => "PARAGRAPH 15",
        16 => "PARAGRAPH 16",
        17 => "PARAGRAPH 17",
        18 => "PARAGRAPH 18",
        19 => "PARAGRAPH 19",
        20 => "PARAGRAPH 20",
        21 => "PARAGRAPH 21",
        22 => "PARAGRAPH 22",
        23 => "PARAGRAPH 23",
        _ => "PARAGRAPH 07",
    }
}
