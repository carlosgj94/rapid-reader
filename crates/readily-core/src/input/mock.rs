use super::{InputEvent, InputProvider};

/// No-hardware input source used during bring-up.
#[derive(Default, Debug, Clone, Copy)]
pub struct MockInput;

impl MockInput {
    pub const fn new() -> Self {
        Self
    }
}

impl InputProvider for MockInput {
    type Error = core::convert::Infallible;

    fn poll_event(&mut self) -> Result<Option<InputEvent>, Self::Error> {
        Ok(None)
    }
}
