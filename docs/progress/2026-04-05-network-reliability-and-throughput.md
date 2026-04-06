# Session Progress: Network Reliability And Throughput

This note summarizes the concrete networking, PSRAM, storage, and throughput
work completed in the April 5, 2026 session.

## Implemented

- Enabled PSRAM in octal mode and added it to the allocator.
- Added dual-heap telemetry so internal and external pressure are visible in
  logs and reports.
- Established an explicit memory-placement policy and moved safe request
  buffers plus selected reader/storage working sets into PSRAM.
- Replaced oversized Saved collection fetches with bounded pagination, fixing
  the earlier `ResponseTooLarge` failure mode on blank cards.
- Reworked package download/staging to remove the most obvious throughput
  bottlenecks:
  - larger package and stage chunks
  - bounded flush policy
  - async queued storage writes
  - open-handle staging
  - direct writes to free final package slots
  - immediate open-after-commit for uncached articles
- Raised the runtime SD SPI clock to `8 MHz` after mount.
- Added runtime TLS session resumption and backend-path readiness / reprobe
  gating.
- Moved large long-lived async task state out of embassy task storage and grew
  the stable internal heap budget to `160 KiB`.
- Added queueing for uncached article opens when the backend is temporarily
  unavailable, plus sleep suppression while a fetch is active.

## Measured Impact

Healthy package and open timings are dramatically better than the early April 5
baseline.

Representative request improvements:

- early baseline:
  - `88199` byte package in `25440 ms`
  - `219227` byte package in `25440 ms`
  - `219227` byte commit in `32512 ms`
- later healthy run:
  - `54067` byte package in `4476 ms`
  - `140395` byte package in `10002 ms`
  - `219227` byte package in `13702 ms`
  - `219227` byte commit in `15132 ms`

This means package throughput and SD staging are no longer the primary limiter
when the network path is healthy.

## Current Product-Level Result

The firmware now has:

- real backend-driven Saved sync
- real uncached article download and local open
- much faster package and commit paths
- materially better internal-memory headroom during reader open
- retry/recovery behavior that can survive some DNS and transport wobble

Recent healthy runs can open uncached articles in the rough range of:

- `54 KB` article: about `4.5 s` package request
- `140 KB` article: about `10.0 s` package request
- `219 KB` article: about `13.7 s` package request

## Still Failing

The remaining reliability issues are now concentrated in the network stack:

- Wi-Fi disconnect/reassociate churn
- on-device DNS failure
- occasional TLS handshake timeout
- occasional request flush / stream-read instability

These failures now dominate user-visible problems more than raw SD or package
throughput does.

## What To Do Next

The next work should prioritize reliability over more throughput tuning:

1. DNS and endpoint-recovery hardening
2. request-class specific timeout policy
3. explicit Wi-Fi driver configuration and observability
4. soak metrics and service-level reliability reporting

That is the work needed to move the current networking path from a capable
prototype to a more production-grade embedded client.

The follow-up execution backlog is documented in
[Session Plan: Network And Storage Stability](2026-04-06-network-and-storage-action-plan.md)
and
[Network And Storage Tickets](2026-04-06-network-and-storage-tickets.md).
