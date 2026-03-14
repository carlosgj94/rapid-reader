# RSVP Reading

This note documents the current countdown and reading loop.

## Entering Reading

The normal entry path is:

1. Select a book in the library.
2. Enter the countdown screen.
3. Wait for the countdown to complete, or press once to skip directly into reading.

Current defaults:

- Countdown: 3 seconds
- WPM: 230
- WPM range: 80..600
- Dot pause: 240 ms
- Comma pause: 240 ms

## Countdown Screen

Current behavior:

- Shows the selected title, current WPM, and cover slot if the book has a cover.
- Ticks once per second until it reaches reading.
- Uses a pulse animation between countdown steps.

Controls:

- Press: skip the remaining countdown and start reading immediately.
- Rotate: change WPM before reading starts.

## Reading Loop

The reading loop is owned by `ReaderApp::tick()` and advances one word at a time.

Current behavior:

- The renderer shows one RSVP word at a time with a center anchor layout.
- The reading header shows the current section label rather than the full book title.
- The paragraph progress counter tracks the current word index and total words in the paragraph.
- Word delay is `60000 / WPM`, with extra pause when the emitted token ends in sentence or clause
  punctuation.
- Typography comes from persisted style settings: font family, size, and invert mode.

Controls while reading:

- Press once: toggle pause/resume.
- Press twice within the current 120..450 ms double-press window: return to the library.
- Rotate clockwise while actively reading: increase WPM.
- Rotate counter-clockwise while actively reading: decrease WPM.

## Paused Reading

When reading is paused:

- The screen stays in the reading view and runs a pause animation.
- Rotating no longer changes WPM.
- Rotation now enters chapter navigation, seeded from the current chapter.

This makes pause the gateway into navigation. There is no separate pause menu.

## End-Of-Chunk And End-Of-Text Behavior

Current stream behavior:

- If the current chunk runs out but the book is not terminal, the app requests an SD refill and
  waits in reading rather than bouncing back to the library.
- If the source reaches terminal end-of-text, reading pauses in place and shows an `END` fallback
  word if no other word is buffered.
- If a resume or opening position is unreadable before any word renders, the app first tries to
  requeue chapter 1; if that fails it falls back toward the start of the book.

This behavior favors keeping the user inside the reading flow even when chunk boundaries or resume
positions are imperfect.
