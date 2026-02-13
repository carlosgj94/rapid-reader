use super::*;

impl TextCatalog for SdCatalogSource {
    fn title_count(&self) -> u16 {
        self.catalog_titles.len().clamp(0, u16::MAX as usize) as u16
    }

    fn title_at(&self, index: u16) -> Option<&str> {
        self.catalog_titles
            .get(index as usize)
            .map(|title| title.as_str())
    }

    fn has_cover_at(&self, index: u16) -> bool {
        self.catalog_has_cover
            .get(index as usize)
            .copied()
            .unwrap_or(false)
    }
}
