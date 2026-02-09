# READILY

```text
██████╗ ███████╗ █████╗ ██████╗ ██╗██╗  ██╗   ██╗
██╔══██╗██╔════╝██╔══██╗██╔══██╗██║██║  ╚██╗ ██╔╝
██████╔╝█████╗  ███████║██║  ██║██║██║   ╚████╔╝
██╔══██╗██╔══╝  ██╔══██║██║  ██║██║██║    ╚██╔╝
██║  ██║███████╗██║  ██║██████╔╝██║███████╗██║
╚═╝  ╚═╝╚══════╝╚═╝  ╚═╝╚═════╝ ╚═╝╚══════╝╚═╝
```

An embedded Rust RSVP reader for ESP32-S3 + Sharp memory LCD.

`readily` is built to feel like a product, not just a hardware demo: fast word presentation, clean reading UI, animated navigation, and persistent user settings.

## What It Does Today

- Library-style home screen with animated book cards
- Reading flow with 3-second countdown
- RSVP center-anchor word rendering
- Pause overlay + chapter/paragraph navigation selector
- Adjustable typography/style (font, size, invert mode, WPM)
- Flash-backed persisted settings on ESP32-S3
- Rotary encoder input (rotate + press)

## Architecture

```text
readily (workspace root)
├─ src/bin/main.rs                # board wiring + main loop
├─ crates/readily-core            # app state machine, traits, screen view models
├─ crates/readily-hal-esp32s3     # ESP32-S3 input/render/storage/display adapters
└─ crates/ls027b7dh01             # Sharp LS027 protocol + framebuffer driver
```

Design rule:
- Core logic is hardware-independent.
- HAL crates adapt hardware to core traits.

## Hardware Wiring (Current Firmware Defaults)

### Display (Sharp LS027 breakout)

| Breakout Pin | Firmware Signal | ESP32-S3 GPIO |
|---|---|---|
| `CLK` | SPI SCK | `GPIO4` |
| `DI` | SPI MOSI | `GPIO5` |
| `CS` | `SCS` | `GPIO6` |
| `DISP` | display enable | `GPIO7` |
| `EMD` | `EXTCOMIN` | `GPIO9` |
| `GND` | ground | `GND` |
| `3V` | power | `3V3` |

Notes:
- `EIN` is not used by current code.
- Prefer 3.3V supply for this setup.
- If your board revision expects supply on `VIN`, feed `VIN` with 3.3V here.

### Rotary Encoder

| Encoder Pin | ESP32-S3 GPIO |
|---|---|
| `CLK` | `GPIO10` |
| `DT`  | `GPIO11` |
| `SW`  | `GPIO12` |

## Controls

- Home: rotate to select book/settings, press to enter
- Reading:
  - single press: pause/resume
  - double press: exit back to home
- Pause:
  - rotate: browse chapter/paragraph selectors
  - press: confirm selection / jump

## Build, Flash, Monitor

The project is configured with:
- toolchain channel: `esp`
- default target: `xtensa-esp32s3-none-elf`
- runner: `espflash flash --monitor --chip esp32s3`

So this is enough to flash and open serial monitor:

```bash
cargo run --release
```

## Developer Checks

```bash
cargo fmt --all
cargo check --offline
cargo clippy --offline --workspace --lib
```

Note:
- `cargo test --workspace` may fail under the default embedded target context.

## Key Runtime Defaults

- WPM: `230` (range `80..600`)
- Dot pause: `240 ms`
- Comma pause: `240 ms`
- Countdown: `3` seconds
- Settings save debounce: `1500 ms`

## Current Content Backend

The active source is `FakeSdCatalogSource` (stub) in:

`crates/readily-core/src/content/sd_stub.rs`

It already matches core traits, so replacing it with a real SD implementation is a drop-in change at the app boundary.

## Near-Term Roadmap

- Real SD text catalog + paragraph/chapter metadata
- Book cover assets in library cards
- More navigation affordances for long books
- Additional motion presets tuned for memory-LCD refresh behavior
