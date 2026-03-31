use domain::input::InputGesture;

pub trait InputService {
    fn pop_gesture(&mut self) -> Option<InputGesture>;
}

#[derive(Debug, Default)]
pub struct NoopInputService;

impl InputService for NoopInputService {
    fn pop_gesture(&mut self) -> Option<InputGesture> {
        None
    }
}
