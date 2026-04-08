use crate::{content::CollectionKind, settings::TOPIC_CATEGORY_COUNT};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum UiRoute {
    #[default]
    Dashboard,
    Collection(CollectionKind),
    Reader,
    Settings,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum DashboardFocus {
    Inbox,
    #[default]
    Saved,
    Recommendations,
}

impl DashboardFocus {
    pub const COUNT: usize = 3;

    pub const fn as_collection(self) -> CollectionKind {
        match self {
            Self::Inbox => CollectionKind::Inbox,
            Self::Saved => CollectionKind::Saved,
            Self::Recommendations => CollectionKind::Recommendations,
        }
    }

    pub const fn index(self) -> usize {
        match self {
            Self::Inbox => 0,
            Self::Saved => 1,
            Self::Recommendations => 2,
        }
    }

    pub const fn from_index(index: usize) -> Self {
        match index {
            0 => Self::Inbox,
            1 => Self::Saved,
            _ => Self::Recommendations,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum SettingsMode {
    #[default]
    Master,
    SpeedEdit,
    AppearanceEdit,
    RefreshLoading,
    TopicPreferences,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum SettingsRow {
    #[default]
    ReadingSpeed,
    Appearance,
    RefreshData,
    TopicPreferences,
    NetworkConnection,
    ConnectAccount,
}

impl SettingsRow {
    pub const COUNT: usize = 6;

    pub const fn label(self) -> &'static str {
        match self {
            Self::ReadingSpeed => "Reading Speed",
            Self::Appearance => "Appearance",
            Self::RefreshData => "Refresh Data",
            Self::TopicPreferences => "Topic Preferences",
            Self::NetworkConnection => "Network Connection",
            Self::ConnectAccount => "Connect Account",
        }
    }

    pub const fn index(self) -> usize {
        match self {
            Self::ReadingSpeed => 0,
            Self::Appearance => 1,
            Self::RefreshData => 2,
            Self::TopicPreferences => 3,
            Self::NetworkConnection => 4,
            Self::ConnectAccount => 5,
        }
    }

    pub const fn from_index(index: usize) -> Self {
        match index {
            0 => Self::ReadingSpeed,
            1 => Self::Appearance,
            2 => Self::RefreshData,
            3 => Self::TopicPreferences,
            4 => Self::NetworkConnection,
            _ => Self::ConnectAccount,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum TopicRegion {
    #[default]
    Categories,
    Chips,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum RecommendationsRegion {
    #[default]
    Articles,
    Subtopics,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct TopicFocus {
    pub region: TopicRegion,
    pub category_index: usize,
    pub chip_index: usize,
}

impl TopicFocus {
    pub const fn new() -> Self {
        Self {
            region: TopicRegion::Categories,
            category_index: 0,
            chip_index: 0,
        }
    }
}

impl Default for TopicFocus {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct UiState {
    pub route: UiRoute,
    pub dashboard_focus: DashboardFocus,
    pub saved_index: usize,
    pub inbox_index: usize,
    pub recommendations_index: usize,
    pub recommendations_subtopic_index: usize,
    pub recommendations_focus_flash_ticks: u8,
    pub recommendations_region: RecommendationsRegion,
    pub settings_mode: SettingsMode,
    pub settings_row: SettingsRow,
    pub topic_focus: TopicFocus,
}

impl UiState {
    pub const fn new() -> Self {
        Self {
            route: UiRoute::Dashboard,
            dashboard_focus: DashboardFocus::Saved,
            saved_index: 0,
            inbox_index: 0,
            recommendations_index: 0,
            recommendations_subtopic_index: 0,
            recommendations_focus_flash_ticks: 0,
            recommendations_region: RecommendationsRegion::Articles,
            settings_mode: SettingsMode::Master,
            settings_row: SettingsRow::ReadingSpeed,
            topic_focus: TopicFocus::new(),
        }
    }

    pub const fn collection_index(&self, kind: CollectionKind) -> usize {
        match kind {
            CollectionKind::Saved => self.saved_index,
            CollectionKind::Inbox => self.inbox_index,
            CollectionKind::Recommendations => self.recommendations_index,
        }
    }

    pub fn move_dashboard_previous(&mut self) {
        self.dashboard_focus =
            DashboardFocus::from_index(self.dashboard_focus.index().saturating_sub(1));
    }

    pub fn move_dashboard_next(&mut self) {
        self.dashboard_focus = DashboardFocus::from_index(
            self.dashboard_focus
                .index()
                .saturating_add(1)
                .min(DashboardFocus::COUNT.saturating_sub(1)),
        );
    }

    pub fn move_collection_previous(&mut self, kind: CollectionKind, len: usize) {
        let target = match kind {
            CollectionKind::Saved => &mut self.saved_index,
            CollectionKind::Inbox => &mut self.inbox_index,
            CollectionKind::Recommendations => &mut self.recommendations_index,
        };

        if len == 0 {
            *target = 0;
            return;
        }

        *target = (*target).min(len.saturating_sub(1)).saturating_sub(1);
    }

    pub fn move_collection_next(&mut self, kind: CollectionKind, len: usize) {
        let target = match kind {
            CollectionKind::Saved => &mut self.saved_index,
            CollectionKind::Inbox => &mut self.inbox_index,
            CollectionKind::Recommendations => &mut self.recommendations_index,
        };

        if len == 0 {
            *target = 0;
            return;
        }

        *target = (*target)
            .min(len.saturating_sub(1))
            .saturating_add(1)
            .min(len.saturating_sub(1));
    }

    pub fn move_settings_previous(&mut self) {
        self.settings_row = SettingsRow::from_index(self.settings_row.index().saturating_sub(1));
    }

    pub fn move_settings_next(&mut self) {
        self.settings_row = SettingsRow::from_index(
            self.settings_row
                .index()
                .saturating_add(1)
                .min(SettingsRow::COUNT.saturating_sub(1)),
        );
    }

    pub fn move_topic_previous(&mut self, chip_count: usize) {
        match self.topic_focus.region {
            TopicRegion::Categories => {
                self.topic_focus.category_index = self.topic_focus.category_index.saturating_sub(1);
                self.topic_focus.chip_index = 0;
            }
            TopicRegion::Chips => {
                if chip_count == 0 {
                    self.topic_focus.chip_index = 0;
                } else {
                    self.topic_focus.chip_index = self
                        .topic_focus
                        .chip_index
                        .min(chip_count.saturating_sub(1))
                        .saturating_sub(1);
                }
            }
        }
    }

    pub fn move_topic_next(&mut self, chip_count: usize) {
        match self.topic_focus.region {
            TopicRegion::Categories => {
                self.topic_focus.category_index = self
                    .topic_focus
                    .category_index
                    .saturating_add(1)
                    .min(TOPIC_CATEGORY_COUNT.saturating_sub(1));
                self.topic_focus.chip_index = 0;
            }
            TopicRegion::Chips => {
                if chip_count == 0 {
                    self.topic_focus.chip_index = 0;
                } else {
                    self.topic_focus.chip_index = self
                        .topic_focus
                        .chip_index
                        .min(chip_count.saturating_sub(1))
                        .saturating_add(1)
                        .min(chip_count.saturating_sub(1));
                }
            }
        }
    }
}

impl Default for UiState {
    fn default() -> Self {
        Self::new()
    }
}
