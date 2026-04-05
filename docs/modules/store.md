# Store

## Purpose

The store is the authoritative runtime state container for everything the user
can observe or that multiple modules need to reason about coherently.

It provides:

- one place to derive UI read models from
- one mutation path for domain state
- predictable coordination between app, sync, storage, and connectivity

It does not exist to hold every object in the process.

## Target Slice Layout

The longer-term architecture still targets these slices:

| Slice | Owns |
| --- | --- |
| `device` | pairing identity, firmware-facing device metadata, current time quality |
| `provisioning` | onboarding session state, claim flow status, staged apply status |
| `network` | Wi-Fi status, IP availability, connectivity health |
| `sync` | sync scheduling, in-flight jobs, last-sync state, backoff |
| `content` | source membership, article manifests, local availability, revision state |
| `reader` | active reading session, resume state, speed, completion markers |
| `settings` | user-configurable preferences and persistent app settings |
| `ui` | current screen, focus/navigation context, modal state, screen-local coordination |
| `storage` | storage health, index state, capacity summaries, availability flags |
| `input` | latest delivered gesture, gesture sequence, dropped-gesture diagnostics |
| `sleep` | inactivity timeout, last activity, wake reason, sleep state |
| `power` | placeholder for battery and charging state when hardware exists |

## Current Implemented State

The current `Store` in `crates/domain/src/store.rs` is no longer a tiny
bootstrap shell. It now concretely owns:

- `device`
- `content`
- `input`
- `network`
- `power`
- `reader`
- `settings`
- `sleep`
- `storage`
- `backend_sync`
- `ui`

It also carries one important orchestration helper that is intentionally not a
full slice: a `pending_prepare` request for uncached article opens.

## Current Behavior

Today the store does real cross-module coordination, including:

- recording network-status changes and backend-sync status changes
- queueing an uncached article open when the backend path is not yet usable
- preserving the queued item's `Fetching` state across collection refreshes
- auto-dispatching `PrepareContent(...)` once both network and backend sync are
  in a usable state
- restoring pending prepares if auth becomes invalid
- opening committed reader content and loading later reader windows
- keeping the fetching item selected so the UI reflects the active operation

This means an impatient uncached tap during startup wobble is no longer just
dropped on the floor.

## What The Store Must Not Own

The following still stay outside the store:

- sockets and HTTP clients
- Wi-Fi driver handles
- flash and SD file handles
- framebuffer caches
- renderer glyph caches and layout caches
- parser scratch buffers
- timer objects and scheduler internals

Those belong to services, the platform layer, or the renderer.

## Mutation Model

The current architecture still follows the same write path:

1. a module or platform adapter dispatches a `Command`
2. the store routes the command to the owning slice logic
3. reducers mutate slice state
4. reducers emit zero or more `Effect` values
5. services execute effects
6. services publish typed `Event` values back into the store
7. selectors derive updated read models

That model is now exercising real network/content/storage behavior instead of
only UI and settings.

## Local State Exceptions

These still should not be promoted into the store:

- transport retry internals that do not matter outside backend sync
- parser and formatter scratch buffers
- framebuffer or renderer frame-local bookkeeping
- low-level Wi-Fi driver state

The rule remains: if multiple modules must reason about it, persist it, or
render it, it probably belongs in the store. If it is just service machinery,
it does not.

## Current Missing Pieces

The store is still not the final product model. Important gaps remain:

- provisioning state is not yet first-class
- remote progress upload/reconciliation is not represented end to end
- some long-term content freshness and revision policy remains simplified
- deeper battery/power state is still placeholder-only

But the current store is already a real coordinator for network-aware article
access, not just a UI state bag.
