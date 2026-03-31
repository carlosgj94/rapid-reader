# Input

## Purpose

The input module translates physical device interactions into typed gestures the rest of the system
can understand.

It should isolate hardware quirks and debouncing from navigation and reader logic.

## Current Physical Baseline

The current hardware baseline centers on a rotary encoder with a switch input. Future inputs can be
added later without rewriting app logic if the semantic layer stays stable.

## Input Pipeline

The current pipeline is:

1. `PlatformInputService` observes raw pin state
2. the input service debounces and interprets raw signals
3. the input service enqueues typed gestures in its internal fixed-capacity queue
4. the platform runtime loop drains queued gestures
5. the platform runtime publishes `Event::InputGestureReceived(...)` into the app/store path
6. `app_task` updates `Store` and forwards the gesture into `AppRuntime`

The current hardware split is:

- rotary quadrature is polled on a short cadence
- button edges are detected with GPIO interrupts while awake
- the same button remains the deep-sleep wake input

The current runtime constants are:

- rotation poll cadence: `2 ms`
- button debounce: `20 ms`
- long-press threshold: `600 ms`
- gesture queue capacity: `16`
- one rotation gesture per detent threshold of `2` quadrature steps

## Gesture Model

The stable input boundary is gestures, not screen-specific actions.

The current gesture vocabulary is:

- `Rotate { direction: Clockwise }`
- `Rotate { direction: CounterClockwise }`
- `Click`
- `LongPress`

Important behavior defaults:

- one rotation gesture per stable encoder detent
- no acceleration in v1
- long press threshold is `600 ms`
- long press suppresses click
- the wake press is consumed and never replayed as a click
- clockwise and counterclockwise are documented in physical device terms, not screen semantics

## Separation Of Concerns

The input module owns:

- debouncing
- detent interpretation
- press classification
- the transient input queue
- future gesture classification

The input module does not own:

- which screen is active
- which action a given screen treats as confirm or cancel
- reader progression logic

Those belong to the store and app runtime.

## Current Platform Details

The current ESP32-S3 implementation lives in `crates/platform-esp32s3/src/input.rs`.

It uses:

- pull-up inputs on `GPIO10`, `GPIO11`, and `GPIO12`
- a Gray-code transition table for quadrature decoding
- a fixed-size internal queue rather than heap allocation
- button wake suppression until release after deep-sleep boot

Accepted gestures reset inactivity only after they are successfully delivered into the runtime.
Raw GPIO edges do not count as activity on their own.
Dropped gestures are currently logged by the platform layer and are not yet fed back into the
store as a first-class event.

## Focus And Navigation

The app runtime should expose enough focus context that gestures can be interpreted
consistently across screens.

That means:

- queue navigation and settings navigation can interpret the same gestures differently
- the reader surface can reinterpret gestures without redefining raw input behavior
- focus is a UI concern, not an electrical concern

That focus-aware mapping is still future work. Today the runtime stops at gesture delivery and does
not yet translate gestures into queue navigation or reader controls.

## Extension Path

This architecture should support future additions such as:

- multi-click
- wake-from-input semantics
- alternative physical controls

The gesture layer is the compatibility boundary that makes those additions manageable.
