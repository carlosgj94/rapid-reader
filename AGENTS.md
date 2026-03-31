# AGENTS.md

This file is the execution contract for coding agents working on `readily`.

## Mission

Keep the repository in a stripped, compile-safe state while the product is rebuilt from first
principles.

## System Shape

```text
src/bin/main.rs      -> minimal ESP32-S3 bring-up + LS027 heartbeat loop
crates/ls027b7dh01   -> reusable LS027 protocol + framebuffer primitives
docs/board-config.md -> GPIO contract
docs/ls027-notes.md  -> panel notes
```

The older app, contracts, HAL, SD, network, and reader runtime layers are intentionally removed.

## Current Hardware Contract

### Display

- `GPIO13` -> `CLK` / display SPI SCK
- `GPIO14` -> `DI` / display SPI MOSI
- `GPIO15` -> `CS` / `SCS`
- `GPIO2` -> `DISP`
- `GPIO9` -> `EMD` / `EXTCOMIN`

### SD Card

- `GPIO8` -> `CS`
- `GPIO4` -> `SCK`
- `GPIO40` -> `MOSI`
- `GPIO41` -> `MISO`

### Rotary Encoder

- `GPIO10` -> `CLK`
- `GPIO11` -> `DT`
- `GPIO12` -> `SW`

## Non-Negotiable Rules

- Preserve `#![no_std]` in embedded crates.
- Keep the runtime barebones unless the user explicitly asks to rebuild functionality.
- Keep GPIO documentation aligned with `src/bin/main.rs`.
- Do not touch hardware design files under `kicad/readily/` unless explicitly requested.
- Avoid touching toolchain and target configuration unless explicitly requested.

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
