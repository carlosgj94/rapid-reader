# Target Architecture Overview

## Purpose

The next Motif architecture should be optimized for three things at the same time:

- a fast and legible RSVP reading experience
- modularity that supports aggressive review and iterative LLM-assisted implementation
- a clean Rust design that scales without collapsing into one large embedded application module

This document defines the target architecture at the system level. Module-level behavior is
specified in the docs under `docs/modules/`.

## Product Goal

Motif is not a general e-reader and not a web browser. It is a dedicated article-reading device
that uses RSVP presentation to make long-form web reading fast and focused.

The primary content sources are:

- a user's personal saved article queue
- editorial feeds selected or managed by the backend

The device should read smoothly offline after sync. Connectivity improves freshness and state sync,
but should not block the core reading loop.

## Architecture Principles

### One authoritative store, not one giant module

The system should have one authoritative runtime store so every user-visible subsystem can observe
coherent state. That does not mean every module can mutate arbitrary data or that the store becomes
a global dumping ground.

The intended model is:

- state is partitioned into domain slices
- mutations happen through typed commands
- reducers produce state transitions plus effects
- services execute effects and feed typed events back into the store
- selectors produce read models for the app runtime

This preserves the benefits of a central store while keeping Rust ownership explicit and testable.

### Declarative UI with an imperative renderer backend

The app should be expressed as components, view models, and animation descriptors. The LS027
renderer remains an imperative implementation detail that consumes a prepared scene and paints it
efficiently.

Business logic does not live inside rendering code.

### Offline-first reading

Wi-Fi and backend sync are important, but they are support systems around reading. Once content is
synced and formatted, the device should continue to operate without network access.

### Parse once, format once, render many times

The architecture should isolate three different concerns:

- ingesting source material
- normalizing and formatting it into reading structures
- rendering the current UI state

Those steps should not be blended together in one module.

### Small modules with explicit contracts

The implementation should favor more modules than a typical small embedded application. That is an
intentional design choice to keep review boundaries narrow and to make future automated edits
safer.

## Top-Level Module Map

| Module | Purpose | Owns | Does Not Own |
| --- | --- | --- | --- |
| App/UI runtime | Screens, components, navigation, animation descriptors | screen state, component composition, UI-local ephemeral state | transport, storage, parsing |
| Store | Authoritative runtime state and selectors | slice state, mutation pipeline, derived read models | sockets, drivers, file handles |
| Provisioning | First-time setup and reprovisioning | onboarding session lifecycle, BLE claim flow, staged Wi-Fi/backend bundle apply | steady-state Wi-Fi lifecycle, long-term sync scheduling |
| Wi-Fi | Connectivity transport | association state, IP/link status, retry policy | pairing, content sync protocol |
| Backend sync | Device pairing, manifests, article download, progress upload | sync jobs, sync policy, backend protocol | low-level Wi-Fi control, rendering |
| Formatter | Transform canonical article documents into RSVP-ready reading structures | segmentation, normalization, reading units, import adapters | network, UI state |
| Storage | Internal flash and SD coordination | persistence policy, indexes, cached packages | rendering, UI transitions |
| Input | Physical input translation into typed gestures | debouncing, gesture semantics, gesture delivery | navigation logic, rendering |
| Sleep | Inactivity tracking and deep-sleep entry | sleep policy, wake-source selection, wake/sleep signals | sync, formatting, battery telemetry |
| Power | Battery and charging integration when hardware exists | battery telemetry, charging state | article sync, formatting, deep-sleep policy |
| Platform layer | ESP32-S3-specific adapters | hardware bindings, driver setup, OS/task integration | domain rules |
| LS027 driver | Display protocol primitives | framebuffer, protocol framing | app logic, layout, state |

## Planned Workspace Crates

The initial implementation skeleton should be shaped around these workspace crates:

- `motif`
  Root firmware package and binary.
- `domain`
  Shared `no_std` domain/runtime types.
- `app-runtime`
  Shared `no_std` app/runtime surface types.
- `services`
  Shared `no_std` service traits and no-op implementations.
- `platform-esp32s3`
  ESP32-S3 bootstrap and platform facade.
