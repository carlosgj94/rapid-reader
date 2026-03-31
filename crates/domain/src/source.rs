#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum SourceKind {
    PersonalQueue,
    EditorialFeed,
    Import,
    #[default]
    Unknown,
}
