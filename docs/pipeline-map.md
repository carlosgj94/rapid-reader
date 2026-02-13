# Pipeline Map

This map points to the exact files for SD reading, EPUB parsing, app storage, and UI loading.

## SD Reading And EPUB Probe Flow

- `crates/readily-hal-esp32s3/src/storage/sd_spi.rs`
  - Entry module that assembles all probe/decoder sub-components.
- `crates/readily-hal-esp32s3/src/storage/sd_spi/text_probe_core.rs`
  - Text entry discovery and chapter-based text probe functions.
- `crates/readily-hal-esp32s3/src/storage/sd_spi/stream_scan.rs`
  - Streaming text chunk APIs and catalog scan APIs used by runtime.
- `crates/readily-hal-esp32s3/src/storage/sd_spi/cover_probe.rs`
  - Cover probe and thumbnail extraction orchestration.
- `crates/readily-hal-esp32s3/src/storage/sd_spi/io_entry.rs`
  - ZIP entry read/decompress helpers.
- `crates/readily-hal-esp32s3/src/storage/sd_spi/text_xml.rs`
- `crates/readily-hal-esp32s3/src/storage/sd_spi/media_manifest.rs`
- `crates/readily-hal-esp32s3/src/storage/sd_spi/spine_index.rs`
  - EPUB XML/manifest/spine parsing helpers.

## Core Book Parsing And Runtime Content Model

- `crates/readily-core/src/content/sd_catalog.rs`
  - SD-backed content model entry module.
- `crates/readily-core/src/content/sd_catalog/catalog_setup.rs`
  - Catalog ingestion, title normalization, and catalog slot initialization.
- `crates/readily-core/src/content/sd_catalog/catalog_stream.rs`
  - Stream chunk apply, refill signaling, and chapter hint/state updates.
- `crates/readily-core/src/content/sd_catalog/source_state.rs`
  - Paragraph cursor/state transitions.
- `crates/readily-core/src/content/sd_catalog/traits_catalog.rs`
  - `TextCatalog` behavior.
- `crates/readily-core/src/content/sd_catalog/traits_word.rs`
  - `WordSource`, `SelectableWordSource`, and `ParagraphNavigator` behavior.
- `crates/readily-core/src/content/sd_catalog/traits_navigation.rs`
  - `NavigationCatalog` behavior.
- `crates/readily-core/src/content/sd_catalog/parsing_utils.rs`
  - Word bounds, chapter label extraction, and HTML tag state helpers.
- `crates/readily-core/src/content/sd_catalog/sanitize_chunk.rs`
  - Streaming chunk sanitizer and UTF-8 fallback logic.
- `crates/readily-core/src/content/sd_catalog/html_entities.rs`
  - Named/numeric HTML entity decoding.

## App Settings Storage And UI Load Path

- `crates/readily-hal-esp32s3/src/storage/flash_settings.rs`
  - Flash persistence backend implementation.
- `src/bin/main/settings_sync.rs`
  - Debounced settings save/load synchronization with `ReaderApp`.
- `src/bin/main/initial_catalog.rs`
  - Initial SD scan + first text chunk and cover preload pipeline for library UI boot data.
- `src/bin/main/sd_refill.rs`
  - Runtime SD chunk refill handling and applying data back into app content state.
- `src/bin/main.rs`
  - Board wiring and high-level orchestration (delegates SD/storage pipelines to focused modules).

## Reader UI State Machine

- `crates/readily-core/src/app/mod.rs`
  - App entry module and state definitions.
- `crates/readily-core/src/app/view.rs`
  - View model/screen mapping.
- `crates/readily-core/src/app/input.rs`
  - Input event handling and navigation transitions.
- `crates/readily-core/src/app/runtime.rs`
  - Tick/word timing/runtime stepping.
- `crates/readily-core/src/app/navigation.rs`
  - Navigation and status transitions.
