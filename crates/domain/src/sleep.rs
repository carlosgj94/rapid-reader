pub const DEFAULT_INACTIVITY_TIMEOUT_MS: u64 = 30_000;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum SleepState {
    #[default]
    Awake,
    SleepRequested,
    DeepSleeping,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum WakeReason {
    #[default]
    ColdBoot,
    ExternalButton,
    Unknown,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct SleepConfig {
    pub inactivity_timeout_ms: u64,
}

impl SleepConfig {
    pub const fn new(inactivity_timeout_ms: u64) -> Self {
        Self {
            inactivity_timeout_ms,
        }
    }
}

impl Default for SleepConfig {
    fn default() -> Self {
        Self::new(DEFAULT_INACTIVITY_TIMEOUT_MS)
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct SleepModel {
    pub config: SleepConfig,
    pub state: SleepState,
    pub last_activity_ms: u64,
    pub last_wake_reason: WakeReason,
}

impl SleepModel {
    pub const fn new(config: SleepConfig) -> Self {
        Self {
            config,
            state: SleepState::Awake,
            last_activity_ms: 0,
            last_wake_reason: WakeReason::ColdBoot,
        }
    }

    pub fn note_activity(&mut self, now_ms: u64) {
        self.last_activity_ms = now_ms;
        self.state = SleepState::Awake;
    }

    pub fn request_sleep(&mut self) {
        self.state = SleepState::SleepRequested;
    }

    pub fn should_sleep(&self, now_ms: u64) -> bool {
        matches!(self.state, SleepState::SleepRequested)
            || now_ms.saturating_sub(self.last_activity_ms) >= self.config.inactivity_timeout_ms
    }

    pub fn mark_deep_sleeping(&mut self) {
        self.state = SleepState::DeepSleeping;
    }

    pub fn mark_woke(&mut self, reason: WakeReason, now_ms: u64) {
        self.last_wake_reason = reason;
        self.last_activity_ms = now_ms;
        self.state = SleepState::Awake;
    }
}

impl Default for SleepModel {
    fn default() -> Self {
        Self::new(SleepConfig::default())
    }
}
