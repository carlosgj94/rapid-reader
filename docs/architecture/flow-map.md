# Flow Map

This note maps the current user-visible flows to the code that owns them.

## Boot To Library

Primary ownership:

- `src/bin/main.rs`: top-level boot sequence, hardware bring-up, settings restore, wake restore
- `src/bin/main/loading.rs`: loading phases and progress text
- `src/bin/main/resume_sync.rs`: resume metadata helpers shared by boot restore and runtime save
- `src/bin/main/initial_catalog.rs`: SD scan, first text preload, first cover preload
- `src/bin/main/book_db.rs`: SD-side manifest and resume cache
- `crates/readily-core/src/content/sd_catalog/*`: in-memory catalog state loaded by the boot path

## Library Interaction

Primary ownership:

- `crates/readily-core/src/app/input.rs`: library rotation and press behavior
- `crates/readily-core/src/app/view.rs`: library screen projection and visible item windowing
- `crates/readily-hal-esp32s3/src/render/rsvp/library.rs`: library shelf composition
- `crates/readily-hal-esp32s3/src/render/rsvp/header.rs`: top header for the library screen

## Countdown And Reading

Primary ownership:

- `crates/readily-core/src/app/navigation.rs`: state-entry helpers for countdown and reading
- `crates/readily-core/src/app/runtime.rs`: countdown tick, word advance, end-of-text behavior
- `crates/readily-core/src/app/input.rs`: press and rotate controls during countdown and reading
- `crates/readily-hal-esp32s3/src/render/rsvp/text.rs`: RSVP word rendering
- `crates/readily-hal-esp32s3/src/render/rsvp/countdown.rs`: countdown screen rendering

## Chapter And Paragraph Navigation

Primary ownership:

- `crates/readily-core/src/app/input.rs`: chapter and paragraph selector transitions
- `crates/readily-core/src/app/view.rs`: chapter/paragraph screen projection
- `crates/readily-core/src/app/navigation.rs`: applying confirmed paragraph jumps
- `crates/readily-core/src/content/sd_catalog/traits_navigation.rs`: chapter model and seek queue
- `src/bin/main/sd_refill.rs`: executes pending chunk refill and chapter seek requests
- `crates/readily-hal-esp32s3/src/render/rsvp/navigation.rs`: selector rendering

## Settings, Resume, And Sleep

Primary ownership:

- `crates/readily-core/src/settings.rs`: persisted settings and wake snapshot types
- `crates/readily-core/src/app/view.rs`: exported and imported resume/wake state
- `src/bin/main/settings_sync.rs`: debounced flash save for settings
- `src/bin/main/resume_sync.rs`: debounced/forced resume save policy
- `src/bin/main/ui_loop.rs`: runtime orchestration for render, persistence, inactivity sleep
- `src/bin/main/book_db.rs`: resume persistence in the SD-side DB
- `src/bin/main/power.rs`: deep sleep entry
- `src/bin/main.rs`: inactivity policy and restore precedence

## Rendering And Display Flush

Primary ownership:

- `crates/readily-hal-esp32s3/src/render/rsvp/mod.rs`: renderer state and screen dispatch
- `crates/readily-hal-esp32s3/src/render/rsvp/*.rs`: per-screen composition and shared primitives
- `crates/ls027b7dh01/src/*`: framebuffer and panel protocol
- `src/bin/main/loading.rs`: loading-screen render/flush helper
- `src/bin/main/ui_loop.rs`: flush scheduling and connectivity-driven redraw decisions

## Network Background Tasks

Primary ownership:

- `src/bin/main/network_runtime.rs`: Wi-Fi reconnect loop, DHCP wait, ping loop, connectivity state
- `src/bin/main.rs`: Wi-Fi peripheral bring-up and task assembly

## Low-Level Reference

For exact file references inside the SD and EPUB stack, see [`../pipeline-map.md`](../pipeline-map.md).
