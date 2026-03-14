# Boot To Library

This note documents the current boot path and the current library interaction.

## Cold Boot

On boot the firmware initializes the display, rotary input, SD SPI bus, loading renderer, and the
`SdCatalogSource` content model.

Current boot pipeline:

1. Bring up display and push an initial test frame.
2. Initialize rotary input and SD SPI.
3. Try to load the cached book manifest from the SD-side book DB.
4. If the manifest is missing or invalid, scan `/BOOKS` for `.epub` and `.epu` files.
5. Preload the title list, first text chunk, and cover thumbnail for each discovered book.
6. Save a manifest back to the SD-side book DB when a fresh runtime scan was needed.
7. Load persisted reader settings from flash if available.
8. Restore wake snapshot on deep-sleep boots, and restore plain SD-side resume when progress is
   available on a normal boot.
9. Finish in either the library or a restored paused state, depending on whether resume or wake
   restore succeeded.

The loading screen is renderer-driven and reports phases such as SD probe, library scan, first text
load, and cover decode.

## Library Screen

The library is the default visible screen on a normal boot.

Current behavior:

- The library shows a sliding window of book cards plus one trailing `Settings` card.
- The selected title is rendered as the large centered card.
- Adjacent titles render as smaller left and right cards when they exist.
- Book cover thumbnails are used when preloaded; otherwise a built-in symbolic cover is drawn.
- The header keeps the app title and current library context stable while the card shelf animates.

Controls:

- Rotate clockwise: move to the next library item.
- Rotate counter-clockwise: move to the previous library item.
- Press on a book: select that title and enter the reading countdown.
- Press on `Settings`: open the settings screen.

The cursor wraps across the full item list, including the trailing `Settings` entry.

## Boot Data Sources

Current source precedence:

1. SD-side manifest cache for the catalog and initial stream metadata.
2. Full SD scan of `/BOOKS` when the manifest is missing, invalid, or unusable.
3. Built-in placeholder catalog titles from `SdCatalogSource::new()` when SD probing cannot provide
   a usable catalog.

Important current limitation:

- The built-in placeholder catalog is only a fallback shell. It keeps the UI alive, but it is not a
  real offline library with populated reading content.

## Visible Failure And Fallback Behavior

- Missing `/BOOKS` directory: boot completes and the library falls back to placeholder titles.
- SD probe failure across all configured SPI speeds: boot still completes with placeholder titles.
- Per-book text probe failure: the title can still appear in the library, but reading that entry may
  reach end-of-text immediately.
- Per-book cover decode failure: the library uses the symbolic cover fallback for that book.
- Selecting a placeholder-only fallback title can open countdown and then settle into paused
  end-of-text behavior almost immediately because no real paragraph data exists.
