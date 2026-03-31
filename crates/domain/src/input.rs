#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum RotationDirection {
    Clockwise,
    CounterClockwise,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum InputGesture {
    Rotate { direction: RotationDirection },
    Click,
    LongPress,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub struct InputState {
    pub last_gesture: Option<InputGesture>,
    pub delivered_sequence: u64,
    pub dropped_gesture_count: u32,
}

impl InputState {
    pub const fn new() -> Self {
        Self {
            last_gesture: None,
            delivered_sequence: 0,
            dropped_gesture_count: 0,
        }
    }

    pub fn record_gesture(&mut self, gesture: InputGesture) {
        self.last_gesture = Some(gesture);
        self.delivered_sequence = self.delivered_sequence.saturating_add(1);
    }

    pub fn record_dropped_gestures(&mut self, dropped: u32) {
        self.dropped_gesture_count = self.dropped_gesture_count.saturating_add(dropped);
    }
}
