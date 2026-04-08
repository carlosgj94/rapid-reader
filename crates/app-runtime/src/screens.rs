#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum Screen {
    #[default]
    StartupSplash,
    Dashboard,
    Saved,
    Inbox,
    Recommendations,
    Reader,
    ParagraphNavigation,
    Settings,
}
