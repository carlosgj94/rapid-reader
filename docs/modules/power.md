# Power Placeholder

## Status

Battery telemetry and related hardware integration are not currently available on the PCB path, so
this module is a placeholder for architectural completeness rather than an implementation target.

## Why This Module Still Exists In The Design

Power behavior affects product architecture even before the hardware is ready.

Future concerns include:

- battery level visibility
- charging state
- low-power policy that can inform other modules
- background sync limits under low power

If this subsystem is not named early, power behavior tends to leak into unrelated modules later.

## Intended Responsibilities

When the hardware exists, the power module should own:

- battery and charging telemetry
- power-policy decisions
- coordination with the store on device health and availability

## Non-Responsibilities

The power module should not own:

- article sync semantics
- queue derivation
- formatting decisions
- reader UI structure

It may influence those systems through typed events and state, but should not absorb their logic.

## Current Architectural Contract

For now, the architecture should only assume that a future `PowerService` will be able to report:

- battery or power status
- charging status
- fault or degraded-power states

The rest of the system should keep that integration point open without inventing implementation
detail prematurely.
