# LS027B7DH01 Notes (Bring-Up Contract)

Source: `LS027B7DH01_Rev_Jun_2010.pdf`.

## Fixed panel facts
- Resolution: 400 x 240 pixels (1bpp)
- Line payload: 400 bits = 50 bytes
- Lines: 1..=240

## Key timing used for initial bring-up
- SPI clock (`fSCLK`): start at 1 MHz
- Frame update cadence (`fSCS`): start at 1 Hz equivalent test cadence
- EXTCOMIN: start at 1 Hz square wave (toggle every 500 ms)

## Interface signals
- `SCLK`: SPI clock
- `SI`: SPI data input (MOSI)
- `SCS`: chip select, active-high framing signal
- `DISP`: display enable pin
- `EXTCOMIN`: external common inversion input

## Protocol framing assumptions
- Mode bits are sent in order `M0 M1 M2`, then dummy bits.
- For data update of one line:
  - mode select (3 bits) + dummy (5 bits)
  - gate line address (`AG0..AG7`, 8 bits)
  - line data (`D1..D400`, 400 bits)
  - transfer dummy (16 bits)
- For all-clear:
  - mode select with `M0=0, M2=1`
  - at least 13 dummy bits afterward

## Address mapping assumption
- Gate address is little-endian by line number bits: `AG0` is line bit 0.
- Wire byte for address in MSB-first SPI is `reverse_bits(line_number as u8)`.
  - L1 -> `0x80`
  - L2 -> `0x40`
  - L3 -> `0xC0`
  - L240 (`0xF0`) -> `0x0F`

## Pixel bit order assumption
- First pixel in a line (`D1`) maps to bit 7 of byte 0.
- Pixel index in line maps to `(byte = x / 8, bit = 7 - (x % 8))`.

These assumptions are intentionally explicit so bring-up can validate/fix one dimension at a time if behavior differs on hardware.
