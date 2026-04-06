# Storage

## Purpose

The storage layer is the persistence boundary for Motif.

It owns two different media classes:

- internal flash for small, durable device state
- SD storage for larger article packages, assets, and future imports

This module should never degenerate into scattered raw flash writes from app or platform code.

## Current Internal Flash Design

The current implementation is a durable internal-flash engine in
`crates/platform-esp32s3/src/storage.rs`.

It is intentionally narrow:

- singleton records for compact state
- a durable append-only outbox queue
- raw NOR flash access through `esp-storage::FlashStorage`
- no filesystem
- no encryption in this first pass

The target hardware assumption is the ESP32-S3 N16R8-class device:

- `16 MB` internal flash
- `8 MB` PSRAM

PSRAM is for working memory and caches. It is not authoritative persistence.

## Internal Flash Partitions

The firmware now expects a custom partition table in `partitions/motif.csv`.

Two Motif-owned partitions are reserved:

| Partition | Size | Purpose |
| --- | --- | --- |
| `motif_state` | `256 KiB` | latest-value singleton records and compact metadata |
| `motif_outbox` | `256 KiB` | durable queued items awaiting future sync ack |

The current flashing config in `espflash.toml` assumes the 16 MB module and uses that partition
table by default.

## Record Model

Internal state is append-only and copy-on-write.

Every state record write appends a new committed entry rather than rewriting older data in place.
Delete is represented by a tombstone entry.

Each committed slot carries:

- record magic
- format version
- logical record key
- schema version
- monotonic sequence
- payload length
- CRC32
- commit marker written last

A record is valid only if:

- the header parses
- the payload length is sane
- the CRC matches
- the commit marker is present

That rule is what protects against battery loss during a write.

## Outbox Model

The outbox partition is also append-only.

It stores:

- `enqueue` records for new items
- `ack` records that retire a previously queued sequence number

Boot recovery reconstructs the live queue as:

- all committed enqueue records
- minus any later committed ack record for the same sequence

This avoids in-place queue mutation and keeps recovery logic deterministic.

## Bank Layout and Recovery

Each partition is split into two erase-aligned banks.

The active bank accepts append-only writes until it runs out of room. At that point the engine:

1. scans the live committed data in the active bank
2. erases the inactive bank
3. copies only live data into the inactive bank
4. commits the inactive bank header
5. flips the active-bank pointer

Only after a valid destination bank exists is the previous bank considered disposable.

This means power loss during compaction should not destroy the last committed state.

On boot, mount and recovery:

- scan both banks
- select the newest committed valid bank
- ignore torn or corrupt records
- initialize a fresh bank if neither side is valid

## Service Boundary

The shared contract lives in `crates/services/src/storage.rs`.

The important abstractions are:

- `RecordCodec`
- `QueueCodec`
- `StorageService`
- `StorageHealth`
- `RecordKey`
- `QueueSeq`

The API is typed on purpose. Callers work with record codecs and queue codecs, not raw offsets.

Reserved record families at the architecture level are:

- device settings snapshots
- active Wi-Fi credential
- Wi-Fi credentials or remembered network configuration
- backend credential or device token
- backend token or device credential material
- device identity and pairing metadata
- provisioning metadata for recovery and reprovisioning hygiene
- storage migration metadata

The storage layer does not lock those schemas yet. It only provides the durable pattern.

## SD Responsibilities

SD remains the place for heavier payloads:

- normalized article bodies
- assets
- future EPUB and TXT imports
- staging areas and derived caches that do not belong in internal flash

Internal flash must stay small, durable, and cheap to recover.

## Current SD Package Pipeline

The SD path is now much more concrete than the original architecture notes.

Current implemented behavior includes:

- bounded streaming package writes through the storage command path
- open-handle staging so the active file is not reopened for every chunk
- asynchronous queued package writes instead of an ACK-blocking round trip per
  chunk
- direct writes to a free final package slot where safe, with `copied_bytes=0`
  on commit
- immediate open-after-commit for uncached article prepares
- runtime SD SPI that initializes conservatively at `400 kHz` and switches to
  a product-default `8 MHz` after mount
- boot/storage telemetry that records the SD init clock, runtime clock, and the
  source of that runtime choice
- a controlled build-time override path (`MOTIF_SD_SPI_RUN_HZ`) for deliberate
  higher-clock experiments without changing the product default

Current package/stage buffers are intentionally much larger than the original
baseline:

- package download chunk length: `8192` bytes
- stage write chunk length: `8192` bytes

That work removed SD throughput as the primary bottleneck on healthy runs.

## Guarantees and Non-Goals

Current guarantees:

- no in-place overwrite of the live singleton value
- torn writes are ignored
- corrupt CRC entries are ignored
- queue ack is durable
- storage health can be reported at boot

Current non-goals:

- encryption at rest
- wear-leveling beyond the simple dual-bank copy-on-write model
- arbitrary large-value storage
- direct flash access from app code

## Implementation Status

What exists now:

- typed storage contracts in `domain` and `services`
- a mounted flash-backed internal storage engine in `platform-esp32s3`
- runtime partition discovery by label
- a real persisted-settings record used during bootstrap hydration
- an expanded persisted-settings snapshot carrying timeout, RSVP speed, appearance, and topic
  preferences
- app-level settings persistence through typed store effects executed by the platform storage path
- startup configuration now logs the effective hydrated settings after that record is applied
- recovery, compaction, and queue semantics
- storage health reporting during boot
- persisted backend credential records used by refresh/session startup
- a real SD-backed content/package pipeline with staging, commit, abort, and
  cached package open flows
- PSRAM-backed reader/storage working sets for initial package open and reader
  materialization

What does not exist yet:

- Wi-Fi credential records
- broader app-level persistence beyond settings snapshots and current content
  metadata
- persisted local reading-progress records and remote progress upload
