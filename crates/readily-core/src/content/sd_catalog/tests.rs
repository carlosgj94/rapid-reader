use super::sanitize_chunk::sanitize_epub_chunk;
use super::*;

#[test]
fn titles_are_exposed() {
    let src = SdCatalogSource::new();
    assert_eq!(src.title_count(), 3);
    assert_eq!(src.title_at(0), Some("Don Quijote"));
    assert_eq!(src.title_at(2), Some("Moby Dick"));
}

#[test]
fn catalog_titles_can_be_replaced() {
    let mut src = SdCatalogSource::new();
    let loaded = src.set_catalog_titles_from_iter(["QUIJOTE.EPU", "ALICE.EPUB"]);
    assert_eq!(
        loaded,
        SdCatalogLoadResult {
            loaded: 2,
            truncated: false
        }
    );
    assert_eq!(src.title_count(), 2);
    assert_eq!(src.title_at(0), Some("Quijote"));
    assert_eq!(src.title_at(1), Some("Alice"));
}

#[test]
fn catalog_entries_keep_cover_flags() {
    let mut src = SdCatalogSource::new();
    let loaded = src.set_catalog_entries_from_iter([("Book One", true), ("Book Two", false)]);
    assert_eq!(
        loaded,
        SdCatalogLoadResult {
            loaded: 2,
            truncated: false
        }
    );
    assert!(src.has_cover_at(0));
    assert!(!src.has_cover_at(1));
}

#[test]
fn select_resets_and_streams_words() {
    let mut src = SdCatalogSource::new();
    src.select_text(1).unwrap();

    let first = src.next_word().unwrap().unwrap();
    assert_eq!(first.text, "Alice");
    assert_eq!(src.paragraph_progress(), (1, src.paragraph_progress().1));
}

#[test]
fn injected_epub_chunk_is_streamed() {
    let mut src = SdCatalogSource::new();
    src.set_catalog_entries_from_iter([("Book One", false)]);
    let loaded = src
        .set_catalog_text_chunk_from_bytes(
            0,
            b"<html><body>Hola <b>mundo</b>.</body></html>",
            true,
            "OEBPS/chapter.xhtml",
        )
        .unwrap();
    assert!(loaded.loaded);

    src.select_text(0).unwrap();
    let first = src.next_word().unwrap().unwrap();
    let second = src.next_word().unwrap().unwrap();
    assert_eq!(first.text, "Hola");
    assert_eq!(second.text, "mundo.");
}

#[test]
fn html_head_script_and_style_are_not_rendered() {
    let mut src = SdCatalogSource::new();
    src.set_catalog_entries_from_iter([("Book One", false)]);
    src.set_catalog_text_chunk_from_bytes(
            0,
            b"<html><head><title>Book Title</title><style>.x{color:red}</style></head><body>Hello <script>ignore_me()</script>world</body></html>",
            true,
            "OEBPS/chapter.xhtml",
        )
        .unwrap();
    src.select_text(0).unwrap();

    assert_eq!(src.next_word().unwrap().map(|w| w.text), Some("Hello"));
    assert_eq!(src.next_word().unwrap().map(|w| w.text), Some("world"));
    assert_eq!(src.next_word().unwrap(), None);
}

#[test]
fn html_state_resets_when_resource_changes() {
    let mut src = SdCatalogSource::new();
    src.set_catalog_entries_from_iter([("Book One", false)]);
    src.set_catalog_text_chunk_from_bytes(
        0,
        b"<html><body>uno</body></html>",
        true,
        "OEBPS/chapter1.xhtml",
    )
    .unwrap();
    src.set_catalog_text_chunk_from_bytes(
        0,
        b"<p>dos tres</p>",
        true,
        "OEBPS/chapter2-fragment.xhtml",
    )
    .unwrap();
    src.select_text(0).unwrap();

    assert_eq!(src.next_word().unwrap().map(|w| w.text), Some("dos"));
    assert_eq!(src.next_word().unwrap().map(|w| w.text), Some("tres"));
    assert_eq!(src.next_word().unwrap(), None);
}

