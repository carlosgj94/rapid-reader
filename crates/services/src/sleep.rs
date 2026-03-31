use domain::sleep::{SleepModel, WakeReason};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum SleepStatus {
    #[default]
    Awake,
    SleepPending,
    DeepSleeping,
}

pub trait SleepService {
    fn model(&self) -> &SleepModel;
    fn model_mut(&mut self) -> &mut SleepModel;

    fn status(&self) -> SleepStatus {
        match self.model().state {
            domain::sleep::SleepState::Awake => SleepStatus::Awake,
            domain::sleep::SleepState::SleepRequested => SleepStatus::SleepPending,
            domain::sleep::SleepState::DeepSleeping => SleepStatus::DeepSleeping,
        }
    }

    fn note_activity(&mut self, now_ms: u64) {
        self.model_mut().note_activity(now_ms);
    }

    fn request_sleep(&mut self) {
        self.model_mut().request_sleep();
    }

    fn should_sleep(&self, now_ms: u64) -> bool {
        self.model().should_sleep(now_ms)
    }

    fn mark_deep_sleeping(&mut self) {
        self.model_mut().mark_deep_sleeping();
    }

    fn mark_woke(&mut self, reason: WakeReason, now_ms: u64) {
        self.model_mut().mark_woke(reason, now_ms);
    }
}

#[derive(Debug, Default)]
pub struct NoopSleepService {
    model: SleepModel,
}

impl NoopSleepService {
    pub const fn new() -> Self {
        Self {
            model: SleepModel::new(domain::sleep::SleepConfig::new(
                domain::sleep::DEFAULT_INACTIVITY_TIMEOUT_MS,
            )),
        }
    }
}

impl SleepService for NoopSleepService {
    fn model(&self) -> &SleepModel {
        &self.model
    }

    fn model_mut(&mut self) -> &mut SleepModel {
        &mut self.model
    }
}