- `ls027b7dh01`
  Display protocol and framebuffer primitives.

## Current Runtime Topology

The current implementation already follows the intended crate split, but only a subset of the
target runtime exists.

Today the concrete runtime shape is:

1. `src/bin/main.rs` enters the thread-mode embassy executor through `esp_rtos`
2. `platform-esp32s3::bootstrap::run_minimal(...)` owns hardware bring-up and the platform loop
3. `app_task` owns `Store` and `AppRuntime`
4. bounded embassy channels and signals move typed events and screens between those sides

The important current coordination paths are:

- `APP_EVENT_CH`
  Platform-to-app event delivery such as input gestures and wake events.
- `PLATFORM_CMD_CH`
  App-to-platform commands. Right now this is used for deep-sleep requests.
- `SCREEN_SIGNAL`
  App-to-platform screen invalidation with the latest `Screen` value.

This is intentionally narrower than the target system, but it already establishes the ownership
pattern the rest of the architecture should follow.

## Core Runtime Objects

The future implementation should revolve around a small set of stable concepts:

- `Store`
  The authoritative runtime state container.
- `Command`
  A typed request to mutate state or schedule work.
- `Event`
  A typed fact emitted after work or external observation.
- `Effect`
  A side-effect request emitted from the store and executed by a service.
- `Selector`
  A derived read model used by the app runtime or another service.
- `ArticleDocument`
  The canonical normalized article representation shared across sync, storage, and formatting.
- `ReadingDocument`
  The formatter output consumed by the RSVP reader.
- `Component`
  A composable app runtime unit with props and optional local ephemeral state.
- `AnimationDescriptor`
  A declarative description of motion tied to a state transition.

The current codebase implements only a small subset of these today:

- `Store`
- `Command`
- `Event`
- `Effect`
- `Screen`
- `InputGesture`
- `SleepModel`
- `StorageHealth`
- skeletal provisioning types

## Recommended Layering

The future code layout should follow this dependency direction:

1. driver layer
2. platform adapters
3. services
4. store and domain/runtime layer
5. app/UI runtime

The direction must not reverse. Rendering, Wi-Fi, SD access, and device power cannot become hidden
dependencies of the app or formatter layer.

## Article-Centric Product Loop

The core product loop is:

1. Pair the device with a backend identity.
2. Discover or refresh personal queue items and editorial feed items.
3. Download normalized article packages and store them locally.
4. Produce formatter outputs and local indexes.
5. Present queue and reader surfaces from store selectors.
6. Persist progress and reading state locally.
7. Sync progress and refreshed content back to the backend when possible.

This loop is described in more detail in [Article Lifecycle](../flows/article-lifecycle.md).

## Ownership Rules

- The store owns user-visible truth.
- Services own external resources and asynchronous work.
- Platform adapters own hardware handles and driver integration.
- Renderer-specific caches and layout caches live outside the store.
- Parsers and source adapters cannot write directly into unrelated slices.
- The app runtime can request work, but cannot bypass the store mutation path.

## Soft Performance Targets

The architecture should optimize for perceived immediacy and bounded work rather than premature
micro-optimizations.

The desired outcomes are:

- input causes visible feedback quickly
- screen transitions complete in a small, predictable number of display commits
- most UI changes avoid unnecessary full-screen re-layout and full-screen repaint
- reading path work is precomputed or cached where possible
- parsing and sync work stay off the hot path for active reading

These are intentionally soft targets for now. Numeric budgets can be added later when the first
real implementation path exists.

## Current Code State

The current repository is intentionally thinner than the target architecture described here, but it
now includes a few real subsystem implementations that are part of the rebuild baseline:

- embassy runtime coordination in `platform-esp32s3::bootstrap`
- rotary encoder gesture decoding
- inactivity-driven deep sleep and wake
- internal flash state and outbox storage
- skeletal provisioning interfaces and BLE-first onboarding documentation

What is still missing is the higher-level product logic:

- queue and reader behavior
- BLE onboarding transport and QR claim flow
- Wi-Fi and backend sync
- formatter pipeline
- broader persistent settings schemas beyond the current inactivity timeout
- SD content management
