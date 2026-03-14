# Board Integration Contract

This note follows the current firmware wiring in `src/bin/main.rs`.

## Current Wiring

### Display (Sharp LS027 path in firmware)

- `GPIO13` -> `CLK` / display SPI SCK
- `GPIO14` -> `DI` / display SPI MOSI
- `GPIO15` -> `CS` / `SCS`
- `GPIO2` -> `DISP`
- `GPIO9` -> `EMD` / `EXTCOMIN`

Current display assumptions:

- LS027 uses SPI mode 1 in firmware.
- The board display adapter owns `DISP`, `EMD`, and `CS` pin state for init, frame flush, and
  deep-sleep shutdown.
- The display is explicitly disabled before deep sleep.

### SD Card (SPI)

- `GPIO8` -> `CS`
- `GPIO4` -> `SCK`
- `GPIO40` -> `MOSI`
- `GPIO41` -> `MISO`

Current SD assumptions:

- SD support is enabled in current firmware.
- Boot probes multiple SPI speeds until catalog access succeeds or the configured attempts are
  exhausted.
- `/BOOKS` is the current library root.

### Rotary Encoder

- `GPIO10` -> `CLK`
- `GPIO11` -> `DT`
- `GPIO12` -> `SW`

Current input assumptions:

- Encoder inputs use pull-ups.
- Deep sleep wake uses the encoder switch pin.

## Board-Level Runtime Responsibilities

Board glue currently owns:

- peripheral bring-up
- display init and first frame
- SD preload and runtime refill wiring
- flash settings store integration
- deep sleep entry and wake restore sequencing

Hardware design files still live under `kicad/readily/`, but they are not the primary source of
truth for runtime behavior in this documentation pass.
