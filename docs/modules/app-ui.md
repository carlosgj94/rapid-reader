# App And UI Runtime

## Purpose

The app runtime owns the screens the user experiences. It should feel closer to a component system
than to a collection of full-screen drawing functions.

The target is a modular, composable UI architecture that is expressive enough for motion and
transitions but still disciplined enough for a 1-bit memory LCD.

## Core Surfaces

The first-class app surfaces are:

- `Queue/Library`
  The entry surface for personal saved articles and editorial feed items.
- `Reader`
  The RSVP reading surface.
- `Settings/Sync`
  The place for pairing, connectivity state, sync state, storage health, and device settings.

Article detail or diagnostics can be added later, but should not drive the primary architecture.

## Current Implemented Subset

The current `app-runtime` crate is deliberately small.

Today it owns:

- `Screen`
- `NavigationState`
- placeholder screen models
- `AppRuntime::handle_input_gesture(...)`

What it does not yet own is the actual component tree, selector-driven view preparation, or motion
system described below. `handle_input_gesture(...)` is currently a no-op placeholder, and `tick()`
returns the active `Screen` only.

## Component Model

The app runtime should be built from components, not from copy-pasted screens.

Each component should have:

- stable identity
- explicit props
- optional local ephemeral state
- layout output
- animation hooks
- children only when composition makes sense

Examples of likely component categories are:

- shells and layout containers
- queue list and queue cards
- reader presentation components
- settings sections and status rows
- badges, tabs, controls, overlays, and sheets

## Render Pipeline

The intended pipeline is:

1. selectors create screen view models from the store
2. components turn those view models into a component tree
3. the runtime resolves layout into a render tree or scene tree
4. the renderer consumes that scene plus invalidation hints
5. the display adapter paints only the work that matters

The renderer should stay an output stage. It should not reconstruct app logic on its own.

## Motion Model

Motion is a first-class design tool in this architecture, but it must be appropriate to the panel.

The right mental model is:

- expressive transitions
- small, bounded geometry changes
- predictable state-driven motion
- no visual tricks that only work on alpha-blended displays

Good motion candidates include:

- slide and anchored repositioning
- card expansion and collapse
- reveal and wipe transitions
- inversion-based emphasis or confirmation pulses
- focus movement that follows the active element

Poor fits for this display include:

- transparency fades
- blur-heavy effects
- long-running continuous decorative motion
- transitions that require repainting large areas without a clear UX payoff

## Animation Ownership

Animations should be described by `AnimationDescriptor` values tied to state transitions.

That means:

- components describe what changed
- the runtime decides what animation applies
- the renderer executes the transition using display-friendly primitives

Animation state should not be hidden in arbitrary widget code or buried inside hardware render
calls.

## Snappy UI Principles

To keep the UI feeling immediate:

- attach motion to user intent and state change, not decoration
- prefer transitions that can begin from already-known layout
- reuse layout and render caches where possible
- avoid full-screen invalidation unless the screen actually changed wholesale
- if a transition would delay real content visibility, prefer showing content first

## What Belongs In The App Runtime

The app runtime owns:

- navigation
- component composition
- focus rules
- screen-local ephemeral state
- animation descriptors
- UI read model consumption

The app runtime does not own:

- content parsing
- network logic
- persistence policy
- Wi-Fi connection machinery
- low-level draw operations

## Reader-Specific Requirements

The reader surface is the most performance-sensitive part of the system. It should:

- consume formatter outputs instead of raw article content
- keep reader controls modular and independently reviewable
- maintain stable layout around the active RSVP presentation region
- isolate speed changes, pauses, and session updates from unrelated UI work

## Queue-Specific Requirements

The queue surface should present personal and editorial sources through one coherent model. It
should not feel like two apps stitched together.

That requires:

- unified card or row primitives
- source-aware but source-agnostic navigation
- consistent availability and sync status indicators
- store-derived sorting and grouping rather than ad hoc service queries

## Settings And Sync Surface

Settings should be treated as a real product surface, not as a debug dump.

It should own:

- pairing state
- Wi-Fi status
- last sync status
- storage health
- reader preferences
- future device and power settings
