# READILY

Embedded Rust RSVP reader for ESP32-S3 and a Sharp memory LCD.

The source of truth for current firmware behavior and current code ownership lives under
[`docs/index.md`](docs/index.md). The docs describe what the device does today, not future
features.

## Workspace Layout

```text
src/bin/main.rs                  board wiring, boot/runtime orchestration, sleep, network
crates/readily-core              hardware-agnostic app state, content traits, screen models
crates/readily-hal-esp32s3       ESP32-S3 input, rendering, storage, display adapters
crates/ls027b7dh01               Sharp LS027 protocol, framebuffer, panel primitives
```

Keep the boundary strict:

- `readily-core` stays hardware-agnostic.
- Board-specific code stays in `src/bin/main.rs` or `readily-hal-esp32s3`.

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

## Docs

- [`docs/index.md`](docs/index.md): docs hub
- [`docs/behavior/boot-library.md`](docs/behavior/boot-library.md): boot and library flow
- [`docs/behavior/rsvp-reading.md`](docs/behavior/rsvp-reading.md): reading loop and controls
- [`docs/behavior/navigation.md`](docs/behavior/navigation.md): chapter and paragraph navigation
- [`docs/behavior/runtime-behaviors.md`](docs/behavior/runtime-behaviors.md): settings, resume,
  sleep, and current edge cases
- [`docs/architecture/ownership.md`](docs/architecture/ownership.md): crate and module ownership
- [`docs/architecture/flow-map.md`](docs/architecture/flow-map.md): behavior-to-code map
- [`docs/review/cleanup-spec.md`](docs/review/cleanup-spec.md): prioritized cleanup plan
- [`docs/board-config.md`](docs/board-config.md): current board wiring reference
- [`docs/ls027-notes.md`](docs/ls027-notes.md): low-level panel protocol notes