#[test]
fn stream_detects_multiple_paragraphs_from_html_blocks() {
    let mut src = SdCatalogSource::new();
    src.set_catalog_entries_from_iter([("Book One", false)]);
    src.set_catalog_text_chunk_from_bytes(
            0,
            b"<html><body><h1>Capitulo Uno</h1><p>Primer parrafo.</p><p>Segundo parrafo.</p></body></html>",
            true,
            "OEBPS/chapter-one.xhtml",
        )
        .unwrap();
    src.select_text(0).unwrap();

    assert_eq!(src.paragraph_total(), 3);
    assert_eq!(src.paragraph_preview(0), Some("Capitulo Uno"));
    assert_eq!(src.paragraph_preview(1), Some("Primer parrafo."));
    assert_eq!(src.paragraph_preview(2), Some("Segundo parrafo."));
}

#[test]
fn stream_resource_transitions_advance_current_chapter() {
    let mut src = SdCatalogSource::new();
    src.set_catalog_entries_from_iter([("Book One", false)]);
    src.set_catalog_text_chunk_from_bytes(
        0,
        b"<html><body><p>uno</p></body></html>",
        true,
        "OEBPS/chapter-one.xhtml",
    )
    .unwrap();
    assert_eq!(src.current_chapter_index(), Some(0));
    assert_eq!(src.chapter_count(), 1);

    src.set_catalog_text_chunk_from_bytes(
        0,
        b"<html><body><p>dos</p></body></html>",
        false,
        "OEBPS/chapter-two.xhtml",
    )
    .unwrap();

    assert_eq!(src.current_chapter_index(), Some(1));
    assert_eq!(src.chapter_count(), 2);
    assert_eq!(
        src.chapter_at(1).map(|chapter| chapter.label),
        Some("Chapter Two")
    );
}

#[test]
fn html_entity_split_across_chunks_is_preserved() {
    let mut src = SdCatalogSource::new();
    src.set_catalog_entries_from_iter([("Book One", false)]);
    src.set_catalog_text_chunk_from_bytes(0, b"<p>Uno &amp", false, "OEBPS/chapter.xhtml")
        .unwrap();
    src.set_catalog_text_chunk_from_bytes(0, b"; Dos</p>", false, "OEBPS/chapter.xhtml")
        .unwrap();
    src.select_text(0).unwrap();

    assert_eq!(src.next_word().unwrap().map(|w| w.text), Some("&"));
    assert_eq!(src.next_word().unwrap().map(|w| w.text), Some("Dos"));
}

#[test]
fn stream_chapter_hint_sets_total_and_current() {
    let mut src = SdCatalogSource::new();
    src.set_catalog_entries_from_iter([("Book One", false)]);
    src.set_catalog_text_chunk_from_bytes(
        0,
        b"<html><body><p>uno</p></body></html>",
        false,
        "OEBPS/0007.xhtml",
    )
    .unwrap();
    src.set_catalog_stream_chapter_hint(0, 6, 24).unwrap();

    assert_eq!(src.current_chapter_index(), Some(6));
    assert_eq!(src.chapter_count(), 24);
}

#[test]
fn stream_seek_chapter_emits_targeted_refill_request() {
    let mut src = SdCatalogSource::new();
    src.set_catalog_entries_from_iter([("Book One", false)]);
    src.set_catalog_text_chunk_from_bytes(
        0,
        b"<html><body><p>uno dos</p></body></html>",
        false,
        "OEBPS/0002.xhtml",
    )
    .unwrap();
    src.set_catalog_stream_chapter_hint(0, 1, 12).unwrap();
    src.select_text(0).unwrap();

    assert!(src.seek_chapter(6).unwrap());
    assert_eq!(
        src.take_chunk_refill_request(),
        Some(SdChunkRefillRequest {
            book_index: 0,
            target_chapter: Some(6)
        })
    );
}

