# AGENTS.md

This file is the execution contract for coding agents working on `readily`.

## Mission

Ship a clean, low-latency RSVP reading experience on ESP32-S3 + Sharp LS027 memory LCD, while keeping app logic portable and testable.

## System Shape

```text
src/bin/main.rs                  -> board glue (pins, loop, wiring, flash settings backend)
crates/readily-core              -> no_std app state machine + traits + view models
crates/readily-hal-esp32s3       -> esp32s3 input/render/platform/storage adapters
crates/ls027b7dh01               -> panel protocol + framebuffer + reusable driver primitives
```

Keep this boundary strict:
- `readily-core` must remain hardware-agnostic.
- board-specific code must stay in `src/bin/main.rs` or `readily-hal-esp32s3`.

## Current Hardware Contract (Firmware Defaults)

### Display (LS027 breakout)

- `GPIO13` -> `CLK` / `SCLK`
- `GPIO14` -> `DI` / `SI` (MOSI)
- `GPIO15` -> `CS` / `SCS`
- `GPIO7` -> `DISP`
- `GPIO9` -> `EMD` / `EXTCOMIN`
- `GND` -> `GND`
- `3V3` -> `3V`

Notes:
- Prefer 3.3V rail for this build.
- `EIN` is not used in current firmware.
- If your breakout requires `VIN` as supply input, feed it with 3.3V for this setup.

### SD Card (SPI)

- `GPIO8` -> `CS`
- `GPIO4` -> `SCK`
- `GPIO40` -> `MOSI`
- `GPIO41` -> `MISO`

### Rotary Encoder

- `GPIO10` -> `CLK`
- `GPIO11` -> `DT`
- `GPIO12` -> `SW`
- Encoder inputs use pull-ups in firmware.

## Runtime Contracts

- Target WPM defaults to `230`.
- Dot pause and comma pause are both active (`ReaderConfig`).
- Countdown before reading is `3` seconds.
- Settings persistence is debounced (`1500 ms`) before writing flash.
- UI animations are renderer-driven; app emits state + animation metadata.
- SD stream chunking defaults to `480` bytes.
- Content source uses initial SD scan for titles + first chunks, then refills from SD while reading.

## Current EPUB/SD Behavior (Important)

- Library scan targets `/BOOKS` EPUB-like files (`.epub` / `.epu`).
- Initial boot pipeline:
  - scan catalog
  - load titles/cover flags
  - probe first text chunk per book
  - probe first cover thumbnail per book
- Reading pipeline:
  - chapter/paragraph state is maintained in `FakeSdCatalogSource`
  - when paragraph/chunk is exhausted, a refill request is raised to `main`
  - runtime probes next chunk/resource and applies it back into core state
- Chapter seek can require multiple probe steps across resources for large books; this can be visibly slower.

## Cover Decoding Status

Supported:
- PNG (non-interlaced path)
- JPEG when decoder-compatible (baseline-style path via ESP32 TJPGD)
- PBM

Known unsupported/partial:
- Progressive/unsupported JPEG variants (`jd_prepare status=8`, `fmt3_progressive_or_unsupported`)
- EPUBs without a discoverable cover resource (`no_cover_resource`)

Fallback behavior:
- If explicit cover metadata fails, fallback image candidates are attempted.

## Text Parsing and Rendering Status

- EPUB chunk sanitization is UTF-8 aware and handles chunk-boundary carryover for multibyte codepoints.
- HTML entities are decoded (including quote/accent entities like `&rsquo;`, `&aacute;`, etc.).
- cp1252-like punctuation/accent fallback mapping is present for common problematic bytes.
- Renderer normalization maps typographic quotes/dashes to ASCII glyphs so serif word rendering keeps apostrophes visible.

## Logging Notes (Do Not Misdiagnose)

- `Invalid FAT32 FSInfo sector ... Bad lead signature on InfoSector` is often non-fatal if probes continue and catalog/text load succeeds.
- `wifi connect failed: Disconnected` and `esp_wifi_internal_tx ...` are expected if AP is unavailable and should not block local reading behavior.

## Non-Negotiable Coding Rules

- Preserve `#![no_std]` in embedded crates.
- Do not move business logic into renderer/board glue.
- Avoid heap allocation in hot paths; prefer fixed buffers.
- Keep text shaping/truncation in shared policy helpers (`text_policy` / content utils).
- Keep HAL traits and core traits decoupled (composition over direct dependency).
- Avoid touching toolchain/target config unless explicitly requested.

## Build + Validation Checklist

Run in repo root:

```bash
cargo fmt --all
cargo check --offline
cargo clippy --offline --workspace --lib
```

Flash and monitor:

```bash
cargo run --release
```

Important:
- `.cargo/config.toml` sets default target to `xtensa-esp32s3-none-elf`.
- `cargo test --workspace` may fail in this target context (no standard test harness).

## UI/UX Direction Guardrails

- Reading screen should stay minimal and distraction-free.
- Pause and navigation views can be expressive, but legibility comes first.
- Box/card text must never overflow rendering bounds.
- Keep header layout stable (no accidental animation drift unless intended).
- Library view currently prioritizes larger covers:
  - selected card is taller
  - header shows current title instead of static "Library"
  - bottom title slot under cover was removed to reclaim cover space

## Planned Extension Points

- Faster chapter seek/indexing to reduce sequential probe latency.
- Better progressive JPEG support or deterministic conversion fallback.
- Standalone non-EPUB text source via existing content traits.
- Input expansion (long-press, multi-click semantics) via `InputProvider`.
- Additional transitions/animations behind existing renderer contracts.
