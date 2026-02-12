# Board Integration Contract (ESP32-S3 + LS027B7DH01)

This project splits responsibilities as:
- `crates/ls027b7dh01`: reusable no_std driver and framebuffer logic
- `src/bin/main.rs`: board glue (pins, SPI peripheral, timing loop)

## Required panel connections
- `SCLK` -> SPI SCK pin
- `SI` -> SPI MOSI pin
- `SCS` -> GPIO pin (active-high CS)
- `DISP` -> GPIO output pin
- `EXTCOMIN` -> GPIO output pin
- `GND`/`VCC` as required by panel breakout hardware

## Runtime requirements
- Toggle `EXTCOMIN` periodically (default: every 500 ms).
- Keep this toggling independent from screen redraw cadence.
- Keep redraw/update logic separate so protocol debugging can proceed in phases.

## Bring-up stages in firmware
- `ExtComHeartbeat`: only DISP + EXTCOMIN control
- `ClearLoop`: repeatedly issue all-clear
- `SingleLine`: update specific lines only
- `FullFrame`: write full framebuffer test patterns

## Pin selection
`src/bin/main.rs` contains explicit pin constants that can be changed to match actual wiring.

## SD card wiring
SD card support is disabled in current firmware.
