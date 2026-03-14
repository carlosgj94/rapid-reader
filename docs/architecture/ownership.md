# Ownership

This note defines the current ownership boundaries in the codebase.

## Crate Boundaries

### `src/bin/main.rs` and `src/bin/main/*`

Owns board-specific orchestration:

- pin wiring
- peripheral bring-up
- boot and wake sequencing
- display flush loop
- SD preload and runtime refill wiring
- settings and resume persistence integration
- deep sleep entry
- Wi-Fi and ping background tasks

This layer should coordinate subsystems, not absorb business logic.

### `crates/readily-core`

Owns hardware-agnostic reader behavior:

- app state machine
- input-driven transitions
- screen/view projection
- reading timing
- navigation semantics
- persisted settings and wake snapshot types
- content traits and the current `SdCatalogSource` model

This layer should not know about ESP32 peripherals, display drivers, flash, or SPI.

### `crates/readily-hal-esp32s3`

Owns ESP32-S3 adapters:

- rotary input provider
- frame rendering for the RSVP UI
- flash settings backend
- SD, EPUB, and cover probe helpers
- connectivity snapshot and other hardware-facing support

This layer translates between real hardware/runtime services and the traits used by core.

### `crates/ls027b7dh01`

Owns reusable panel primitives:

- framebuffer
- protocol framing
- graphics helpers

It should stay reusable outside `readily`.

## Current Cross-Layer Contracts

The most important interfaces today are:

- `InputProvider`: event source into `ReaderApp`
- `FrameRenderer`: screen model to framebuffer rendering
- `SettingsStore`: persisted settings backend
- `TextCatalog`, `WordSource`, `SelectableWordSource`, `ParagraphNavigator`,
  `NavigationCatalog`: content contract between app logic and the content source

## Current Reality Checks

- `main.rs` still carries too much orchestration detail and several helper state machines.
- `readily-core::app` is hardware-agnostic but still too concentrated around one large `ReaderApp`
  implementation.
- `readily-hal-esp32s3::render::rsvp` is already split into files, but the renderer still owns
  several unrelated responsibilities in one namespace.

Those are structural cleanup targets, not boundary changes.
