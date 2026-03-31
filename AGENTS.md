# AGENTS.md

This file is the execution contract for coding agents working on `motif`.

## Mission

Keep the repository compile-safe while rebuilding the product from first principles through the new
technical documentation set.

## System Shape

```text
src/bin/main.rs                -> thin firmware entrypoint
crates/domain                  -> shared no_std domain/runtime skeleton
crates/app-runtime             -> shared no_std app/runtime skeleton
crates/services                -> shared no_std service skeleton
crates/platform-esp32s3        -> ESP32-S3 bootstrap and platform facade
crates/ls027b7dh01             -> reusable LS027 protocol + framebuffer primitives
docs/index.md                  -> documentation entrypoint
docs/architecture/overview.md  -> target product and software architecture
docs/modules/*                 -> target module specs
docs/board-config.md           -> current GPIO contract
docs/ls027-notes.md            -> panel notes
```

The current code is intentionally thinner than the target architecture. The docs under `docs/` are
the source of truth for the rebuild direction.

The current implementation baseline already includes:

- embassy-based platform/app runtime coordination
- rotary encoder gesture handling
- inactivity-based deep sleep and wake
- internal flash storage partitions for compact state

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
- Follow the target architecture in `docs/` when rebuilding functionality.
- Keep GPIO documentation aligned with `src/bin/main.rs`.
- Do not touch hardware design files under the KiCad project directory unless explicitly requested.
- Avoid touching toolchain and target configuration unless explicitly requested.

## Build + Validation Checklist

Run in repo root:

```bash
cargo fmt --all
cargo check --offline
cargo clippy --offline --workspace --lib
```

Flash layout notes:

- `.cargo/config.toml` passes the custom `partitions/motif.csv` table directly to the `espflash`
  runner.
- `espflash.toml` configures direct `espflash` invocations with the same table.
- Internal flash reserves `motif_state` and `motif_outbox` for durable device state.

Flash and monitor:

```bash
cargo run --release
```
