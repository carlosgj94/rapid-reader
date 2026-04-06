# Network And Storage Tickets

These tickets are the durable execution backlog after the April 5 throughput
and memory work.

## Ticket 11: DNS And Endpoint Recovery Hardening

### Why

Current runs still show repeated on-device DNS failures during startup refresh
and uncached article opens. Throughput is good enough now that DNS instability
is a primary reliability problem.

### Scope

- add a last-good backend IPv4 cache in RAM
- key the cache to the current network session so disconnect or DHCP change
  invalidates it
- after DNS failure, allow refresh/package requests to retry against the cached
  endpoint while preserving hostname semantics for HTTP Host and TLS SNI
- log cache hit, miss, age, invalidation reason, and fallback success

### Current Progress

- Code landed: the Wi-Fi layer now holds a session-scoped last-good backend
  endpoint cache, probe fallback can use it within the same network session,
  and backend session open / startup refresh now try that cached endpoint
  before declaring DNS dead.
- Still pending: on-device validation that the fallback actually reduces user-
  visible DNS failures without masking real link problems.

### Non-goals

- no Wi-Fi driver tuning here
- no timeout redesign here
- no backend API changes

### Done When

- transient DNS failure no longer kills uncached opens so easily
- fallback behavior is visible in logs
- fallback success can be counted in later soak reports

## Ticket 12: Timeout And Request-Class Policy

### Why

The current firmware still shares too much timeout behavior across startup
refresh, buffered metadata fetches, and streaming package requests.

### Scope

- define explicit timeout budgets for:
  - startup refresh
  - buffered metadata pages
  - streaming package fetch
- make the policy visible in code and telemetry rather than hiding it in shared
  constants
- revisit handshake, body-read, and flush budgets using the April 5 and April 6
  traces as the baseline evidence
- add request-class tags to timeout and timing logs

### Current Progress

- Code landed: backend requests now carry an explicit request class
  (`auth_refresh`, `buffered_metadata`, `streaming_package`), timeout budgets
  are selected per class instead of through shared global constants, and the
  class is now emitted in request timing/phase/headers/progress/complete
  telemetry.
- On-device validation passed in `/tmp/motif-ticket12b.log`: startup reached
  `Ready`, no inactivity sleep fired during dashboard sync, and the run showed
  zero timeout-style transport failures across refresh, metadata, and package
  requests.
- Residual observation: one reused `streaming_package` session delivered a
  `57,123` byte package in about `20s`, while fresh/resumed streaming sessions
  for larger packages completed materially faster. That looks like a follow-up
  on package-session reuse policy, not a reason to keep Ticket 12 open.

### Non-goals

- no DNS fallback here
- no Wi-Fi driver default cleanup here
- no throughput tuning unless a timeout decision requires it

### Done When

- timeout behavior is explainable per request class
- startup and package paths stop sharing obviously mismatched budgets
- false-positive transport failures are measurably reduced

## Ticket 13: Wi-Fi Driver Config And Observability

### Why

The current Wi-Fi path still inherits important behavior from `esp-radio`
defaults. That hides useful facts such as the effective country code,
power-save mode, and driver-level disconnect context.

### Scope

- replace implicit `Default::default()` dependence with explicit Motif-owned
  Wi-Fi config where practical
- explicitly set and log the effective power-save mode
- explicitly set and log the effective country code instead of inheriting the
  vendored `CN` default silently
- log disconnect reason codes and other high-signal radio transitions
- log the effective Wi-Fi driver config at boot so field traces are
  self-describing

### Current Progress

- Code landed: Motif now sets Wi-Fi power-save mode explicitly, hardcodes the
  product-default country code to `ES`, logs the effective driver config at
  boot, and logs `StaConnected` / `StaDisconnected` events with disconnect
  reason and RSSI.
- Still pending: device validation of the new logs on hardware.

### Non-goals

- no speculative power-save enablement here
- no DNS fallback here
- no timeout-policy redesign here

### Done When

- logs show the effective Wi-Fi config without reading source code
- power-save mode and country code are explicit Motif decisions
- disconnects carry actionable reason data instead of only generic
  "disconnected"

## Ticket 14: Network Soak And SLI Reporting

### Why

The project now needs service-level reliability evidence, not just individual
trace reading.

