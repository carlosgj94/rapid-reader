# Board Integration Contract

This note documents the GPIO wiring kept in the stripped firmware source.

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
- `DISP`, `EMD`, and `CS` are driven directly from `src/bin/main.rs`.

### SD Card

- `GPIO8` -> `CS`
- `GPIO4` -> `SCK`
- `GPIO40` -> `MOSI`
- `GPIO41` -> `MISO`

Current SD assumptions:

- These GPIOs remain documented for the next rebuild stage.
- The stripped firmware does not currently initialize or use the SD card path.

### Rotary Encoder

- `GPIO10` -> `CLK`
- `GPIO11` -> `DT`
- `GPIO12` -> `SW`

Current input assumptions:

- These GPIOs remain documented for the next rebuild stage.
- The stripped firmware does not currently initialize or use the encoder path.

## Board-Level Responsibilities

The retained firmware owns:

- minimal ESP32-S3 bring-up
- LS027 init and framebuffer flush
- GPIO contract logging for display, SD, and encoder pins

Hardware design files under `kicad/readily/` are left untouched by this reset.
