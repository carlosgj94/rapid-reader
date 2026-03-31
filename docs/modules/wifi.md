# Wi-Fi

## Purpose

The Wi-Fi module owns connectivity transport and nothing above that.

Its job is to answer:

- is the radio enabled
- is the device associated to a network
- does the device have usable connectivity
- what retry or degraded state are we in

It does not own provisioning, backend pairing, article sync policy, or content semantics.

## Responsibilities

- radio enable and disable lifecycle
- association and reconnect attempts
- link and IP state reporting
- retry and backoff policy
- surfacing connectivity state upward to the rest of the system

## Non-Responsibilities

- phone-assisted onboarding
- backend token exchange
- content refresh scheduling
- article downloads
- reader behavior

Those belong to provisioning, backend sync, or higher layers.

## Current Status

Wi-Fi is intentionally still a stub in firmware.

What exists today:

- architecture docs for the module boundary
- a minimal `WifiService` trait in the services crate
- a no-op implementation used by the platform skeleton

What does not exist yet:

- real station-mode radio bring-up
- scanning
- association
- DHCP
- reconnect policy

## Planned State Model

The concrete enum can still evolve, but the architecture assumes explicit lifecycle states such as:

- `Disabled`
- `Idle`
- `Associating`
- `Connected`
- `Degraded`
- `Backoff`

That is preferable to scattered booleans once the real runtime lands.

## Interaction With Provisioning

Provisioning will eventually ask the Wi-Fi module to:

- scan nearby networks
- attempt association with staged credentials
- report whether connectivity is usable

That does not make Wi-Fi the provisioning owner. Provisioning owns onboarding; Wi-Fi remains a
transport module.

## Persistence Boundary

Wi-Fi credentials should live in internal storage, not inside transient radio state.

For v1, Motif should remember one active network only. The future provisioning flow should stage
candidate credentials first and only replace the live record after successful validation.
