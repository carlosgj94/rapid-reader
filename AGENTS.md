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

## Current Hardware Contract

### Display (LS027 breakout)

- `GPIO4` -> `CLK` / `SCLK`
- `GPIO5` -> `DI` / `SI` (MOSI)
- `GPIO6` -> `CS` / `SCS`
- `GPIO7` -> `DISP`
- `GPIO9` -> `EMD` / `EXTCOMIN`
- `GND` -> `GND`
- `3V3` -> `3V`

Notes:
- Prefer 3.3V rail for this build.
- `EIN` is not used in current firmware.
- If your breakout requires `VIN` as supply input, feed it with 3.3V for this setup.

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

## Planned Extension Points

- Real SD-backed content source (implement existing content traits, do not bypass app layer).
- Cover/image support in library cards.
- Input expansion (long-press, multi-click semantics) via `InputProvider`.
- Additional transitions/animations behind existing renderer contracts.
