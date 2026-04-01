use crate::sleep::DEFAULT_INACTIVITY_TIMEOUT_MS;

pub const TOPIC_CATEGORY_COUNT: usize = 4;
pub const TOPIC_CHIP_COUNT: usize = 7;
pub const DEFAULT_READING_SPEED_WPM: u16 = 260;
pub const MIN_READING_SPEED_WPM: u16 = 200;
pub const MAX_READING_SPEED_WPM: u16 = 360;
pub const READING_SPEED_STEP_WPM: u16 = 20;
pub const REFRESH_LOADING_DURATION_MS: u64 = 720;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PersistedSettings {
    pub inactivity_timeout_ms: u64,
    pub reading_speed_wpm: u16,
    pub appearance: AppearanceMode,
    pub topics: TopicPreferences,
}

impl PersistedSettings {
    pub const fn new(inactivity_timeout_ms: u64) -> Self {
        Self::with_preferences(
            inactivity_timeout_ms,
            DEFAULT_READING_SPEED_WPM,
            AppearanceMode::Light,
            TopicPreferences::new(),
        )
    }

    pub const fn with_preferences(
        inactivity_timeout_ms: u64,
        reading_speed_wpm: u16,
        appearance: AppearanceMode,
        topics: TopicPreferences,
    ) -> Self {
        Self {
            inactivity_timeout_ms,
            reading_speed_wpm,
            appearance,
            topics,
        }
    }
}

impl Default for PersistedSettings {
    fn default() -> Self {
        Self::new(DEFAULT_INACTIVITY_TIMEOUT_MS)
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum AppearanceMode {
    #[default]
    Light,
    Dark,
}

impl AppearanceMode {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Light => "LIGHT",
            Self::Dark => "DARK",
        }
    }

    pub const fn toggled(self) -> Self {
        match self {
            Self::Light => Self::Dark,
            Self::Dark => Self::Light,
        }
    }

    pub const fn to_byte(self) -> u8 {
        match self {
            Self::Light => 0,
            Self::Dark => 1,
        }
    }

