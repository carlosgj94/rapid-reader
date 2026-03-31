# Backend Sync

## Purpose

The backend sync module is the device's bridge to remote product state.

It will eventually own:

- device pairing validation
- source manifest refresh
- article package acquisition
- progress upload
- sync scheduling and retry policy

It depends on Wi-Fi availability, but it is not the Wi-Fi module.

## Current Status

Backend sync is still a skeleton.

What exists today:

- architecture docs for the module boundary
- a no-op backend sync service boundary in the services crate

What does not exist yet:

- device pairing exchange
- manifest refresh
- content downloads
- progress upload
- retry and backoff orchestration

## Pairing Model

The target v1 identity model is a paired device token rather than a full login UI on-device.

That implies:

- the device is paired once through provisioning
- the resulting credential is stored locally
- later sync traffic uses that device identity

Provisioning owns the onboarding session that delivers pairing material. Backend sync takes over
after the device has network connectivity and needs to validate or use that credential.

## Sync Responsibilities

The sync module should eventually coordinate:

1. refresh source manifests
2. compare remote article revisions with local availability
3. schedule package downloads through storage
4. publish availability changes into the store
5. upload progress and local state changes when possible

## Offline Behavior

If the device is offline:

- the reader operates from local content
- freshness becomes stale rather than invalid
- pending sync work accumulates locally
- work resumes when connectivity returns

## Storage Boundary

Backend sync should not write raw files or flash records directly. It should ask the storage module
to stage and commit durable data so persistence policy stays in one place.
