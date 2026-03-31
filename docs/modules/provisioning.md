# Provisioning

## Purpose

Provisioning owns first-time setup and explicit reprovisioning.

Its job is to move Motif from:

- no network
- no backend identity

to:

- one working Wi-Fi network
- one valid backend credential
- a standalone device that no longer depends on the phone

Provisioning is not the normal Wi-Fi runtime and it is not the steady-state backend sync engine.

## Chosen Direction

Motif should use a phone-assisted BLE provisioning flow.

The planned device-side shape is:

- custom BLE onboarding transport
- one provisioning session carrying both Wi-Fi credentials and backend pairing material
- one active remembered Wi-Fi network in v1
- BLE enabled only while the device is unprovisioned or explicitly re-entering provisioning mode
- standalone operation after setup completes

The phone app is a separate project. This repo only defines the device-side architecture and the
contract that app must speak.

## Current Status

Provisioning is intentionally still a skeleton in firmware.

What exists today:

- documentation for the intended BLE-first flow
- shared domain types for provisioning state and bundle shape
- a no-op provisioning service boundary in the services crate

What does not exist yet:

- BLE transport
- QR payload generation or rendering
- Wi-Fi credential exchange
- backend pairing exchange
- commit logic for onboarding records

## Connection Steps

The intended setup flow is:

1. Motif enters provisioning mode when it has no valid network/backend setup, or later when the
   user explicitly asks to reprovision.
2. Motif advertises over BLE and exposes only the onboarding transport.
3. Motif presents a claim payload that lets the phone identify the exact device.
4. The phone app connects over BLE and authenticates the onboarding session.
5. The phone app asks Motif to scan nearby Wi-Fi networks.
6. Motif returns the scan results over BLE.
7. The user picks a network and types the password on the phone.
8. The phone app sends one provisioning bundle containing:
   - selected SSID
   - Wi-Fi password
   - backend pairing token
9. Motif stages that bundle, validates Wi-Fi and backend pairing, and only then commits the new
   live configuration.
10. Motif turns BLE off and continues as a standalone device.

## Reprovisioning Rule

Reprovisioning follows the same flow with one critical rule:

- the last known-good configuration remains active until the replacement has fully succeeded

That prevents a failed reprovision attempt from bricking a working device.

## Module Boundary

Provisioning owns:

- session lifecycle
- claim/auth handshake
- onboarding progress state
- staged apply and commit rules

Provisioning does not own:

- steady-state Wi-Fi reconnect policy
- backend sync scheduling
- article downloads
- general UI navigation

## State Model

The planned provisioning state model is:

- `Disabled`
- `Unprovisioned`
- `Advertising`
- `SessionAuthenticating`
- `ScanningWifi`
- `AwaitingBundle`
- `ApplyingBundle`
- `ConnectingWifi`
- `ValidatingBackend`
- `Provisioned`
- `FailedRetryable`
- `FailedTerminal`

Those states already exist as shared domain vocabulary even though the implementation is still
stubbed.
