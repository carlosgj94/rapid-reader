# Runtime Behaviors

This note collects the current persistence, resume, sleep, and visible edge-case behavior that sits
around the primary reading flows.

## Settings Persistence

Persisted settings currently include:

- WPM
- font family
- font size
- invert mode
- optional wake snapshot for deep sleep restore

Current behavior:

- Settings changes are tracked continuously from app state.
- Flash writes are debounced by 1500 ms.
- If flash storage is unavailable, the device keeps running and settings become volatile for that
  session.

## Resume Persistence

The current resume payload includes:

- selected book
- chapter index
- paragraph index within chapter
- word index within paragraph

Current behavior:

- Resume progress is stored in the SD-side progress DB, keyed by the book short name.
- Paragraph movement while actively reading is saved with a debounce window.
- Entering a sleep-eligible state after progress changed can force an earlier save.
- Resume is also flushed before deep sleep.

Current defaults:

- Resume save debounce: 4000 ms
- Minimum spacing between forced saves: 500 ms

## Deep Sleep And Wake

Current sleep policy:

- Inactivity sleep fires after 60000 ms with no input.
- Sleep is only allowed when the app is not actively reading words.
- Library, settings, paused reading, chapter navigation, paragraph navigation, and status screens
  are sleep-eligible.

Before sleep the firmware:

1. Saves resume progress to the SD-side DB when possible.
2. Persists settings plus wake snapshot to flash.
3. Shows a `SLEEPING... / PRESS TO WAKE` status screen briefly.
4. Powers the display down and enters deep sleep.

Wake behavior:

- Wake comes from the encoder switch pin.
- Wake first tries to restore the flash wake snapshot.
- If SD-side resume data exists, it is merged into the wake snapshot's reading location.
- If no wake snapshot exists, plain SD-side resume restore is attempted.
- Restored reading resumes in a paused state, never in active autoplay.

## Current Stream And Parsing Behavior

- EPUB text is read in 480-byte chunks.
- UTF-8 carryover, HTML stripping, HTML entity decode, and common cp1252 punctuation cleanup happen
  before a chunk becomes reader-visible text.
- When a chunk contains no reader-visible text but the stream is not terminal, the core queues
  another refill instead of treating it as hard end-of-book.
- Chapter metadata is partly heuristic until the target resource has been loaded.

## Current Cover Behavior

Supported today:

- PNG
- compatible JPEG path
- PBM

Visible fallback behavior:

- Unsupported or missing covers fall back to the symbolic shelf card artwork.
- Progressive or unsupported JPEGs fail closed and do not block boot or reading.

## Current Operational Notes

- Wi-Fi failure does not block local reading behavior.
- FAT32 FSInfo warnings can be noisy without being fatal if the SD catalog still loads.
- Placeholder library titles can appear when SD probing fails, but they are not a substitute for a
  real populated library.
