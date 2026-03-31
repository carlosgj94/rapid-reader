use crate::screens::Screen;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct NavigationState {
    pub active_screen: Screen,
}

impl NavigationState {
    pub const fn new() -> Self {
        Self {
            active_screen: Screen::Boot,
        }
    }
}

impl Default for NavigationState {
    fn default() -> Self {
        Self::new()
    }
}
