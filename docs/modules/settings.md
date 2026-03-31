# Settings

## Purpose

The settings module owns user-configurable device preferences that must be available as live runtime
state and also survive reboot and deep sleep.

It is separate from `device` state:

- settings are user preferences
- device state is firmware-facing identity and lifecycle state

That means pairing and boot origin do not belong here.

## Current Implementation Boundary

Today the settings path is real, but intentionally narrow.

The current implementation includes:

- a live `settings` slice in the store
- a persisted settings record in internal flash
- bootstrap hydration from flash into the live store
- platform sleep configuration derived from the hydrated settings
- one startup log of the effective loaded settings

The current settings schema contains only:

- `inactivity_timeout_ms`

The default remains `30_000 ms` when no persisted settings record exists.

## Runtime Ownership

The current source-of-truth split is:

- `Store.settings`
  The live authoritative settings used by the app runtime.
- internal flash persisted settings record
  The durable copy used to restore settings on boot.

The live store is the value other modules should observe during runtime.
Flash is a persistence layer, not the live state owner.

## Boot and Deep-Sleep Behavior

Deep sleep is treated as a fresh boot, not as resumed in-memory application state.

On boot, the current runtime:

1. mounts internal storage
2. reads the persisted settings record if one exists
3. builds a `BootstrapSnapshot`
4. constructs `Store::from_bootstrap(...)`
5. configures the platform sleep service from the same hydrated timeout
6. logs the effective `SettingsState`

This guarantees that:

- cold boot and deep-sleep wake use the same hydration path
- the store and platform sleep timeout agree at startup
- defaults are deterministic when no record exists

## Current Field Semantics

### `inactivity_timeout_ms`

This is the inactivity window used by the sleep service before entering deep sleep.

Current behavior:

- stored as milliseconds
- mirrored into `Store.sleep.config.inactivity_timeout_ms`
- applied to the platform sleep service during bootstrap
- defaults to `30_000 ms`

## Logging

The current firmware logs the effective hydrated settings once during startup.

That log is intended to answer a simple operational question:

- what settings is the device actually running with right now

This log should reflect the live store value, not merely the raw bytes read from flash.

## Future Expansion

This module is the intended home for future preferences such as:

- theme mode
- font family
- font scale
- reading-speed preferences
- future device-level UI behavior

Those should be added as typed fields in the settings model and persisted through the same internal
storage path.

## Non-Goals

The settings module should not own:

- pairing or backend identity
- transport credentials as live runtime state
- flash compaction policy
- sleep hardware entry mechanics

Those belong to `device`, storage, or the platform layer.
