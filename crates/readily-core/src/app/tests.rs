use super::*;
use crate::{
    content::sd_catalog::SdCatalogSource,
    input::{InputEvent, InputProvider},
    render::Screen,
    settings::ResumeState,
};

struct ScriptedInput<'a> {
    events: &'a [InputEvent],
    cursor: usize,
}

impl<'a> ScriptedInput<'a> {
    const fn new(events: &'a [InputEvent]) -> Self {
        Self { events, cursor: 0 }
    }
}

impl InputProvider for ScriptedInput<'_> {
    type Error = ();

    fn poll_event(&mut self) -> Result<Option<InputEvent>, Self::Error> {
        let Some(event) = self.events.get(self.cursor).copied() else {
            return Ok(None);
        };
        self.cursor = self.cursor.saturating_add(1);
        Ok(Some(event))
    }
}

fn make_content() -> SdCatalogSource {
    let mut content = SdCatalogSource::new();
    let _ = content.set_catalog_entries_from_iter([("Book", false)]);
    let _ = content.set_catalog_text_chunk_from_bytes(
        0,
        b"<html><body><p>Hello world.</p></body></html>",
        true,
        "OEBPS/chapter-1.xhtml",
    );
    content
}

#[test]
fn same_tick_double_press_is_not_treated_as_home_shortcut() {
    let content = make_content();
    let events = [InputEvent::Press, InputEvent::Press];
    let input = ScriptedInput::new(&events);
    let mut app = ReaderApp::new(content, input, ReaderConfig::default(), "Test", 1);

    assert!(app.import_resume_state(
        ResumeState {
            selected_book: 0,
            chapter_index: 0,
            paragraph_in_chapter: 0,
            word_index: 1,
        },
        0,
    ));

    let _ = app.tick(1_000);

    let mut reading_paused = None;
    app.with_screen(1_000, |screen| match screen {
        Screen::Reading { paused, .. } => reading_paused = Some(paused),
        Screen::Library { .. } => reading_paused = None,
        _ => {}
    });

    assert_eq!(reading_paused, Some(true));
}
