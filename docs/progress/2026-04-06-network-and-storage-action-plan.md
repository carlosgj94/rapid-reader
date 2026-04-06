# Session Plan: Network And Storage Stability

This is the execution plan for the April 6, 2026 session.

## Goal

Move the current firmware from "fast on healthy runs" to "predictably reliable
under normal Wi-Fi and SD-card churn" without giving back the throughput gains
already landed on April 5.

## Current Baseline To Preserve

- PSRAM enabled in octal mode
- internal heap held at `160 KiB`
- backend TCP buffers at `16 KiB` RX / `4 KiB` TX
- package and stage chunk sizes at `8 KiB`
- SD SPI at `400 kHz` init and `8 MHz` runtime
- package fetch retry budget at `3` attempts

These settings are the current reference point. Do not change multiple knobs at
once unless a ticket explicitly says to.

## What Is Already Good

- Saved sync is no longer blocked by oversized buffered responses.
- Healthy uncached package fetches are much faster than the original baseline.
- Direct package-slot commit and open-after-commit are working.
- PSRAM is now used for request buffers and selected reader/storage working
  sets.

## What Is Still Failing

- intermittent Wi-Fi disconnect / reassociation churn
- on-device DNS failure
- occasional TLS handshake timeout
- occasional request flush or stream-read instability
- SD stability has improved, but the current `8 MHz` runtime clock still needs
  a deliberate stability and integrity pass before pushing it higher

## Execution Order For Today

1. [Ticket 13: Wi-Fi Driver Config And Observability](2026-04-06-network-and-storage-tickets.md#ticket-13-wi-fi-driver-config-and-observability)
2. [Ticket 11: DNS And Endpoint Recovery Hardening](2026-04-06-network-and-storage-tickets.md#ticket-11-dns-and-endpoint-recovery-hardening)
3. [Ticket 12: Timeout And Request-Class Policy](2026-04-06-network-and-storage-tickets.md#ticket-12-timeout-and-request-class-policy)
4. [Ticket 15: SD Runtime Stability And Clock Validation](2026-04-06-network-and-storage-tickets.md#ticket-15-sd-runtime-stability-and-clock-validation)
5. [Ticket 14: Network Soak And SLI Reporting](2026-04-06-network-and-storage-tickets.md#ticket-14-network-soak-and-sli-reporting)

## End-Of-Day Success Criteria

- no hidden Wi-Fi driver defaults remain for power-save mode or country code
- DNS failure no longer kills article opens as easily
- timeout behavior is explicit per request class
- SD stage/commit/open flows are validated repeatedly at the retained runtime
  clock, and any higher-clock experiment is justified by data
- reliability is summarized with repeatable metrics instead of one-off log
  anecdotes
