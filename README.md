# MOTIF

Embedded Rust firmware for an ESP32-S3 board with a Sharp LS027 memory LCD.

This repository is in a docs-first rebuild state.

Today:

- `src/bin/main.rs` is a minimal bring-up baseline
- `crates/ls027b7dh01` contains the reusable display driver primitives
- `crates/domain`, `crates/app-runtime`, `crates/services`, and `crates/platform-esp32s3` define
  the barebones architecture skeleton
- the baseline already includes the current embassy runtime split, encoder input handling,
  deep-sleep integration, internal flash storage, and boot-hydrated settings
- the new target architecture is documented under `docs/`

The runtime is intentionally thinner than the target system. The technical documentation is now the
source of truth for the rebuild direction.

## Start Here

- [`docs/index.md`](docs/index.md): documentation entrypoint and reading order
- [`docs/architecture/overview.md`](docs/architecture/overview.md): target architecture
- [`docs/flows/article-lifecycle.md`](docs/flows/article-lifecycle.md): end-to-end content flow
- [`docs/modules/provisioning.md`](docs/modules/provisioning.md): BLE-first onboarding model

## Build And Flash

The workspace defaults to `xtensa-esp32s3-none-elf` via `.cargo/config.toml`.
The Cargo runner now passes the Motif partition table and `16mb` flash size explicitly to
`espflash`, and `espflash.toml` keeps the same layout available for direct `espflash` commands.

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
