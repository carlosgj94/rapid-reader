use domain::{
    content::ArticleDocument,
    formatter::{ReadingDocument, format_article_document},
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
        FormatterStatus::Ready
    }

    fn format(&mut self, article: &ArticleDocument) -> ReadingDocument {
        format_article_document(article)
    }
}