    pub const fn from_byte(value: u8) -> Self {
        match value {
            1 => Self::Dark,
            _ => Self::Light,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum RefreshState {
    #[default]
    Idle,
    Refreshing,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct TopicPreferences {
    pub enabled_by_category: [[bool; TOPIC_CHIP_COUNT]; TOPIC_CATEGORY_COUNT],
}

impl TopicPreferences {
    pub const fn new() -> Self {
        Self {
            enabled_by_category: [
                [true, false, false, false, true, false, false],
                [true, false, true, false, false, false, false],
                [true, true, false, true, false, false, false],
                [true, false, true, false, true, false, false],
            ],
        }
    }

    pub fn toggle_chip(&mut self, category_index: usize, chip_index: usize) {
        if let Some(row) = self.enabled_by_category.get_mut(category_index)
            && let Some(chip) = row.get_mut(chip_index)
        {
            *chip = !*chip;
        }
    }

    pub fn to_bits(&self) -> u32 {
        let mut bits = 0u32;
        let mut bit_index = 0u32;
        let mut category_index = 0usize;

        while category_index < TOPIC_CATEGORY_COUNT {
            let mut chip_index = 0usize;
            while chip_index < TOPIC_CHIP_COUNT {
                if self.enabled_by_category[category_index][chip_index] {
                    bits |= 1u32 << bit_index;
                }
                bit_index += 1;
                chip_index += 1;
            }
            category_index += 1;
        }

        bits
    }

    pub fn from_bits(bits: u32) -> Self {
        let mut enabled_by_category = [[false; TOPIC_CHIP_COUNT]; TOPIC_CATEGORY_COUNT];
        let mut bit_index = 0u32;
        let mut category_index = 0usize;

        while category_index < TOPIC_CATEGORY_COUNT {
            let mut chip_index = 0usize;
            while chip_index < TOPIC_CHIP_COUNT {
                enabled_by_category[category_index][chip_index] = (bits & (1u32 << bit_index)) != 0;
                bit_index += 1;
                chip_index += 1;
            }
            category_index += 1;
        }

        Self {
            enabled_by_category,
        }
    }
}

impl Default for TopicPreferences {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct SettingsState {
    pub inactivity_timeout_ms: u64,
    pub reading_speed_wpm: u16,
    pub appearance: AppearanceMode,
    pub refresh_state: RefreshState,
    pub refresh_started_at_ms: Option<u64>,
    pub topics: TopicPreferences,
}

impl SettingsState {
    pub const fn new(inactivity_timeout_ms: u64) -> Self {
        Self {
            inactivity_timeout_ms,
            reading_speed_wpm: DEFAULT_READING_SPEED_WPM,
            appearance: AppearanceMode::Light,
            refresh_state: RefreshState::Idle,
            refresh_started_at_ms: None,
            topics: TopicPreferences::new(),
        }
    }

    pub const fn from_persisted(settings: PersistedSettings) -> Self {
        let reading_speed_wpm = if settings.reading_speed_wpm < MIN_READING_SPEED_WPM {
            MIN_READING_SPEED_WPM
        } else if settings.reading_speed_wpm > MAX_READING_SPEED_WPM {
            MAX_READING_SPEED_WPM
        } else {
            settings.reading_speed_wpm
        };

        Self {
            inactivity_timeout_ms: settings.inactivity_timeout_ms,
            reading_speed_wpm,
            appearance: settings.appearance,
            refresh_state: RefreshState::Idle,
            refresh_started_at_ms: None,
            topics: settings.topics,
        }
    }

    pub const fn to_persisted(self) -> PersistedSettings {
        PersistedSettings::with_preferences(
            self.inactivity_timeout_ms,
            self.reading_speed_wpm,
            self.appearance,
            self.topics,
        )
    }

    pub fn adjust_reading_speed(&mut self, increase: bool) {
        let stepped = if increase {
            self.reading_speed_wpm
                .saturating_add(READING_SPEED_STEP_WPM)
        } else {
            self.reading_speed_wpm
                .saturating_sub(READING_SPEED_STEP_WPM)
        };

        self.reading_speed_wpm = stepped.clamp(MIN_READING_SPEED_WPM, MAX_READING_SPEED_WPM);
    }

    pub fn toggle_appearance(&mut self) {
        self.appearance = self.appearance.toggled();
    }

    pub fn start_refresh(&mut self, now_ms: u64) {
        self.refresh_state = RefreshState::Refreshing;
        self.refresh_started_at_ms = Some(now_ms);
    }

    pub fn complete_refresh(&mut self) {
        self.refresh_state = RefreshState::Idle;
        self.refresh_started_at_ms = None;
    }

    pub fn reading_speed_label(&self) -> &'static str {
        match self.reading_speed_wpm {
            200 => "200 WPM",
            220 => "220 WPM",
            240 => "240 WPM",
            260 => "260 WPM",
            280 => "280 WPM",
            300 => "300 WPM",
            320 => "320 WPM",
            340 => "340 WPM",
            360 => "360 WPM",
            _ => "260 WPM",
        }
    }
}

impl Default for SettingsState {
    fn default() -> Self {
        Self::new(DEFAULT_INACTIVITY_TIMEOUT_MS)
    }
}

pub const fn topic_category_label(index: usize) -> &'static str {
    match index {
        0 => "Technology",
        1 => "Culture",
        2 => "Design",
        3 => "World",
        _ => "Technology",
    }
}

pub const fn topic_chip_label(category_index: usize, chip_index: usize) -> &'static str {
    match category_index {
        0 => match chip_index {
            0 => "iPhone",
            1 => "Android",
            2 => "Computers",
            3 => "Linux",
            4 => "AI",
            5 => "Web",
            6 => "LLM",
            _ => "iPhone",
        },
        1 => match chip_index {
            0 => "Film",
            1 => "Music",
            2 => "Museums",
            3 => "Criticism",
            4 => "Radio",
            5 => "Poetry",
            6 => "Stage",
            _ => "Film",
        },
        2 => match chip_index {
            0 => "Type",
            1 => "Objects",
            2 => "Editorial",
            3 => "Materials",
            4 => "Interfaces",
            5 => "Studios",
            6 => "Print",
            _ => "Type",
        },
        3 => match chip_index {
            0 => "Cities",
            1 => "Climate",
            2 => "Trade",
            3 => "Mobility",
            4 => "Energy",
            5 => "Borders",
            6 => "Policy",
            _ => "Cities",
        },
        _ => "iPhone",
    }
}