### Scope

- extend reporting to summarize:
  - startup retry count
  - Wi-Fi disconnect count
  - DNS failure count
  - TLS handshake timeout count
  - flush/body-read timeout count
  - package success ratio
  - median and p95 request time by request class
  - SD mount/commit/open failure count
- define a repeatable soak-run protocol for manual sessions
- make the report output easy to compare across tickets

### Current Progress

- Code landed: `scripts/memtrace_report.py` now parses both `MEMTRACE` rows and
  high-signal plain log lines, emits `sli.csv`, `request-class-summary.csv`,
  and `log-events.csv`, and adds reliability / request-class sections to the
  Markdown summary.
- The report now summarizes startup retries, Wi-Fi disconnects, DNS fallback
  attempts and hard failures, TLS handshake timeouts, flush/body-read
  timeouts, package success ratio, per-class attempt/success counts, and
  median / p95 successful request latency by request class.
- Manual soak protocol captured in
  [2026-04-06-network-soak-protocol.md](2026-04-06-network-soak-protocol.md)
  so repeated runs use the same capture and report commands.

### Non-goals

- no firmware tuning here unless reporting requires a new event field
- no backend API changes

### Done When

- repeated runs produce comparable reliability summaries
- tickets can be judged by success rate, not only by feel
- the repo has a lightweight soak protocol the team can keep using

## Ticket 15: SD Runtime Stability And Clock Validation

### Why

The SD runtime clock jump to `8 MHz` delivered a real performance win, but it
still needs an explicit stability and integrity pass before being treated as a
finished hardware setting, and any move above `8 MHz` must be evidence-driven.

### Scope

- validate mount, stage, abort, commit, reopen, and post-sleep behavior at the
  current `8 MHz` runtime clock
- add or tighten SD-specific observability if the current logs are not enough
  to diagnose mount/reopen/CRC issues
- only after a clean repeated run set, do one controlled A/B test at a higher
  runtime clock such as `12 MHz`
- keep the higher clock only if it is stable and yields a real end-to-end win

### Current Progress

- Code landed: the SD runtime clock is now an explicit product-default policy
  instead of an unlabeled raw constant, storage static-inventory and boot mount
  telemetry now emit the init/runtime clock and source, and a controlled
  build-time override path (`MOTIF_SD_SPI_RUN_HZ`) exists for later `12 MHz`
  A/B runs.
- Still pending: repeated on-device stability runs at the retained `8 MHz`
  setting, then one evidence-driven higher-clock experiment only if `8 MHz`
  stays clean.

### Non-goals

- no package protocol redesign here
- no chunk-size changes here
- no storage-format changes here

### Done When

- repeated stage/commit/open cycles are stable at the retained clock
- any higher-clock experiment is justified by measured gain and zero integrity
  regressions
- SD runtime configuration is documented as a deliberate choice rather than a
  lucky value

## Follow-up Experiment: Package Transfer Chunk Tuning

### Why

Longer soak runs are now mostly reliable, but uncached article opens are still
slower than they should be. The current package path is stable enough that it
is reasonable to try a more aggressive transfer profile.

### Scope

- keep backend package download chunking and storage stage chunking aligned
- widen the chunk size materially beyond the original `8 KiB` baseline
- widen stage flush cadence with the chunk so storage does fewer forced flushes
- make the effective transfer profile explicit in boot and static-inventory
  logs
- keep an easy build-time override for A/B runs at even larger chunk sizes

### Current Progress

- Code landed: backend package streaming and storage staging now read their
  chunk size from one shared transfer-tuning policy instead of separate raw
  constants.
- The current product-default transfer profile is `65536` byte chunks with a
  derived `131072` byte stage flush interval. The initial `32768` byte pass was
  retained only long enough to compare against the `65536` byte override.
- A controlled build-time override path
  (`MOTIF_PACKAGE_TRANSFER_CHUNK_LEN`) exists for deliberate higher-chunk A/B
  runs without touching the code again.

### Non-goals

- no Wi-Fi retry redesign here
- no TLS/session reuse redesign here
- no storage format changes here

### Done When

- repeated article-download runs show a real end-to-end latency win
- higher chunks do not introduce alloc or corruption regressions
- the effective package transfer profile is obvious from the logs
