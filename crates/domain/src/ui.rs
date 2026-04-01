use crate::content::{ARTICLE_COUNT_PER_COLLECTION, CollectionKind};

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
    pub const fn as_collection(self) -> CollectionKind {
        match self {
            Self::Inbox => CollectionKind::Inbox,
            Self::Saved => CollectionKind::Saved,
            Self::Recommendations => CollectionKind::Recommendations,
        }
    }

    pub const fn previous(self) -> Self {
        match self {
            Self::Inbox => Self::Recommendations,
            Self::Saved => Self::Inbox,
            Self::Recommendations => Self::Saved,
        }
    }

    pub const fn next(self) -> Self {
        match self {
            Self::Inbox => Self::Saved,
            Self::Saved => Self::Recommendations,
            Self::Recommendations => Self::Inbox,
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

    pub const fn previous(self) -> Self {
        match self {
            Self::ReadingSpeed => Self::ConnectAccount,
            Self::Appearance => Self::ReadingSpeed,
            Self::RefreshData => Self::Appearance,
            Self::TopicPreferences => Self::RefreshData,
            Self::NetworkConnection => Self::TopicPreferences,
            Self::ConnectAccount => Self::NetworkConnection,
        }
    }

    pub const fn next(self) -> Self {
        match self {
            Self::ReadingSpeed => Self::Appearance,
            Self::Appearance => Self::RefreshData,
            Self::RefreshData => Self::TopicPreferences,
            Self::TopicPreferences => Self::NetworkConnection,
            Self::NetworkConnection => Self::ConnectAccount,
            Self::ConnectAccount => Self::ReadingSpeed,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum TopicRegion {
    #[default]
    Categories,
    Chips,
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
    pub settings_mode: SettingsMode,
    pub settings_row: SettingsRow,
    pub topic_focus: TopicFocus,
}

impl UiState {
    pub const fn new() -> Self {
        Self {
            route: UiRoute::Dashboard,
            dashboard_focus: DashboardFocus::Saved,
            saved_index: 1,
            inbox_index: 1,
            recommendations_index: 1,
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
        self.dashboard_focus = self.dashboard_focus.previous();
    }

    pub fn move_dashboard_next(&mut self) {
        self.dashboard_focus = self.dashboard_focus.next();
    }

    pub fn move_collection_previous(&mut self, kind: CollectionKind) {
        let target = match kind {
            CollectionKind::Saved => &mut self.saved_index,
            CollectionKind::Inbox => &mut self.inbox_index,
            CollectionKind::Recommendations => &mut self.recommendations_index,
        };

        *target = (*target + ARTICLE_COUNT_PER_COLLECTION - 1) % ARTICLE_COUNT_PER_COLLECTION;
    }

    pub fn move_collection_next(&mut self, kind: CollectionKind) {
        let target = match kind {
            CollectionKind::Saved => &mut self.saved_index,
            CollectionKind::Inbox => &mut self.inbox_index,
            CollectionKind::Recommendations => &mut self.recommendations_index,
        };

        *target = (*target + 1) % ARTICLE_COUNT_PER_COLLECTION;
    }

    pub fn move_settings_previous(&mut self) {
        self.settings_row = self.settings_row.previous();
    }

    pub fn move_settings_next(&mut self) {
        self.settings_row = self.settings_row.next();
    }

    pub fn move_topic_previous(&mut self, chip_count: usize) {
        match self.topic_focus.region {
            TopicRegion::Categories => {
                self.topic_focus.category_index =
                    (self.topic_focus.category_index + 4usize - 1) % 4usize;
                self.topic_focus.chip_index = 0;
            }
            TopicRegion::Chips => {
                self.topic_focus.chip_index =
                    (self.topic_focus.chip_index + chip_count - 1) % chip_count.max(1);
            }
        }
    }

    pub fn move_topic_next(&mut self, chip_count: usize) {
        match self.topic_focus.region {
            TopicRegion::Categories => {
                self.topic_focus.category_index = (self.topic_focus.category_index + 1) % 4usize;
                self.topic_focus.chip_index = 0;
            }
            TopicRegion::Chips => {
                self.topic_focus.chip_index = (self.topic_focus.chip_index + 1) % chip_count.max(1);
            }
        }
    }
}

impl Default for UiState {
    fn default() -> Self {
        Self::new()
    }
}
