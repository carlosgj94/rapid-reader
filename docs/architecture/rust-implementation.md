# Rust Implementation Guidelines

This document sets the quality bar for the future implementation. It exists to keep the rebuild
from drifting into a monolithic embedded application with vague ownership and brittle async logic.

## Quality Bar

The implementation should look like it was designed, not accumulated.

That means:

- state machines are explicit
- ownership boundaries are narrow
- interfaces are typed and domain-specific
- side effects are isolated
- modules are small enough to review comfortably
- hot paths avoid unnecessary allocation and reparsing

## Recommended Package Boundaries

The initial workspace skeleton should use these crate boundaries:

- `motif`
  Thin firmware entrypoint only.
- `domain`
  Shared runtime/domain concepts.
- `app-runtime`
  Shared app and UI runtime surface types.
- `services`
  Shared service interfaces and no-op service shells.
- `platform-esp32s3`
  ESP32-S3-specific bootstrap and platform adapters.
- `ls027b7dh01`
  Reusable LS027 driver primitives.

The eventual codebase should separate concerns into a few stable layers.

### Domain and runtime layer

Owns:

- commands, events, effects
- store slices
- selectors
- canonical article and reading types
- formatter contracts

This layer should stay platform-agnostic.

### App/UI runtime layer

Owns:

- components
- navigation and screen composition
- animation descriptors
- UI view models

This layer should depend on selectors and formatter outputs, not on hardware drivers.

### Service layer

Owns:

- Wi-Fi orchestration
- backend sync
- storage policy
- input translation
- future power policy

Services may hold async handles and external clients, but they should still communicate with the
rest of the system through typed commands and events.

### Platform layer

Owns:

- ESP32-S3 adapters
- task startup
- driver wiring
- hardware-specific implementations of service contracts

### Driver layer

Owns:

- LS027 protocol and framebuffer primitives

## Design Rules

### Prefer typed state machines over boolean soup

Use enums for lifecycle state. Avoid combinations of booleans that require external knowledge to
interpret.

### Prefer concrete domain types over stringly APIs

Commands, events, selector keys, source kinds, and status markers should be represented with Rust
types, not ad hoc strings and maps.

### Keep effectful code at the edge

Reducers and formatter logic should be pure or nearly pure. Network calls, flash writes, SD I/O,
timers, and hardware interaction belong in services or platform adapters.

### Do not turn traits into an escape hatch

Traits are useful at real boundaries such as parser adapters, storage backends, and platform
services. They should not be used to hide core control flow or to replace normal module structure.

Prefer:

- concrete structs for the dominant implementation path
- enums for closed sets of behavior
- traits only where multiple implementations are genuinely expected

### Keep hot paths allocation-aware

Rendering, reader stepping, and frequently used selector paths should not repeatedly allocate or
reparse data. Precompute and cache durable derived structures where it makes the reading path more
predictable.

### Keep dependency policy strict on embedded paths

This codebase targets a low-level `no_std` device-first runtime.

That means:

- prefer `no_std`-compatible crates on firmware paths
- avoid importing ESP-IDF-oriented convenience layers unless the architecture explicitly changes
- avoid large convenience dependencies that pull in heap-heavy parsing or transport stacks
- justify every new dependency by device-side leverage, not by implementation convenience

For setup-related work in particular:

- provisioning, QR generation, BLE framing, and credential transport must stay compatible with the
  current embedded stack
- library choice should favor bounded memory behavior and small surface area

### Avoid hidden globals

The store is authoritative, but it should be passed and owned explicitly by the runtime. Avoid
mutable global singletons for domain state, service clients, or UI state.

## Concurrency Model

The desired model is cooperative and deterministic:

- the store serializes authoritative mutations
- services perform async work outside the store
- services re-enter the system through typed events
- long-running I/O never blocks the render path

This does not require only one task. It does require one coherent mutation path.

## Current Implemented Runtime Shape

The current firmware already uses that model in a minimal form.

Today:

- `main` starts the embassy-capable `esp_rtos` runtime
- `platform-esp32s3::bootstrap::run_minimal(...)` owns the hardware-facing loop
- `app_task` owns `Store` and `AppRuntime`
- embassy channels and signals are the cross-task boundary

The current async split is:

- platform side
  input sampling, display heartbeat, deep-sleep coordination, boot hardware setup
- app side
  sequential event handling, store mutation, placeholder screen selection

This is the shape future modules should extend rather than bypass.

## Current Async Boundaries

The runtime currently uses embassy timers and coordination primitives rather than a blocking delay
loop:

- `Ticker` for encoder sampling and heartbeat cadence
- `Timer::at(...)` for inactivity sleep deadlines
- bounded channels and signals for app/platform coordination

One important limitation is still intentional in the current baseline:

- LS027 display operations are still synchronous SPI writes inside the platform task

That means the runtime is embassy-shaped and cooperative, but the display path itself is not yet an
async driver.

## Error Strategy

Error handling should stay structured by layer.

- domain/runtime errors describe invalid transitions or unsupported content states
- service errors describe transport, storage, or parser failures
- platform errors describe hardware interaction failures

Avoid flattening all failures into opaque generic error values at the core of the system.

## Testing Strategy

The future test pyramid should look like this:

- unit tests for reducers, selectors, and formatter stages
- scenario tests for article lifecycle flows
- contract tests for parser adapters and storage backends
- platform integration tests for ESP32-specific wiring where feasible

The store mutation model should be easy to exercise in deterministic tests without hardware.

## Observability

The implementation should log and expose meaningful lifecycle facts rather than raw debug noise.

Important categories include:

- pairing and sync phases
- content ingestion and formatter failures
- storage degradation
- UI navigation and reader session transitions
- wake, sleep, and power events when that subsystem exists

## Reviewability Rules

To keep the codebase maintainable under human and automated editing:

- prefer many small modules over a few huge files
- keep public interfaces narrow
- make ownership readable from the type signatures
- avoid mixing domain logic, formatting logic, and platform code in one file
- keep each module responsible for one layer of abstraction
