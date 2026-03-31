#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub struct ReaderProgress {
    pub unit_index: u32,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub struct ReaderSession {
    pub progress: ReaderProgress,
}

impl ReaderSession {
    pub const fn new() -> Self {
        Self {
            progress: ReaderProgress { unit_index: 0 },
        }
    }
}
