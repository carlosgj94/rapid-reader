# Article Lifecycle

This document traces one article through the target system from backend discovery to local reading
and eventual progress sync.

## Overview

The product has two first-class upstream source classes:

- personal saved articles
- editorial feed articles

Both should flow through the same local pipeline after sync. Differences in source origin must not
create separate reader implementations.

## Phase 1: Discovery

The backend sync module refreshes source manifests when:

- the device completes pairing
- Wi-Fi becomes usable after being unavailable
- the user requests manual refresh
- a scheduled refresh window opens

The result of discovery is metadata and availability information, not yet a rendered article.

At this stage the store should learn:

- source membership
- article identity and revision
- ordering, recency, and source grouping
- download status
- whether a newer revision supersedes cached local content

## Phase 2: Package Acquisition

For articles that are missing or stale, backend sync requests a normalized article package from the
backend.

That package should be:

- source-agnostic
- stable enough to cache locally
- structured enough for the formatter to produce reading units without reparsing raw HTML

The package is written through the storage module, not directly by sync.

## Phase 3: Local Materialization

Once an article package lands locally:

- storage records availability and revision information
- the content slice reflects that the article can be opened offline
- formatter work can run immediately or lazily, depending on policy

The system should separate package presence from formatter readiness. An article can be downloaded
before it is fully prepared for reading.

## Phase 4: Formatting

The formatter consumes the normalized article package and produces a reading-oriented representation.

The output should include:

- RSVP-ready reading units
- stable anchors for resume and navigation
- derived metadata needed by the reader UI
- any formatter warnings or degradation markers

Formatter output should be cacheable so active reading does not repeatedly redo the same work.

## Phase 5: Queue Presentation

The queue or library surface is derived from selectors over the store. It should combine:

- source grouping
- freshness state
- local availability
- reading progress
- pinning or saved-state metadata

The app should not separately query services to build the queue screen. It should read a prepared
view model from selectors.

## Phase 6: Active Reading Session

When the user opens an article:

- the reader slice creates or restores a session
- the app runtime renders the reader surface from a reader view model
- the formatter output feeds the RSVP presentation logic
- progress updates are persisted locally as the user advances

Reading must continue without Wi-Fi. Network state may affect badges or deferred sync, but should
not stall the reading session.

## Phase 7: Local Progress Persistence

Local persistence should happen before remote sync is attempted.

The local progress record should be sufficient to restore:

- current article identity
- current position anchor
- reader speed and session context where appropriate
- last-read timestamps and completion markers

The device should always prefer local continuity over waiting for backend confirmation.

## Phase 8: Progress And State Sync

When network and backend state allow it, backend sync uploads:

- read progress
- completion or dismissal state
- source-level refresh acknowledgements if needed

The system should tolerate delayed progress upload. Local truth should remain usable while remote
reconciliation is pending.

## Refresh And Revision Handling

If a backend refresh provides a newer article revision:

- the new package should be staged before it replaces the old one
- existing local progress should be preserved where anchors remain valid
- if anchors become incompatible, the reader should fall back to the closest safe restore point

The user experience should favor predictable resume behavior over aggressive replacement.

## Failure Handling

### No Wi-Fi

- queue and reader continue from local data
- refresh jobs remain pending
- progress stays queued for later upload

### Package Download Failure

- manifest metadata may still appear in the queue
- the article should be marked unavailable offline until package acquisition succeeds

### Formatter Failure

- the article remains present in storage
- the UI should surface that the content could not be prepared for reading
- failure state should not corrupt unrelated articles or queue state

### SD Unavailable

- storage should mark content availability as degraded
- queue selectors should reflect the missing local package state
- settings and device state in internal storage should remain intact

## Architectural Consequence

This lifecycle requires four boundaries to stay explicit:

- backend sync owns remote protocol
- storage owns persistence
- formatter owns reading preparation
- app/UI runtime consumes selectors and never reconstructs the pipeline itself

