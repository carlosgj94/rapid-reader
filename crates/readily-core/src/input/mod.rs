//! Input abstraction layer.

/// Logical actions consumed by the reader app.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputEvent {
    RotateCw,
    RotateCcw,
    Press,
}

/// Polled input provider.
pub trait InputProvider {
    type Error;

    fn poll_event(&mut self) -> Result<Option<InputEvent>, Self::Error>;
}
