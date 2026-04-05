# Article Lifecycle

This document traces one article through the current Motif firmware and calls
out where the implementation already matches the target architecture versus
where it is still incomplete.

## Overview

The product still has two first-class upstream source classes:

- personal saved articles
- editorial feed articles

The current firmware is much further along on the Saved path than on the full
editorial-feed path, but the local package and reader pipeline is shared.

## Phase 1: Discovery

Today discovery happens through backend sync after refresh-session success.

Current implemented behavior:

- backend sync waits for backend-path readiness from the Wi-Fi module
- it refreshes the access session
- it fetches Saved content metadata in bounded pages
- it publishes collection updates into the store

The important correctness change already landed: discovery is no longer blocked
by one oversized buffered Saved response.

## Phase 2: Package Acquisition

For articles that are missing locally, backend sync requests a normalized
package from the backend.

Current implemented behavior:

- uncached opens trigger a streaming package request
- the store can queue that open if the backend path is not yet usable
- package bytes stream through the storage module instead of going directly to
  the reader
- retries coordinate with backend-path recovery rather than burning all attempts
  blindly on a marginal link

This path is now materially faster than the original baseline, but it still
fails sometimes when DNS, Wi-Fi, or TLS churn.

## Phase 3: Local Materialization

The storage layer now performs real staging and commit work on SD.

Current implemented behavior:

- package chunks are staged through the storage queue
- the active stage file stays open during the download
- free-slot downloads can write directly to the final package slot
- successful uncached opens can commit and open immediately
- failed prepares abort the stage instead of leaving partial packages live

This means package presence and reader open are now much closer together in the
uncached-open path than they were originally.

## Phase 4: Reader Materialization

Once the package is committed:

- storage opens the cached reader package
- paragraph metadata and the initial reader window are materialized
- the store opens the reader session and transitions the UI to Reader

Current reader/storage working sets now prefer PSRAM where safe, which reduces
internal-memory pressure during open and paging.

## Phase 5: Queue Presentation

The Saved view is derived from store-backed collection state.

Current implemented behavior:

- metadata can appear before the package is local
- package state is surfaced per item
- uncached taps while the backend is not usable are now queued instead of being
  dropped
- a queued item stays visibly `Fetching` across collection refreshes

This is important because the device can spend several seconds recovering from
network instability, and the queue now reflects that work instead of hiding it.

## Phase 6: Active Reading Session

When the user opens an article:

- the reader slice creates or restores a session
- the app runtime renders the reader surface from the reader model
- later paging can trigger additional reader-window loads from storage

Reading remains offline-capable once the package is local.

## Phase 7: Sleep And Long Operations

The current firmware now suppresses inactivity sleep during active fetching so
the device does not deep-sleep in the middle of an uncached package open.

That safeguard now depends on both:

- the fetching state being visible in the collection UI
- the pending prepare staying selected across collection refreshes

This is now part of the real lifecycle for long-running uncached opens.

## Phase 8: Progress And Sync

This area is still incomplete.

The current firmware can open and read backend-provided packages, but it still
does not complete the full remote progress-upload story described in the target
architecture.

## Failure Handling

### No Wi-Fi Or No Backend Path

- cached reading still works
- Saved metadata can remain visible from prior sync
- uncached opens may queue until the backend path becomes usable
- if recovery fails for long enough, the open eventually fails cleanly

### Package Download Failure

- the article remains listed in the queue
- the stage is aborted rather than partially committed
- package state leaves `Fetching` and the user can try again later

### DNS / TLS / Transport Failure

This is now the dominant remaining failure class.

Current package opens can still fail because of:

- DNS failure on device
- TLS handshake timeout
- request flush or body-read instability
- Wi-Fi disconnect/reassociate churn

This is the main reliability frontier now that storage throughput is no longer
the first bottleneck.

### SD Unavailable

- content availability degrades
- internal flash state remains intact
- queue selectors should still reflect that local package access is degraded

## Architectural Consequence

The same boundary rule still holds:

- Wi-Fi owns connectivity evidence
- backend sync owns remote protocol and retry policy
- storage owns staging, commit, and local package opening
- the store coordinates user-visible state
- app/UI consumes selectors and should not reconstruct the pipeline itself
