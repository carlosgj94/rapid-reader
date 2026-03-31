# READILY

Embedded Rust firmware for an ESP32-S3 board with a Sharp LS027 memory LCD.

This repository is intentionally stripped down to a barebones compile target:

- `src/bin/main.rs` contains the minimal board bring-up and display heartbeat loop
- `crates/ls027b7dh01` contains the reusable LS027 driver primitives

The older app, contracts, HAL, SD, network, and reader runtime layers have been removed.

## Build And Flash

The workspace defaults to `xtensa-esp32s3-none-elf` via `.cargo/config.toml`.

Flash and monitor:

```bash
cargo run --release
```

Useful local checks:

```bash
cargo fmt --all
cargo check --offline
cargo clippy --offline --workspace --lib
```

## Hardware Docs

- [`docs/board-config.md`](docs/board-config.md): current GPIO wiring used by the stripped firmware
- [`docs/ls027-notes.md`](docs/ls027-notes.md): low-level LS027 protocol notes
