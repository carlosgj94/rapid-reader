use domain::{
    content::{ArticleDocument, ReadingDocument},
    source::SourceKind,
};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum FormatterStatus {
    #[default]
    Uninitialized,
    Ready,
}

pub trait SourceAdapter {
    fn source_kind(&self) -> SourceKind;
}

pub trait FormatterService {
    fn status(&self) -> FormatterStatus;
    fn format(&mut self, article: &ArticleDocument) -> ReadingDocument;
}

#[derive(Debug, Default)]
pub struct NoopFormatterService;

impl FormatterService for NoopFormatterService {
    fn status(&self) -> FormatterStatus {
        FormatterStatus::Uninitialized
    }

    fn format(&mut self, _article: &ArticleDocument) -> ReadingDocument {
        ReadingDocument
    }
}
