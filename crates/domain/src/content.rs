use crate::source::SourceKind;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ArticleDocument {
    pub source: SourceKind,
}

impl ArticleDocument {
    pub const fn new(source: SourceKind) -> Self {
        Self { source }
    }
}

impl Default for ArticleDocument {
    fn default() -> Self {
        Self::new(SourceKind::Unknown)
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub struct ReadingDocument;
