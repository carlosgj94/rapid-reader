# Board Integration Contract

This note documents the current hardware contract used by the stripped firmware baseline and by the
target architecture docs.

## Current Wiring

### Display

- `GPIO13` -> `CLK` / display SPI SCK
- `GPIO14` -> `DI` / display SPI MOSI
- `GPIO15` -> `CS` / `SCS`
- `GPIO2` -> `DISP`
- `GPIO9` -> `EMD` / `EXTCOMIN`

Current display assumptions:

- LS027 uses SPI mode 1 in firmware.
- The stripped firmware initializes the panel, clears it once, and then refreshes a minimal
  heartbeat frame.
- On deep sleep entry, firmware clears the panel, drives `DISP` and `EMD` low, and holds those
  RTC pins low during sleep.

### SD Card

- `GPIO8` -> `CS`
- `GPIO4` -> `SCK`
- `GPIO40` -> `MOSI`
- `GPIO41` -> `MISO`

Current SD assumptions:

- The target architecture uses SD for article packages, assets, imports, and larger caches.
- The stripped firmware baseline does not currently initialize the SD path.

### Rotary Encoder

- `GPIO10` -> `CLK`
- `GPIO11` -> `DT`
- `GPIO12` -> `SW`

Current input assumptions:

- The target architecture treats the encoder as the primary navigation input.
- Firmware currently initializes the encoder path for rotation, click, and long-press detection.
- Rotary movement is polled on a short cadence.
- The encoder switch uses GPIO edge interrupts while awake.
- The current encoder software contract is one gesture per detent, plus `Click` and `LongPress`.
- The current long-press threshold is `600 ms`.

### Battery / Power Sensing

- Battery telemetry is not currently wired into the PCB path for software integration.
- The future power module should be treated as an architectural placeholder until that hardware
  path exists.

### Deep Sleep Wake

- `GPIO12` is the provisional deep-sleep wake input in software.
- This uses the encoder switch path until a dedicated wake button exists.
- Firmware consumes the wake press until release so a wake does not also trigger a click event.

## Board-Level Responsibilities

The current stripped firmware owns:

- minimal ESP32-S3 bring-up
- LS027 init and framebuffer flush
- embassy runtime wiring between platform and app/store tasks
- rotary encoder gesture decoding and queueing
- inactivity-based deep sleep and wake
- internal flash storage mounting and health logging
- GPIO contract logging for display, SD, and encoder pins

Hardware design files under the KiCad project directory are left untouched by this reset.
