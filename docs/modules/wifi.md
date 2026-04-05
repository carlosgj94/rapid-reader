# Wi-Fi

## Purpose

The Wi-Fi module owns radio-level connectivity and the local evidence for
"can the device actually reach the backend right now?"

It answers:

- is the station started
- is the device associated
- does the device have an IP address
- has backend-path reachability been proven recently enough to trust requests
- are we offline, reconnecting, or degraded

It does not own auth tokens, manifest policy, article semantics, or reader
behavior.

## Responsibilities

- station start and reconnect lifecycle
- association and disconnect handling
- IP availability reporting
- backend-path probe execution and readiness tracking
- invalidating readiness on link, DNS, connect, TLS, or request-I/O failure
- publishing typed network status upward to the store/backend sync layer

## Non-Responsibilities

- onboarding and provisioning ownership
- backend refresh-token exchange
- collection pagination and package downloads
- reader open policy
- storage writes

Those belong to provisioning, backend sync, and storage.

## Current Implemented State

Wi-Fi is no longer a stub.

The current firmware now has a real station-mode connection task in
`crates/platform-esp32s3/src/internet.rs` with:

- station config and start
- reconnect after disconnect
- DHCP/IP-backed online state
- a probe task that confirms backend-path reachability with real DNS+connect
- a shared backend-path readiness bit that request code can wait on
- invalidation of that readiness when Wi-Fi, DNS, connect, TLS, or request I/O
  fails

This means the rest of the firmware no longer treats "associated with an IP"
as equivalent to "safe to spend backend retries."

## Current Configuration Reality

The current code still relies too much on driver defaults.

Important current facts:

- the platform constructs Wi-Fi with `esp_radio::wifi::new(..., Default::default())`
- the effective modem power-save mode is currently `None`
- that is true because the vendored `esp-radio` default resolves to
  `PowerSaveMode::None`, not because Motif sets it explicitly
- the same default config currently carries a default country code of `CN`
- client association still starts from `ClientConfig::default()`

So power save is probably not the reason for the present instability, but the
configuration is still too implicit for a production-quality networking stack.

## Known Gaps

Current reliability work is now concentrated here:

- repeated on-device DNS failures
- occasional Wi-Fi disconnect/reassociate churn
- limited observability for disconnect reasons and effective driver settings
- hidden dependency on library defaults for country code and other radio knobs
- no last-good-endpoint fallback yet when DNS is temporarily unhealthy

These are now higher priority than raw package-throughput tuning.

## Interaction With Backend Sync

Backend sync is allowed to depend on Wi-Fi's backend-path readiness signal, but
Wi-Fi still does not own higher-level retry policy.

The split is:

- Wi-Fi says whether the transport path is plausibly usable
- backend sync decides which request class to run, how many attempts to spend,
  and what to do when content fetches fail

That keeps radio/transport concerns separate from product semantics.

## Persistence Boundary

Wi-Fi credentials should still live in internal storage rather than inside
transient radio state. The firmware currently assumes one active network, and
future provisioning should continue to stage credentials before replacing the
active record.
