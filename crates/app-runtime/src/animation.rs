#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum MotionDirection {
    #[default]
    Forward,
    Backward,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum AnimationDescriptor {
    #[default]
    None,
    BandReveal(MotionDirection),
    ListStep(MotionDirection),
    ReaderEnter,
    ReaderExit,
    ModalReveal,
    ModalHide,
    ParagraphTickMove(MotionDirection),
    SettingsValuePulse,
    AppearanceFlip,
    RefreshPulse,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct TransitionPlan {
    pub animation: AnimationDescriptor,
    pub steps: u8,
    pub frame_ms: u16,
}

impl TransitionPlan {
    pub const fn none() -> Self {
        Self {
            animation: AnimationDescriptor::None,
            steps: 1,
            frame_ms: 0,
        }
    }

    pub const fn new(animation: AnimationDescriptor, steps: u8, frame_ms: u16) -> Self {
        Self {
            animation,
            steps,
            frame_ms,
        }
    }
}

impl Default for TransitionPlan {
    fn default() -> Self {
        Self::none()
    }
}
