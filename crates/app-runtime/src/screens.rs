#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum Screen {
    #[default]
    Boot,
    Queue,
    Reader,
    Settings,
}
