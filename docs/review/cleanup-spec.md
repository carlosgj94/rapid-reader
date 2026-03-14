# Cleanup Spec

This is the current structural cleanup plan for the codebase. It is not an implementation log and
it does not authorize feature work.

## Goal

Keep current behavior unchanged while making the codebase smaller in responsibility per file,
easier to navigate, and less prone to LLM-style overgrowth.

Guiding rules:

- Prefer fewer responsibilities per file.
- Prefer deletion over adding an abstraction that does not pay for itself.
- Keep `no_std` constraints and fixed-buffer patterns intact.
- Do not move business logic into board glue or render code.
- Split by semantic responsibility, not by arbitrary file-count targets.

## Priority 0: `src/bin/main.rs` Must Become An Orchestrator

Current problem:

- `main.rs` still owns board bring-up, Wi-Fi retry loops, ping loop, boot restore logic, local
  persistence policy, inactivity sleep policy, rendering flush policy, and several runtime
  constants.
- The file is readable only because pieces were already extracted into `src/bin/main/*`, but the
  remaining top-level still carries too many unrelated decisions.

Target state:

- `main.rs` keeps only top-level wiring and task assembly.
- Bootstrapping, network tasks, UI loop, and sleep/persistence policy each move behind named
  modules with explicit inputs and outputs.
- Constants move closer to the subsystem that owns them instead of accumulating in one file header.

Concrete cleanup targets:

- Extract Wi-Fi and ping loops out of `main.rs`.
- Extract the UI runtime loop that coordinates refill, render, settings sync, resume sync, and
  sleep.
- Move resume-sync helper types out of `main.rs`.
- Remove repeated loading-frame flush blocks behind a single helper so boot code stops duplicating
  the same display error path.

Acceptance bar:

- `main.rs` reads like composition, not policy implementation.

## Priority 1: Replace The `readily-core::app` Include Shards With Real Modules

Current problem:

- `crates/readily-core/src/app/mod.rs` defines one large `ReaderApp` type and spreads its behavior
  through `include!` files.
- `view.rs` mixes screen projection with resume import/export, sleep policy, and helper logic.
- `input.rs`, `runtime.rs`, and `navigation.rs` all mutate the same wide state surface.

Target state:

- Keep `ReaderApp` as the facade, but move implementation into real modules with explicit
  boundaries.
- Separate screen projection from persistence/resume handling.
- Separate transition entry helpers from input-event dispatch.

Concrete cleanup targets:

- Replace `include!` with normal modules.
- Split current `view.rs` into at least:
  - screen projection
  - sleep and resume import/export
  - shared content-derived helpers
- Split input handling by UI domain or transition family so library/settings/reading/navigation do
  not live in one long file.
- Keep shared word-state reset and entry helpers centralized to delete duplicated state-reset code.

Acceptance bar:

- A reader can find "how reading advances", "how navigation works", and "how wake restore works"
  without opening one giant mixed-responsibility file.

## Priority 2: Shrink The Renderer Namespace

Current problem:

- `crates/readily-hal-esp32s3/src/render/rsvp/mod.rs` owns renderer state, loading rendering,
  connectivity state, cover thumbnail cache, screen dispatch, constants, and wildcard imports.
- `library.rs` and `navigation.rs` still each mix layout, decoration, and reusable widget helpers.

Target state:

- One renderer facade, smaller focused submodules underneath it.
- Shared render primitives stay shared, but screen-specific helpers stop leaking across the whole
  module namespace.

Concrete cleanup targets:

- Split renderer state from cover thumbnail cache.
- Remove broad `use self::{...::*}` imports in favor of explicit imports.
- Keep screen composition files focused on one screen family each.
- Move generic card, icon, and selector helpers into clearly named shared modules if more than one
  screen uses them; otherwise keep them local to the owning screen file.

Acceptance bar:

- A future change to the library shelf or navigation selector should not require scanning a broad
  renderer namespace to discover hidden helper coupling.

## Priority 3: Keep Documentation Drift Out Of The Runtime

Current problem:

- Top-level docs had drifted away from the current firmware shape.
- Older notes mixed bring-up history with current behavior.

Target state:

- `docs/index.md` is the navigation hub.
- Behavior notes describe current behavior only.
- Architecture notes describe current ownership only.
- Reference notes stay clearly marked as low-level support material.

Concrete cleanup targets:

- Treat docs drift as a maintenance bug.
- Update docs whenever flow ownership or hardware wiring changes.
- Avoid reintroducing large README-level duplication when the small notes already carry the detail.

## Secondary Review Targets

These are real targets, but they should come after the three priorities above:

- Revisit `crates/readily-core/src/content/sd_catalog.rs` to see whether fallback catalog defaults,
  stream bookkeeping, and parser state need clearer separation.
- Review `crates/readily-hal-esp32s3/src/storage/sd_spi/*` only when ownership leakage into app or
  glue code requires it. That area is large, but it is already more decomposed than `main.rs` and
  `readily-core::app`.

## Non-Negotiable Constraints For The Cleanup Pass

- No feature additions.
- No behavior changes to the documented flows.
- Preserve `no_std`.
- Preserve fixed-buffer hot paths.
- Keep the crate boundary between core and HAL strict.
