# Docs Index

Current-state documentation for the firmware in this repository.

These notes describe what the device does today and how the current codebase is divided. Future
features should get their own notes when they exist.

## Read This First

- [`../README.md`](../README.md): quick project entrypoint
- [`behavior/boot-library.md`](behavior/boot-library.md): cold boot, wake boot, and library flow
- [`behavior/rsvp-reading.md`](behavior/rsvp-reading.md): countdown, reading loop, pause, and WPM
- [`behavior/navigation.md`](behavior/navigation.md): chapter and paragraph selection flow
- [`behavior/runtime-behaviors.md`](behavior/runtime-behaviors.md): persistence, resume, sleep,
  and visible edge cases

## Architecture And Ownership

- [`architecture/ownership.md`](architecture/ownership.md): crate boundaries and ownership rules
- [`architecture/flow-map.md`](architecture/flow-map.md): where each user flow lives in code
- [`pipeline-map.md`](pipeline-map.md): low-level SD, EPUB, storage, and UI file map

## Hardware References

- [`board-config.md`](board-config.md): current firmware wiring and integration contract
- [`ls027-notes.md`](ls027-notes.md): Sharp LS027 protocol and bring-up notes

## Review And Cleanup

- [`review/cleanup-spec.md`](review/cleanup-spec.md): prioritized structural cleanup plan

## Intent

- Small notes over one large spec.
- Current behavior over future roadmap.
- Code ownership and flow tracing over narrative prose.
- Enough detail for a future engineer or agent to modify the firmware without rediscovering the
  architecture from scratch.
