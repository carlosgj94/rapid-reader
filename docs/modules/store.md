# Store

## Purpose

The store is the authoritative runtime state container for everything the user can observe or that
other modules need to reason about coherently.

It exists to provide:

- one place to derive UI read models from
- one mutation path for domain state
- predictable coordination between app, sync, storage, and connectivity

It does not exist to hold every object in the process.

## What The Store Owns

The target slice layout is:

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

## Current Implemented Subset

The current `Store` implementation is intentionally much smaller than the target slice map.

Today it concretely owns:

- `device`
  boot origin and pairing state
- `input`
  last delivered gesture, delivered sequence, and reserved space for dropped-gesture diagnostics
- `reader`
  active article session, formatter output, playback mode, progress, and paragraph anchors
- `settings`
  live user preferences including timeout, reading speed, appearance, and topic preferences
- `sleep`
  inactivity timeout, last activity, wake reason, sleep state
- `storage`
  mount and free-space health mirror

The current mutation surface is still smaller than the target architecture, but no longer only a
bootstrap skeleton:

- `dispatch(Command::RequestDeepSleep)` requests deep sleep
- `dispatch(Command::Ui(...))` routes encoder gestures through the current screen state
- settings mutations emit typed persistence effects instead of writing to storage directly
- `handle_event(Event::InputGestureReceived(...), now_ms)` records the gesture and mirrors
  activity
- `handle_event(Event::ReaderTick(...), now_ms)` advances timed RSVP playback
- `Store::from_bootstrap(...)` hydrates device boot state, persisted settings, sleep, and storage
  health before normal events begin
- `handle_event(Event::WokeFromDeepSleep, now_ms)` updates wake state

At startup, the hydrated settings are also logged once so device logs show the effective live
configuration after defaults and persisted values have been resolved.

This is still not the finished application state model, but it now includes a real reader,
settings persistence effects, and selector-driven UI state.

## What The Store Must Not Own

The following stay outside the store:

- sockets and HTTP clients
- Wi-Fi driver handles
- flash and SD file handles
- framebuffer caches
- renderer glyph caches and layout caches
- parser scratch buffers
- timer objects and scheduler internals

Those belong to services, the platform layer, or the renderer.

## Mutation Model

The recommended write path is:

1. a module or platform adapter dispatches a `Command`
2. the store routes the command to the owning slice logic
3. reducers mutate slice state
4. reducers emit zero or more `Effect` values
5. services execute effects
6. services publish typed `Event` values back into the store
7. selectors derive updated read models

This model gives the benefits of a central store without introducing arbitrary cross-module writes.

## Key Runtime Types

- `Command`
  A request to change domain state or trigger work.
- `Event`
  A fact that something already happened.
- `Effect`
  Work to be performed outside the store.
- `Selector`
  A read model builder that converts authoritative state into module- or UI-specific views.

## Ownership Rules

- Each slice owns its invariants.
- A slice may read other slices when deriving behavior, but it should not directly mutate them.
- Cross-slice coordination should happen through commands, events, and selectors.
- Services never mutate state directly. They only feed the store through typed events.

## Why This Is Better Than A Raw Singleton

A raw singleton makes every module globally reachable and globally mutable. That becomes fragile
quickly in Rust and becomes even harder to review when the code is heavily automated.

This architecture keeps the user-visible advantages of a single source of truth while imposing:

- explicit ownership
- typed write paths
- deterministic testing
- narrow review surfaces

## Local State Exceptions

Some state should remain local and not be promoted into the store:

- component-local ephemeral animation state
- per-frame renderer bookkeeping
- parser scratch state
- transport retry internals that do not matter outside their service

The rule is simple: if other modules need to reason about it, persist it, or render it, it likely
belongs in the store. If it is purely internal machinery, it likely does not.

## Selector Strategy

Selectors should produce stable read models such as:

- `QueueScreenModel`
- `ReaderScreenModel`
- `SettingsScreenModel`
- `ConnectivityBadgeModel`
- `SyncStatusModel`
- `StorageHealthModel`

The app runtime should prefer consuming selectors instead of re-deriving UI state inside
components.

## Testing Consequences

The store design should allow:

- reducer tests for each slice
- scenario tests that replay commands and events
- deterministic selector tests
- service contract tests with mocked effect execution

The current store is already small enough to support deterministic tests around input delivery,
sleep state transitions, and deep-sleep requests without requiring hardware.
