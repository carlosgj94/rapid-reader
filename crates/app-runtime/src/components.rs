use crate::screens::Screen;
use domain::{
    content::{CONTENT_META_MAX_BYTES, CONTENT_TITLE_MAX_BYTES},
    formatter::{MAX_PARAGRAPH_PREVIEW_BYTES, MAX_STAGE_SEGMENT_BYTES, StageFont},
    selectors::{
        ActiveScreenModel, ContentListScreenModel, DashboardScreenModel, ParagraphNavigationModel,
        ReaderScreenModel, RecommendationBarModel, RecommendationTabModel, SettingsScreenModel,
        StartupSplashScreenModel,
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
pub struct SyncIndicator {
    pub label: &'static str,
    pub spinner_phase: u8,
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
    pub sync_indicator: Option<SyncIndicator>,
    pub rail: VerticalRail,
    pub items: [DashboardItem; 3],
    pub band: SelectionBand,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct StartupSplashShell {
    pub appearance: AppearanceMode,
    pub progress_width: u16,
    pub stripe_phase: u8,
    pub skip_hint: &'static str,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ContentRow {
    pub meta: domain::text::InlineText<CONTENT_META_MAX_BYTES>,
    pub title: domain::text::InlineText<CONTENT_TITLE_MAX_BYTES>,
    pub progress_badge: Option<domain::text::InlineText<8>>,
    pub is_fetching: bool,
    pub selected: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct HelpHint {
    pub text: &'static str,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct RecommendationTab {
    pub label: domain::text::InlineText<{ domain::selectors::RECOMMENDATION_TAB_LABEL_MAX_BYTES }>,
    pub active: bool,
    pub focused: bool,
    pub flash: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct RecommendationBar {
    pub tabs: [RecommendationTab; domain::selectors::RECOMMENDATION_VISIBLE_TABS],
    pub visible_count: usize,
    pub show_left_more: bool,
    pub show_right_more: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ContentListShell {
    pub appearance: AppearanceMode,
    pub status: StatusCluster,
    pub rail: VerticalRail,
    pub large_rail: bool,
    pub recommendations_bar: Option<RecommendationBar>,
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
    pub selected: bool,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PauseModal {
    pub title: &'static str,
    pub rows: [PauseModalRow; 4],
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct LoadingModal {
    pub title: &'static str,
    pub progress_width: u16,
    pub stripe_phase: u8,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ReaderModal {
    Pause(PauseModal),
    Loading(LoadingModal),
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct RsvpStage {
    pub title: domain::text::InlineText<CONTENT_TITLE_MAX_BYTES>,
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
    pub modal: Option<ReaderModal>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ParagraphMapRail {
    pub selected_index: u8,
    pub total_ticks: u8,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ParagraphNavigationShell {
    pub appearance: AppearanceMode,
    pub title: domain::text::InlineText<CONTENT_TITLE_MAX_BYTES>,
    pub current_index: u16,
    pub total: u16,
    pub counter: domain::text::InlineText<16>,
    pub previous_top: domain::text::InlineText<MAX_PARAGRAPH_PREVIEW_BYTES>,
    pub selected_label: domain::text::InlineText<16>,
    pub selected_excerpt: domain::text::InlineText<MAX_PARAGRAPH_PREVIEW_BYTES>,
    pub previous_bottom: domain::text::InlineText<MAX_PARAGRAPH_PREVIEW_BYTES>,
    pub final_excerpt: domain::text::InlineText<MAX_PARAGRAPH_PREVIEW_BYTES>,
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

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PreparedScreen {
    StartupSplash(StartupSplashShell),
    Dashboard(DashboardShell),
    Collection(ContentListShell),
    Reader(ReaderShell),
    ParagraphNavigation(ParagraphNavigationShell),
    Settings(SettingsShell),
}

pub fn compose(model: ActiveScreenModel) -> (Screen, PreparedScreen) {
    match model {
        ActiveScreenModel::StartupSplash(model) => (
            Screen::StartupSplash,
            PreparedScreen::StartupSplash(compose_startup_splash(model)),
        ),
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

fn compose_startup_splash(model: StartupSplashScreenModel) -> StartupSplashShell {
    StartupSplashShell {
        appearance: model.appearance,
        progress_width: model.progress_width,
        stripe_phase: model.stripe_phase,
        skip_hint: model.skip_hint,
    }
}

fn compose_dashboard(model: DashboardScreenModel) -> DashboardShell {
    DashboardShell {
        appearance: model.appearance,
        status: StatusCluster {
            battery_percent: model.status.battery_percent,
            wifi_online: model.status.network == domain::network::NetworkStatus::Online,
        },
        sync_indicator: model.sync_indicator.map(|indicator| SyncIndicator {
            label: indicator.label,
            spinner_phase: indicator.spinner_phase,
        }),
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
    let recommendations_bar = model.recommendations_bar.map(compose_recommendation_bar);
    ContentListShell {
        appearance: model.appearance,
        status: StatusCluster {
            battery_percent: model.status.battery_percent,
            wifi_online: model.status.network == domain::network::NetworkStatus::Online,
        },
        rail: VerticalRail {
            text: model.rail_label,
        },
        large_rail: matches!(
            model.selected_collection,
            domain::content::CollectionKind::Saved
                | domain::content::CollectionKind::Inbox
                | domain::content::CollectionKind::Recommendations
        ),
        recommendations_bar,
        rows: [
            ContentRow {
                meta: model.rows[0].meta,
                title: model.rows[0].title,
                progress_badge: model.rows[0].progress_badge,
                is_fetching: model.rows[0].is_fetching,
                selected: false,
            },
            ContentRow {
                meta: model.rows[1].meta,
                title: model.rows[1].title,
                progress_badge: model.rows[1].progress_badge,
                is_fetching: model.rows[1].is_fetching,
                selected: true,
            },
            ContentRow {
                meta: model.rows[2].meta,
                title: model.rows[2].title,
                progress_badge: model.rows[2].progress_badge,
                is_fetching: model.rows[2].is_fetching,
                selected: false,
            },
        ],
        band: if recommendations_bar.is_some() {
            SelectionBand { y: 100, height: 64 }
        } else {
            SelectionBand { y: 106, height: 68 }
        },
        help: HelpHint {
            text: "long press_",
        },
    }
}

fn compose_recommendation_bar(model: RecommendationBarModel) -> RecommendationBar {
    RecommendationBar {
        tabs: [
            compose_recommendation_tab(model.tabs[0]),
            compose_recommendation_tab(model.tabs[1]),
            compose_recommendation_tab(model.tabs[2]),
            compose_recommendation_tab(model.tabs[3]),
        ],
        visible_count: model.visible_count,
        show_left_more: model.show_left_more,
        show_right_more: model.show_right_more,
    }
}

fn compose_recommendation_tab(model: RecommendationTabModel) -> RecommendationTab {
    RecommendationTab {
        label: model.label,
        active: model.active,
        focused: model.focused,
        flash: model.flash,
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
        modal: model.modal.map(|modal| match modal {
            domain::selectors::ReaderModalModel::Pause(actions) => ReaderModal::Pause(PauseModal {
                title: "PAUSED",
                rows: [
                    PauseModalRow {
                        label: actions[0].label,
                        action: actions[0].action,
                        selected: actions[0].selected,
                        enabled: actions[0].enabled,
                    },
                    PauseModalRow {
                        label: actions[1].label,
                        action: actions[1].action,
                        selected: actions[1].selected,
                        enabled: actions[1].enabled,
                    },
                    PauseModalRow {
                        label: actions[2].label,
                        action: actions[2].action,
                        selected: actions[2].selected,
                        enabled: actions[2].enabled,
                    },
                    PauseModalRow {
                        label: actions[3].label,
                        action: actions[3].action,
                        selected: actions[3].selected,
                        enabled: actions[3].enabled,
                    },
                ],
            }),
            domain::selectors::ReaderModalModel::Loading(loading) => {
                ReaderModal::Loading(LoadingModal {
                    title: "LOADING",
                    progress_width: loading.progress_width,
                    stripe_phase: loading.stripe_phase,
                })
            }
        }),
    }
}

fn compose_paragraph_navigation(model: ParagraphNavigationModel) -> ParagraphNavigationShell {
    ParagraphNavigationShell {
        appearance: model.appearance,
        title: model.title,
        current_index: model.current_index,
        total: model.total,
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
            PreparedScreen::StartupSplash(shell) => shell.appearance,
            PreparedScreen::Dashboard(shell) => shell.appearance,
            PreparedScreen::Collection(shell) => shell.appearance,
            PreparedScreen::Reader(shell) => shell.appearance,
            PreparedScreen::ParagraphNavigation(shell) => shell.appearance,
            PreparedScreen::Settings(shell) => shell.appearance,
        }
    }
}

fn counter_label(current_index: u16, total: u16) -> domain::text::InlineText<16> {
    let mut label = domain::text::InlineText::new();
    push_decimal_2(&mut label, current_index);
    let _ = label.try_push_str(" / ");
    push_decimal_2(&mut label, total);
    label
}

fn paragraph_label(current_index: u16) -> domain::text::InlineText<16> {
    let mut label = domain::text::InlineText::new();
    let _ = label.try_push_str("PARAGRAPH ");
    push_decimal_2(&mut label, current_index);
    label
}

fn push_decimal_2(target: &mut domain::text::InlineText<16>, value: u16) {
    let clamped = value.min(999);
    if clamped >= 100 {
        let _ = target.try_push_char((b'0' + ((clamped / 100) % 10) as u8) as char);
    } else {
        let _ = target.try_push_char('0');
    }
    let _ = target.try_push_char((b'0' + ((clamped / 10) % 10) as u8) as char);
    let _ = target.try_push_char((b'0' + (clamped % 10) as u8) as char);
}
