use domain::{
    input::{InputGesture, RotationDirection},
    runtime::{Command, UiCommand},
};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct NavigationState;

impl NavigationState {
    pub const fn new() -> Self {
        Self
    }

    pub const fn command_for_gesture(gesture: InputGesture) -> Command {
        match gesture {
            InputGesture::Rotate {
                direction: RotationDirection::Clockwise,
            } => Command::Ui(UiCommand::FocusNext),
            InputGesture::Rotate {
                direction: RotationDirection::CounterClockwise,
            } => Command::Ui(UiCommand::FocusPrevious),
            InputGesture::Click => Command::Ui(UiCommand::Confirm),
            InputGesture::LongPress => Command::Ui(UiCommand::Back),
        }
    }
}

impl Default for NavigationState {
    fn default() -> Self {
        Self::new()
    }
}
