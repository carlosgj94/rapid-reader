# Motif Technical Documentation

This documentation set defines the target architecture for the next implementation phase and the
current implementation baseline where concrete subsystems already exist.

The codebase is still intentionally thin, but it is no longer only a bare bring-up binary. The
current firmware now includes:

- an embassy-shaped runtime split between platform ownership and app/store processing
- a working rotary-encoder gesture pipeline
- deep sleep and wake handling tied to inactivity and the encoder switch
- a durable internal-flash storage engine for compact records and an outbox queue
- a boot-hydrated live store with real `device`, `settings`, `sleep`, `input`, and `storage`
  state
- skeletal provisioning interfaces for BLE-first onboarding

The docs below call out when a section describes the current implementation versus the longer-term
target shape.

## Product Definition

Motif is a dedicated RSVP reading device for internet articles on ESP32-S3 hardware with a Sharp
LS027 memory LCD.

The target product loop is:

1. Sync a user's personal saved articles and editorial feeds from the backend.
2. Cache normalized article packages locally.
3. Format those packages into RSVP-ready reading structures on device.
4. Render a fast, motion-capable reader UI with offline-first behavior.
5. Persist reading progress locally and sync it back when the network is available.

## Reading Order

1. [Architecture Overview](architecture/overview.md)
2. [Article Lifecycle](flows/article-lifecycle.md)
3. [Rust Implementation Guidelines](architecture/rust-implementation.md)
4. [Store](modules/store.md)
5. [Settings](modules/settings.md)
6. [Provisioning](modules/provisioning.md)
7. [App And UI Runtime](modules/app-ui.md)
8. [Wi-Fi](modules/wifi.md)
9. [Backend Sync](modules/backend-sync.md)
10. [Formatter And Content Pipeline](modules/formatter.md)
11. [Storage](modules/storage.md)
12. [Input](modules/input.md)
13. [Sleep](modules/sleep.md)
14. [Power Placeholder](modules/power.md)

## System At A Glance

- The app runtime is declarative and component-based rather than screen-by-screen imperative code.
- The store is the single authoritative runtime state container, but domain ownership stays split
  by slice and module.
- Provisioning owns first-time setup and reprovisioning.
- Wi-Fi only manages connectivity.
- Backend sync manages pairing, manifests, article packages, and progress exchange.
- Formatter owns the transformation from canonical article data to RSVP-ready reading units.
- Internal storage and SD storage have distinct responsibilities.
- Hardware-specific implementation stays at the platform edge.

## Current Implemented Baseline

Today the codebase concretely implements:

- the workspace split across `domain`, `app-runtime`, `services`, `platform-esp32s3`, and
  `ls027b7dh01`
- an embassy-based runtime handoff between the platform loop and an `app_task`
- typed input gestures for the rotary encoder
- inactivity-based deep sleep with button wake
- internal flash partitions `motif_state` and `motif_outbox`
- persisted settings hydration into the live store and platform sleep service
- skeletal provisioning interfaces and BLE-first onboarding documentation

The codebase does not yet implement the full queue, reader, Wi-Fi, backend sync, or formatter
systems described elsewhere in these docs.

## Current Hardware References

- [Board Integration Contract](board-config.md)
- [LS027 Notes](ls027-notes.md)
