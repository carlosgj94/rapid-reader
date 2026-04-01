#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PowerStatus {
    pub battery_percent: u8,
}

impl PowerStatus {
    pub const fn new(battery_percent: u8) -> Self {
        Self { battery_percent }
    }
}

impl Default for PowerStatus {
    fn default() -> Self {
        Self::new(82)
    }
}
