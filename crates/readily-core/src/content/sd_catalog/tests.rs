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
    src.set_catalog_entries_from_iter([("Book One", false), ("Book Two", false)]);
    src.set_catalog_text_chunk_from_bytes(
        1,
        b"<html><body><p>Alice</p></body></html>",
        true,
        "OEBPS/chapter.xhtml",
    )
    .unwrap();
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
    assert_eq!(src.next_word().unwrap().map(|word| word.text), Some("Hola"));
    assert_eq!(
        src.next_word().unwrap().map(|word| word.text),
        Some("mundo")
    );
    assert_eq!(src.next_word().unwrap().map(|word| word.text), Some("."));
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
fn stream_chapter_metadata_prefers_parser_label() {
    let mut src = SdCatalogSource::new();
    src.set_catalog_entries_from_iter([("Book One", false)]);
    src.set_catalog_text_chunk_from_bytes(
        0,
        b"<html><body><p>uno</p></body></html>",
        false,
        "OEBPS/0007.xhtml",
    )
    .unwrap();
    src.set_catalog_stream_chapter_metadata(0, 6, 24, Some("VII"))
        .unwrap();

    assert_eq!(src.current_chapter_index(), Some(6));
    assert_eq!(src.chapter_count(), 24);
    assert_eq!(src.chapter_at(6).map(|chapter| chapter.label), Some("VII"));
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
fn stream_chapter_data_ready_tracks_refill_and_loaded_state() {
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

    assert!(src.chapter_data_ready(1));
    assert!(src.seek_chapter(6).unwrap());
    assert!(!src.chapter_data_ready(6));

    let _ = src.take_chunk_refill_request();
    src.set_catalog_text_chunk_from_bytes(
        0,
        b"<html><body><p>capitulo siete</p><p>parrafo dos</p></body></html>",
        false,
        "OEBPS/0007.xhtml",
    )
    .unwrap();
    src.set_catalog_stream_chapter_hint(0, 6, 12).unwrap();

    assert!(src.chapter_data_ready(6));
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

#[cfg(not(target_arch = "xtensa"))]
mod epub_fixture_audit {
    extern crate std;

    use super::*;
    use miniz_oxide::inflate::{
        TINFLStatus,
        core::{DecompressorOxide, decompress, inflate_flags},
    };
    use std::{string::String as StdString, vec::Vec as StdVec};

    const BOOK_01_EPUB: &[u8] = include_bytes!("../../../../../tests/fixtures/epub/book_01.epub");
    const BOOK_01_TITLE: &str = "The Great Gatsby";
    const EXPECTED_CHAPTER_LABELS: [&str; 9] =
        ["I", "II", "III", "IV", "V", "VI", "VII", "VIII", "IX"];
    const EXPECTED_CHAPTER_AUDIT: [ChapterAuditExpectation; 9] = [
        ChapterAuditExpectation {
            label: "I",
            paragraph_total: 4,
            preview_1_prefix: "In my younger and more vulnerable years",
            preview_2_prefix: "“Whenever you feel like criticizing anyone,”",
        },
        ChapterAuditExpectation {
            label: "II",
            paragraph_total: 2,
            preview_1_prefix: "About halfway between West Egg and New York",
            preview_2_prefix: "",
        },
        ChapterAuditExpectation {
            label: "III",
            paragraph_total: 2,
            preview_1_prefix: "There was music from my neighbour’s house through the summer nights.",
            preview_2_prefix: "",
        },
        ChapterAuditExpectation {
            label: "IV",
            paragraph_total: 3,
            preview_1_prefix: "On Sunday morning while church bells rang in the villages alongshore",
            preview_2_prefix: "“He’s a bootlegger,” said the young ladies,",
        },
        ChapterAuditExpectation {
            label: "V",
            paragraph_total: 3,
            preview_1_prefix: "When I came home to West Egg that night I was afraid for a moment",
            preview_2_prefix: "At first I thought it was another party,",
        },
        ChapterAuditExpectation {
            label: "VI",
            paragraph_total: 5,
            preview_1_prefix: "About this time an ambitious young reporter from New York arrived one morning",
            preview_2_prefix: "“Anything to say about what?” inquired Gatsby politely.",
        },
        ChapterAuditExpectation {
            label: "VII",
            paragraph_total: 2,
            preview_1_prefix: "It was when curiosity about Gatsby was at its highest",
            preview_2_prefix: "",
        },
        ChapterAuditExpectation {
            label: "VIII",
            paragraph_total: 3,
            preview_1_prefix: "I couldn’t sleep all night; a foghorn was groaning incessantly on the Sound",
            preview_2_prefix: "Crossing his lawn, I saw that his front door was still open and",
        },
        ChapterAuditExpectation {
            label: "IX",
            paragraph_total: 2,
            preview_1_prefix: "After two years I remember the rest of that day,",
            preview_2_prefix: "",
        },
    ];
    const ZIP_EOCD_SIG: [u8; 4] = [0x50, 0x4B, 0x05, 0x06];
    const ZIP_CDIR_SIG: [u8; 4] = [0x50, 0x4B, 0x01, 0x02];
    const ZIP_LOCAL_SIG: [u8; 4] = [0x50, 0x4B, 0x03, 0x04];
    const ZIP_EOCD_MIN_BYTES: usize = 22;
    const ZIP_CDIR_HEADER_BYTES: usize = 46;
    const ZIP_LOCAL_HEADER_BYTES: usize = 30;
    const ZIP_MAX_CDIR_ENTRIES: usize = 4096;

    #[derive(Debug)]
    struct TocChapter {
        label: StdString,
        resource: StdString,
        fragment: StdString,
    }

    #[derive(Clone, Copy, Debug)]
    struct ChapterAuditExpectation {
        label: &'static str,
        paragraph_total: u16,
        preview_1_prefix: &'static str,
        preview_2_prefix: &'static str,
    }

    #[test]
    fn book_01_parser_and_storage_report_expected_chapters_and_paragraphs() {
        let opf = extract_zip_entry(BOOK_01_EPUB, b"OEBPS/content.opf")
            .expect("fixture should contain OEBPS/content.opf");
        let title = extract_dc_title(&opf).expect("fixture should expose dc:title");
        assert_eq!(title, BOOK_01_TITLE);

        let toc = extract_zip_entry(BOOK_01_EPUB, b"OEBPS/toc.ncx")
            .expect("fixture should contain OEBPS/toc.ncx");
        let chapters = parse_ncx_chapters(&toc);
        assert_eq!(chapters.len(), EXPECTED_CHAPTER_LABELS.len());
        for (index, expected) in EXPECTED_CHAPTER_LABELS.iter().enumerate() {
            assert_eq!(chapters[index].label.as_str(), *expected);
        }

        for (chapter_index, chapter) in chapters.iter().enumerate() {
            let expected = EXPECTED_CHAPTER_AUDIT[chapter_index];
            let chapter_position = chapter_index as u16;
            let chapter_total = chapters.len() as u16;
            let resource = extract_zip_entry(BOOK_01_EPUB, chapter.resource.as_bytes())
                .expect("chapter resource must be present in fixture EPUB");
            let start = find_fragment_start(&resource, chapter.fragment.as_str());
            let end = (start + super::SD_CATALOG_TEXT_BYTES).min(resource.len());
            let chunk = &resource[start..end];
            let end_of_resource = end >= resource.len();

            let mut storage = SdCatalogSource::new();
            storage.set_catalog_entries_from_iter([(BOOK_01_TITLE, false)]);
            let loaded = storage
                .set_catalog_text_chunk_from_bytes(
                    0,
                    chunk,
                    end_of_resource,
                    chapter.resource.as_str(),
                )
                .expect("storage should accept parsed chapter bytes");
            assert!(
                loaded.loaded,
                "chapter {} should decode into visible text",
                chapter.label
            );
            storage
                .set_catalog_stream_chapter_metadata(
                    0,
                    chapter_position,
                    chapter_total,
                    Some(chapter.label.as_str()),
                )
                .expect("storage should accept parser chapter metadata");
            storage
                .select_text(0)
                .expect("book selection should be valid");

            let paragraph_total = storage.paragraph_total();
            assert!(
                paragraph_total > 0,
                "chapter {} must expose at least one paragraph preview",
                chapter.label
            );
            let preview_0 = storage.paragraph_preview(0).unwrap_or("");
            let preview_1 = storage.paragraph_preview(1).unwrap_or("");
            let preview_2 = storage.paragraph_preview(2).unwrap_or("");
            assert_eq!(chapter.label.as_str(), expected.label);
            assert_eq!(preview_0, expected.label);
            assert_eq!(paragraph_total, expected.paragraph_total);
            assert!(
                !preview_0.is_empty(),
                "chapter {} first preview should not be empty",
                chapter.label
            );
            assert!(!preview_0.contains('<'), "preview text should be sanitized");
            assert!(
                preview_1.starts_with(expected.preview_1_prefix),
                "chapter {} preview_1 prefix mismatch: {:?}",
                chapter.label,
                preview_1
            );
            if expected.preview_2_prefix.is_empty() {
                assert_eq!(preview_2, "");
            } else {
                assert!(
                    preview_2.starts_with(expected.preview_2_prefix),
                    "chapter {} preview_2 prefix mismatch: {:?}",
                    chapter.label,
                    preview_2
                );
            }
            assert_eq!(storage.current_chapter_index(), Some(chapter_position));
            assert_eq!(storage.chapter_count(), chapter_total);
            assert_eq!(
                storage
                    .chapter_at(chapter_position)
                    .map(|entry| entry.label),
                Some(chapter.label.as_str())
            );
            assert_eq!(
                storage
                    .chapter_at(chapter_position)
                    .map(|entry| entry.paragraph_count),
                Some(paragraph_total)
            );

            std::println!(
                "fixture chapter={} label={} paragraph_total={} preview_0={:?} preview_1={:?} preview_2={:?}",
                chapter_position + 1,
                chapter.label,
                paragraph_total,
                preview_0,
                preview_1,
                preview_2
            );
        }
    }

    fn parse_ncx_chapters(ncx: &[u8]) -> StdVec<TocChapter> {
        let mut chapters = StdVec::new();
        let mut cursor = 0usize;
        while let Some(nav_start) = find_ascii_case_insensitive(ncx, b"<navPoint", cursor) {
            let Some(nav_end_start) = find_ascii_case_insensitive(ncx, b"</navPoint>", nav_start)
            else {
                break;
            };
            let nav_end = nav_end_start.saturating_add(b"</navPoint>".len());
            let scope = &ncx[nav_start..nav_end];

            let label = extract_tag_text(scope, b"<text>", b"</text>").unwrap_or_default();
            let src = extract_content_src(scope).unwrap_or_default();
            if !EXPECTED_CHAPTER_LABELS.contains(&label.as_str()) {
                cursor = nav_end;
                continue;
            }

            let (resource, fragment) = split_href_and_fragment(src.as_str(), "OEBPS/toc.ncx");
            chapters.push(TocChapter {
                label,
                resource,
                fragment,
            });
            cursor = nav_end;
        }
        chapters
    }

    fn split_href_and_fragment(href: &str, toc_path: &str) -> (StdString, StdString) {
        let base_dir = toc_path.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");
        if let Some((resource, fragment)) = href.split_once('#') {
            (
                resolve_relative_resource(resource.trim(), base_dir),
                StdString::from(fragment.trim()),
            )
        } else {
            (
                resolve_relative_resource(href.trim(), base_dir),
                StdString::new(),
            )
        }
    }

    fn resolve_relative_resource(resource: &str, base_dir: &str) -> StdString {
        if resource.starts_with('/') || resource.contains(':') || base_dir.is_empty() {
            return StdString::from(resource);
        }
        if resource.starts_with(base_dir) {
            return StdString::from(resource);
        }
        let mut resolved = StdString::from(base_dir);
        if !resolved.ends_with('/') {
            resolved.push('/');
        }
        resolved.push_str(resource);
        resolved
    }

    fn extract_dc_title(opf: &[u8]) -> Option<StdString> {
        extract_tag_text(opf, b"<dc:title>", b"</dc:title>")
    }

    fn extract_content_src(scope: &[u8]) -> Option<StdString> {
        let content_start = find_ascii_case_insensitive(scope, b"<content", 0)?;
        let content_end_rel = scope[content_start..]
            .iter()
            .position(|byte| *byte == b'>')?;
        let content_end = content_start.saturating_add(content_end_rel);
        let tag = &scope[content_start..content_end];
        extract_xml_attribute(tag, b"src")
    }

    fn extract_xml_attribute(tag: &[u8], name: &[u8]) -> Option<StdString> {
        let mut cursor = 0usize;
        while let Some(attr_pos) = find_ascii_case_insensitive(tag, name, cursor) {
            let boundary_ok = attr_pos == 0
                || tag[attr_pos - 1].is_ascii_whitespace()
                || tag[attr_pos - 1] == b'<'
                || tag[attr_pos - 1] == b'/';
            if !boundary_ok {
                cursor = attr_pos.saturating_add(1);
                continue;
            }
            let mut idx = attr_pos.saturating_add(name.len());
            while idx < tag.len() && tag[idx].is_ascii_whitespace() {
                idx += 1;
            }
            if idx >= tag.len() || tag[idx] != b'=' {
                cursor = attr_pos.saturating_add(1);
                continue;
            }
            idx += 1;
            while idx < tag.len() && tag[idx].is_ascii_whitespace() {
                idx += 1;
            }
            if idx >= tag.len() {
                return None;
            }
            let quote = tag[idx];
            if quote != b'"' && quote != b'\'' {
                cursor = idx.saturating_add(1);
                continue;
            }
            idx += 1;
            let value_start = idx;
            while idx < tag.len() && tag[idx] != quote {
                idx += 1;
            }
            if idx <= value_start {
                return None;
            }
            return Some(StdString::from_utf8_lossy(&tag[value_start..idx]).into_owned());
        }
        None
    }

    fn extract_tag_text(xml: &[u8], open: &[u8], close: &[u8]) -> Option<StdString> {
        let open_pos = find_ascii_case_insensitive(xml, open, 0)?;
        let value_start = open_pos.saturating_add(open.len());
        let close_pos = find_ascii_case_insensitive(xml, close, value_start)?;
        if close_pos <= value_start {
            return None;
        }
        let value = StdString::from_utf8_lossy(&xml[value_start..close_pos]).into_owned();
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(StdString::from(trimmed))
    }

    fn find_fragment_start(resource: &[u8], fragment: &str) -> usize {
        let fragment = fragment.trim_start_matches('#').trim();
        if fragment.is_empty() {
            return 0;
        }

        let lower: StdVec<u8> = resource
            .iter()
            .map(|byte| byte.to_ascii_lowercase())
            .collect();
        let fragment_lower = fragment.to_ascii_lowercase();
        let patterns = [
            std::format!("id=\"{}\"", fragment_lower),
            std::format!("id='{}'", fragment_lower),
            std::format!("xml:id=\"{}\"", fragment_lower),
            std::format!("xml:id='{}'", fragment_lower),
            std::format!("name=\"{}\"", fragment_lower),
            std::format!("name='{}'", fragment_lower),
        ];
        for pattern in patterns.iter() {
            if let Some(match_pos) = find_exact_bytes(&lower, pattern.as_bytes(), 0) {
                return lower[..match_pos]
                    .iter()
                    .rposition(|byte| *byte == b'<')
                    .unwrap_or(match_pos);
            }
        }
        0
    }

    fn find_ascii_case_insensitive(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
        if needle.is_empty()
            || haystack.len() < needle.len()
            || from > haystack.len() - needle.len()
        {
            return None;
        }
        (from..=haystack.len() - needle.len()).find(|index| {
            haystack[*index..*index + needle.len()]
                .iter()
                .zip(needle.iter())
                .all(|(left, right)| left.eq_ignore_ascii_case(right))
        })
    }

    fn find_exact_bytes(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
        if needle.is_empty()
            || haystack.len() < needle.len()
            || from > haystack.len() - needle.len()
        {
            return None;
        }
        (from..=haystack.len() - needle.len())
            .find(|index| &haystack[*index..*index + needle.len()] == needle)
    }

    fn extract_zip_entry(archive: &[u8], path: &[u8]) -> Option<StdVec<u8>> {
        let eocd_offset = find_eocd_offset(archive)?;
        let cdir_entries = read_u16_le(archive, eocd_offset + 10) as usize;
        let cdir_offset = read_u32_le(archive, eocd_offset + 16) as usize;

        let mut cursor = cdir_offset;
        for _ in 0..cdir_entries.min(ZIP_MAX_CDIR_ENTRIES) {
            if cursor + ZIP_CDIR_HEADER_BYTES > archive.len() {
                return None;
            }
            if archive[cursor..cursor + 4] != ZIP_CDIR_SIG {
                return None;
            }

            let compression = read_u16_le(archive, cursor + 10);
            let compressed_size = read_u32_le(archive, cursor + 20) as usize;
            let uncompressed_size = read_u32_le(archive, cursor + 24) as usize;
            let name_len = read_u16_le(archive, cursor + 28) as usize;
            let extra_len = read_u16_le(archive, cursor + 30) as usize;
            let comment_len = read_u16_le(archive, cursor + 32) as usize;
            let local_offset = read_u32_le(archive, cursor + 42) as usize;

            let name_start = cursor + ZIP_CDIR_HEADER_BYTES;
            let name_end = name_start + name_len;
            if name_end > archive.len() {
                return None;
            }
            let name = &archive[name_start..name_end];

            let next_cursor = name_end
                .checked_add(extra_len)
                .and_then(|value| value.checked_add(comment_len))?;

            if eq_ascii_case_insensitive(name, path) {
                if local_offset + ZIP_LOCAL_HEADER_BYTES > archive.len() {
                    return None;
                }
                if archive[local_offset..local_offset + 4] != ZIP_LOCAL_SIG {
                    return None;
                }
                let local_name_len = read_u16_le(archive, local_offset + 26) as usize;
                let local_extra_len = read_u16_le(archive, local_offset + 28) as usize;
                let data_start = local_offset
                    .checked_add(ZIP_LOCAL_HEADER_BYTES)?
                    .checked_add(local_name_len)?
                    .checked_add(local_extra_len)?;
                let data_end = data_start.checked_add(compressed_size)?;
                if data_end > archive.len() {
                    return None;
                }
                let compressed = &archive[data_start..data_end];

                return match compression {
                    0 => Some(compressed.to_vec()),
                    8 => inflate_raw_entry(compressed, uncompressed_size),
                    _ => None,
                };
            }

            cursor = next_cursor;
        }
        None
    }

    fn find_eocd_offset(archive: &[u8]) -> Option<usize> {
        if archive.len() < ZIP_EOCD_MIN_BYTES {
            return None;
        }
        (0..=archive.len() - ZIP_EOCD_MIN_BYTES)
            .rev()
            .find(|offset| archive[*offset..].starts_with(&ZIP_EOCD_SIG))
    }

    fn inflate_raw_entry(compressed: &[u8], expected_size: usize) -> Option<StdVec<u8>> {
        let mut inflater = DecompressorOxide::new();
        let mut output = StdVec::new();
        output.resize(expected_size.max(1), 0);

        let mut input_pos = 0usize;
        let mut output_pos = 0usize;
        loop {
            let has_more_input = input_pos < compressed.len();
            let mut flags = inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF;
            if has_more_input {
                flags |= inflate_flags::TINFL_FLAG_HAS_MORE_INPUT;
            }

            let (status, consumed, written) = decompress(
                &mut inflater,
                &compressed[input_pos..],
                output.as_mut_slice(),
                output_pos,
                flags,
            );
            input_pos = input_pos.saturating_add(consumed);
            output_pos = output_pos.saturating_add(written);

            match status {
                TINFLStatus::Done => break,
                TINFLStatus::NeedsMoreInput => {
                    if !has_more_input {
                        return None;
                    }
                }
                TINFLStatus::HasMoreOutput => return None,
                _ => return None,
            }
        }

        output.truncate(output_pos.min(output.len()));
        Some(output)
    }

    fn eq_ascii_case_insensitive(left: &[u8], right: &[u8]) -> bool {
        left.len() == right.len()
            && left
                .iter()
                .zip(right.iter())
                .all(|(left, right)| left.eq_ignore_ascii_case(right))
    }

    fn read_u16_le(bytes: &[u8], offset: usize) -> u16 {
        u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
    }

    fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
        u32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ])
    }
}
