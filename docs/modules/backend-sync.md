# Backend Sync

## Purpose

Backend sync is the device's bridge to remote product state.

It owns:

- refresh-token based session renewal
- saved/recommendation collection refresh
- normalized article package acquisition
- transport retry policy above the Wi-Fi layer
- publication of sync/content availability into the store

It depends on Wi-Fi/backend-path readiness, but it is not the Wi-Fi module.

## Current Implemented State

Backend sync is no longer a skeleton.

The current firmware in `crates/platform-esp32s3/src/backend.rs` now implements:

- refresh-session exchange against `/device/v1/auth/session/refresh`
- paginated Saved collection refresh with bounded page size
- streaming package fetch for uncached articles
- reusable access sessions plus serialized TLS session resumption
- package retry and recovery logic that coordinates with backend-path readiness
- direct handoff into storage staging/commit/open flows
- typed status publication back into the store

On a blank card, Saved can now repopulate and uncached article opens can
complete without relying on oversized buffered responses.

## Current Runtime Shape

At a high level the current flow is:

1. wait for backend-path readiness from the Wi-Fi layer
2. refresh the access session
3. fetch Saved metadata in bounded pages
4. publish collection updates into the store
5. on uncached open, stream the package through storage
6. commit and open the package locally

When the path is healthy, current package fetch timings are already
substantially improved from the original baseline. Recent successful runs show:

- `54067` byte package in about `4476 ms`
- `140395` byte package in about `10002 ms`
- `219227` byte package in about `13702 ms`

That means backend sync is no longer dominated by the old SD/package pipeline.

## Current Remaining Problems

The main remaining failures are network-side, not storage-side:

- DNS failure on device
- occasional TLS handshake timeout
- occasional request flush / body-read instability
- Wi-Fi disconnects that force package-recovery paths

When these occur, backend sync can still fail an uncached article open even
though the happy path is now fast.

## Pairing And Credential Model

The current implementation still assumes a stored refresh token / device-side
credential rather than a full login UI. Provisioning remains the source of that
credential; backend sync owns using and refreshing it.

## Storage Boundary

Backend sync still must not write raw files directly. It asks storage to:

- begin package staging
- append package chunks
- commit or abort staged content
- open committed reader content

That keeps persistence policy and recovery logic inside the storage boundary.

## Still Missing

The architecture still has important unfinished areas:

- progress upload and remote reconciliation
- explicit last-good-endpoint fallback when DNS fails
- request-class specific timeout policy cleanly documented and tuned
- compile-time TLS feature pruning
- automated long-run soak infrastructure beyond the current manual reports

Those are now the next frontier for turning the current implementation from a
working prototype into a more production-grade network client.
