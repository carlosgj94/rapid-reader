# Sleep

## Purpose

The sleep module owns inactivity tracking and deep-sleep entry policy.

It is separate from battery and charging because sleep is required now, while battery telemetry is
not yet wired into the PCB path.

## Current Implementation Boundary

The current implementation uses:

- a singleton-like sleep model that other modules can talk to
- a default inactivity timeout of 30 seconds
- ESP32-S3 deep sleep as the actual sleep mechanism
- `GPIO12` as the provisional wake button input because it is the encoder switch pin and an RTC
  capable GPIO
- a display low-power transition that clears the panel, drives `DISP` and `EMD` low, and holds
  those RTC pins low through deep sleep
- embassy timers in the platform runtime rather than a blocking delay loop

## Intended Behavior

- the device tracks the last activity time
- if no activity occurs for the configured timeout, it enters deep sleep
- wake is expected from a button-style external signal
- the inactivity timeout is currently driven by the hydrated settings model, with a default of
  `30_000 ms` when no settings record exists

More concretely, the current runtime behaves as follows:

- accepted input gestures reset the inactivity timer
- raw button edges do not reset inactivity unless they become a valid delivered gesture
- the wake button is the only deep-sleep wake source
- the wake press is consumed until release so waking does not emit a synthetic click
- app code may request sleep, but the platform layer remains authoritative for actually entering
  deep sleep
- the current timeout can now be hydrated from persisted settings during boot
- active RSVP playback suppresses inactivity sleep while the live reader stage is running
- paused RSVP and paragraph navigation remain eligible for normal inactivity sleep behavior

## Platform Notes

The implementation is built on the `esp-hal` RTC deep-sleep path:

- `Rtc::sleep_deep(&[...])`
- an RTC wake source configured on the wake button pin
- an `EXT0` low-level wake on `GPIO12`
- RTC pull-up enabled on the wake pin before sleep

## Runtime Integration

The current sleep path is split across three layers:

- `domain::sleep`
  platform-agnostic sleep model and timeout tracking
- `services::sleep`
  service boundary for sleep operations
- `platform-esp32s3::sleep`
  real ESP32-S3 deep-sleep entry

The platform runtime computes inactivity deadlines with embassy time and enters deep sleep from the
platform side, not from the store.
The live store mirrors the hydrated timeout and wake state, but deep sleep is still controlled by
the platform sleep service.
The platform deadline logic now also inspects the active prepared screen so only live reader
playback suppresses inactivity sleep.

## Display Interaction

Before deep sleep, the platform layer transitions the display into its lowest current software
state by:

1. clearing the panel
2. driving `DISP` low
3. driving `EMD` low
4. holding those RTC pins low through sleep

The last image may still appear faintly retained on a memory LCD, but that is not the same thing
as active display driving.

## Current Limitations

- the timeout still defaults to `30_000 ms` when no persisted settings record exists
- there is no dedicated wake button yet, so the encoder switch is shared with wake
- wake reason is currently coarse and modeled as external button versus cold boot
- battery telemetry does not participate in sleep policy yet

This keeps the service aligned with the real ESP32-S3 power path rather than a simulated sleep
state.