#[test]
fn chapter_label_infers_h_suffix_pattern() {
    let mut src = SdCatalogSource::new();
    src.set_catalog_entries_from_iter([("Book One", false)]);
    src.set_catalog_text_chunk_from_bytes(
        0,
        b"<html><body><p>uno</p></body></html>",
        false,
        "OEBPS/4969397097534755666_768-h-0.htm.xhtml",
    )
    .unwrap();

    assert_eq!(
        src.chapter_at(0).map(|chapter| chapter.label),
        Some("Chapter 1")
    );
}

#[test]
fn stream_chunk_requests_refill_when_depleted() {
    let mut src = SdCatalogSource::new();
    src.set_catalog_entries_from_iter([("Book One", false)]);
    src.set_catalog_text_chunk_from_bytes(0, b"uno dos", false, "OEBPS/chapter.xhtml")
        .unwrap();
    src.select_text(0).unwrap();

    assert_eq!(src.next_word().unwrap().map(|w| w.text), Some("uno"));
    assert_eq!(src.next_word().unwrap().map(|w| w.text), Some("dos"));
    assert_eq!(src.next_word().unwrap(), None);
    assert!(src.is_waiting_for_refill());
    assert_eq!(
        src.take_chunk_refill_request(),
        Some(SdChunkRefillRequest {
            book_index: 0,
            target_chapter: None
        })
    );
}

#[test]
fn stream_chunk_end_of_resource_still_requests_refill() {
    let mut src = SdCatalogSource::new();
    src.set_catalog_entries_from_iter([("Book One", false)]);
    src.set_catalog_text_chunk_from_bytes(0, b"uno dos", true, "OEBPS/chapter.xhtml")
        .unwrap();
    src.select_text(0).unwrap();

    assert_eq!(src.next_word().unwrap().map(|w| w.text), Some("uno"));
    assert_eq!(src.next_word().unwrap().map(|w| w.text), Some("dos"));
    assert_eq!(src.next_word().unwrap(), None);
    assert!(src.is_waiting_for_refill());
    assert_eq!(
        src.take_chunk_refill_request(),
        Some(SdChunkRefillRequest {
            book_index: 0,
            target_chapter: None
        })
    );
}

#[test]
fn sanitize_preserves_utf8_accents() {
    let mut html_state = HtmlParseState::default();
    let (sanitized, truncated, tail_start) =
        sanitize_epub_chunk("salió corazón".as_bytes(), &mut html_state, true);
    assert_eq!(sanitized.as_str(), "salió corazón");
    assert!(!truncated);
    assert_eq!(tail_start, None);
}

#[test]
fn stream_chunk_reassembles_split_utf8_codepoint() {
    let mut src = SdCatalogSource::new();
    src.set_catalog_entries_from_iter([("Book One", false)]);
    src.set_catalog_text_chunk_from_bytes(0, b"sali\xc3", false, "OEBPS/chapter.xhtml")
        .unwrap();
    src.set_catalog_text_chunk_from_bytes(0, b"\xb3 bien", false, "OEBPS/chapter.xhtml")
        .unwrap();
    src.select_text(0).unwrap();

    assert_eq!(src.next_word().unwrap().map(|w| w.text), Some("salió"));
    assert_eq!(src.next_word().unwrap().map(|w| w.text), Some("bien"));
}

#[test]
fn stream_chunk_decodes_typographic_apostrophe_entity() {
    let mut src = SdCatalogSource::new();
    src.set_catalog_entries_from_iter([("Book One", false)]);
    src.set_catalog_text_chunk_from_bytes(0, b"can&rsquo;t stop", false, "OEBPS/chapter.xhtml")
        .unwrap();
    src.select_text(0).unwrap();

    assert_eq!(src.next_word().unwrap().map(|w| w.text), Some("can't"));
    assert_eq!(src.next_word().unwrap().map(|w| w.text), Some("stop"));
}
