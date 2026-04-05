# Session Progress: RSVP, Theme, And Persistence

This note summarizes the concrete implementation progress completed in the April 1, 2026 session.

Some "still missing" items listed here were addressed in later sessions. See
[Session Progress: Network Reliability And Throughput](2026-04-05-network-reliability-and-throughput.md)
for the newer backend/content pipeline status.

## Implemented

- Replaced the static RSVP placeholder with a real mock-backed reading engine driven from the
  store.
- Added a formatter path that turns built-in script paragraphs into a `ReadingDocument` with:
  - RSVP reading units
  - paragraph anchors
  - stage split data
  - token dwell metadata
- Added contraction-safe token handling so words such as `there's` and `I'd` stay whole.
- Added ORP-style anchor placement and stable stage rendering around a fixed center line.
- Added a dedicated reader tick so RSVP playback advances on time instead of depending on UI
  transition cadence.
- Added live reader progression, pause, paragraph navigation, and resume behavior through the
  store.
- Added active-reader sleep suppression so the device stays awake only during live RSVP playback.
- Tightened the stage split gap so the left and right token segments sit closer together.
- Made the renderer theme-aware and fixed the pause modal to invert against the active theme.

## Settings And Persistence

- Expanded `PersistedSettings` beyond inactivity timeout to also include:
  - reading speed
  - appearance
  - topic preferences
- Added typed `Effect::PersistSettings(...)` so the store requests persistence without writing
  flash directly.
- Wired the platform task to execute that effect through the storage module.
- Preserved backward compatibility with the legacy timeout-only settings payload.
- Confirmed boot hydration still restores settings into the live store and now restores appearance
  as part of the same path.

## Current Product-Level Result

The current firmware now has:

- a usable embedded RSVP reading surface
- timed word progression with punctuation-aware pacing
- pause and paragraph navigation flows
- persisted reader preferences across reboot and deep sleep
- a renderer that follows the persisted light/dark theme

## Still Missing

- backend-supplied article packages flowing through the formatter
- persisted local reading-progress records
- backend progress upload
- final serif typography and final visual tuning
- a dedicated async storage task

## Validation Run

The session changes were validated with:

- `cargo fmt --all`
- `cargo check --offline`
- `cargo clippy --offline --workspace --lib`

Workspace test execution is still constrained by the embedded target and local toolchain setup, so
the repo-level `cargo test --offline --workspace --lib` path remains unavailable in this
environment.
