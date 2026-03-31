use ::domain::input::{InputGesture, RotationDirection};
use ::services::input::InputService;
use esp_hal::gpio::{AnyPin, Event as GpioEvent, Input, InputConfig, Pull};

const INPUT_QUEUE_CAPACITY: usize = 16;
const BUTTON_DEBOUNCE_MS: u64 = 20;
const LONG_PRESS_MS: u64 = 600;
const DETENT_DELTA: i8 = 2;
const ROTARY_TRANSITIONS: [i8; 16] = [0, -1, 1, 0, 1, 0, 0, -1, -1, 0, 0, 1, 0, 1, -1, 0];

#[derive(Debug)]
pub struct PlatformInputService<'d> {
    encoder_clk: Input<'d>,
    encoder_dt: Input<'d>,
    button: Input<'d>,
    wake_button_pin: Option<AnyPin<'d>>,
    queue: GestureQueue,
    encoder_state: EncoderState,
    button_state: ButtonState,
    dropped_gestures: u32,
}

impl<'d> PlatformInputService<'d> {
    pub fn new(
        encoder_clk_pin: AnyPin<'d>,
        encoder_dt_pin: AnyPin<'d>,
        button_pin: AnyPin<'d>,
        woke_from_deep_sleep: bool,
    ) -> Self {
        let config = InputConfig::default().with_pull(Pull::Up);
        let encoder_clk = Input::new(encoder_clk_pin, config);
        let encoder_dt = Input::new(encoder_dt_pin, config);

        // The input driver needs its own pin instance while we still retain the raw
        // wake pin for deep-sleep handoff later.
        let button_driver_pin = unsafe { button_pin.clone_unchecked() };
        let mut button = Input::new(button_driver_pin, config);
        button.clear_interrupt();
        button.listen(GpioEvent::AnyEdge);

        let initial_encoder_sample = sample_encoder_inputs(&encoder_clk, &encoder_dt);
        let initial_button_pressed = button.is_low();

        Self {
            encoder_clk,
            encoder_dt,
            button,
            wake_button_pin: Some(button_pin),
            queue: GestureQueue::new(),
            encoder_state: EncoderState::new(initial_encoder_sample),
            button_state: ButtonState::new(initial_button_pressed, woke_from_deep_sleep),
            dropped_gestures: 0,
        }
    }

    pub fn sample(&mut self, now_ms: u64) {
        self.sample_rotation();
        self.sample_button(now_ms);
    }

    pub fn take_dropped_gesture_count(&mut self) -> u32 {
        let dropped = self.dropped_gestures;
        self.dropped_gestures = 0;
        dropped
    }

    pub fn take_wake_button(&mut self) -> AnyPin<'d> {
        self.button.unlisten();
        self.button.clear_interrupt();
        self.wake_button_pin
            .take()
            .expect("wake button pin can only be taken once")
    }

    fn sample_rotation(&mut self) {
        let current_sample = sample_encoder_inputs(&self.encoder_clk, &self.encoder_dt);
        if let Some(direction) = self.encoder_state.sample(current_sample) {
            self.push_gesture(InputGesture::Rotate { direction });
        }
    }

    fn sample_button(&mut self, now_ms: u64) {
        let current_pressed = self.button.is_low();

        if self.button.is_interrupt_set() {
            self.button.clear_interrupt();
            self.button.listen(GpioEvent::AnyEdge);
            self.button_state.begin_debounce(current_pressed, now_ms);
        } else if self.button_state.needs_resync(current_pressed) {
            self.button_state.begin_debounce(current_pressed, now_ms);
        }

        if let Some(gesture) = self.button_state.update(current_pressed, now_ms) {
            self.push_gesture(gesture);
        }

        if let Some(gesture) = self.button_state.poll_long_press(now_ms) {
            self.push_gesture(gesture);
        }
    }

    fn push_gesture(&mut self, gesture: InputGesture) {
        if !self.queue.push(gesture) {
            self.dropped_gestures = self.dropped_gestures.saturating_add(1);
        }
    }
}

impl InputService for PlatformInputService<'_> {
    fn pop_gesture(&mut self) -> Option<InputGesture> {
        self.queue.pop()
    }
}

fn sample_encoder_inputs(clk: &Input<'_>, dt: &Input<'_>) -> u8 {
    ((clk.is_high() as u8) << 1) | (dt.is_high() as u8)
}

#[derive(Debug)]
struct GestureQueue {
    head: usize,
    len: usize,
    entries: [Option<InputGesture>; INPUT_QUEUE_CAPACITY],
}

impl GestureQueue {
    const fn new() -> Self {
        Self {
            head: 0,
            len: 0,
            entries: [None; INPUT_QUEUE_CAPACITY],
        }
    }

    fn push(&mut self, gesture: InputGesture) -> bool {
        if self.len == INPUT_QUEUE_CAPACITY {
            return false;
        }

        let tail = (self.head + self.len) % INPUT_QUEUE_CAPACITY;
        self.entries[tail] = Some(gesture);
        self.len += 1;
        true
    }

    fn pop(&mut self) -> Option<InputGesture> {
        if self.len == 0 {
            return None;
        }

        let gesture = self.entries[self.head].take();
        self.head = (self.head + 1) % INPUT_QUEUE_CAPACITY;
        self.len -= 1;
        gesture
    }
}

#[derive(Debug, Clone, Copy)]
struct EncoderState {
    previous_sample: u8,
    accumulated_delta: i8,
}

impl EncoderState {
    const fn new(initial_sample: u8) -> Self {
        Self {
            previous_sample: initial_sample,
            accumulated_delta: 0,
        }
    }

    fn sample(&mut self, current_sample: u8) -> Option<RotationDirection> {
        let transition = ((self.previous_sample as usize) << 2) | current_sample as usize;
        self.previous_sample = current_sample;

        let delta = ROTARY_TRANSITIONS[transition];
        if delta == 0 {
            return None;
        }

        self.accumulated_delta += delta;

        if self.accumulated_delta <= -DETENT_DELTA {
            self.accumulated_delta = 0;
            return Some(RotationDirection::CounterClockwise);
        }

        if self.accumulated_delta >= DETENT_DELTA {
            self.accumulated_delta = 0;
            return Some(RotationDirection::Clockwise);
        }

        None
    }
}

#[derive(Debug, Clone, Copy)]
struct ButtonState {
    stable_pressed: bool,
    candidate_pressed: Option<bool>,
    candidate_since_ms: u64,
    press_started_ms: Option<u64>,
    long_press_emitted: bool,
    suppress_until_release: bool,
}

impl ButtonState {
    const fn new(initial_pressed: bool, suppress_until_release: bool) -> Self {
        Self {
            stable_pressed: initial_pressed,
            candidate_pressed: None,
            candidate_since_ms: 0,
            press_started_ms: None,
            long_press_emitted: false,
            suppress_until_release,
        }
    }

    fn needs_resync(&self, current_pressed: bool) -> bool {
        self.candidate_pressed.is_none() && current_pressed != self.stable_pressed
    }

    fn begin_debounce(&mut self, current_pressed: bool, now_ms: u64) {
        if self.candidate_pressed != Some(current_pressed) {
            self.candidate_pressed = Some(current_pressed);
            self.candidate_since_ms = now_ms;
        }
    }

    fn update(&mut self, current_pressed: bool, now_ms: u64) -> Option<InputGesture> {
        let candidate_pressed = self.candidate_pressed?;

        if current_pressed != candidate_pressed {
            self.candidate_pressed = Some(current_pressed);
            self.candidate_since_ms = now_ms;
            return None;
        }

        if now_ms.saturating_sub(self.candidate_since_ms) < BUTTON_DEBOUNCE_MS {
            return None;
        }

        self.candidate_pressed = None;
        if candidate_pressed == self.stable_pressed {
            return None;
        }

        self.stable_pressed = candidate_pressed;
        if candidate_pressed {
            self.on_pressed(now_ms);
            None
        } else {
            self.on_released(now_ms)
        }
    }

    fn poll_long_press(&mut self, now_ms: u64) -> Option<InputGesture> {
        if self.suppress_until_release || !self.stable_pressed || self.long_press_emitted {
            return None;
        }

        let press_started_ms = self.press_started_ms?;

        if now_ms.saturating_sub(press_started_ms) < LONG_PRESS_MS {
            return None;
        }

        self.long_press_emitted = true;
        Some(InputGesture::LongPress)
    }

    fn on_pressed(&mut self, now_ms: u64) {
        if self.suppress_until_release {
            return;
        }

        self.press_started_ms = Some(now_ms);
        self.long_press_emitted = false;
    }

    fn on_released(&mut self, now_ms: u64) -> Option<InputGesture> {
        if self.suppress_until_release {
            self.suppress_until_release = false;
            self.press_started_ms = None;
            self.long_press_emitted = false;
            return None;
        }

        let gesture = if self.long_press_emitted {
            None
        } else {
            self.press_started_ms.and_then(|press_started_ms| {
                (now_ms.saturating_sub(press_started_ms) >= BUTTON_DEBOUNCE_MS)
                    .then_some(InputGesture::Click)
            })
        };

        self.press_started_ms = None;
        self.long_press_emitted = false;
        gesture
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_clockwise_after_one_positive_detent() {
        let mut state = EncoderState::new(0b00);

        assert_eq!(state.sample(0b10), None);
        assert_eq!(state.sample(0b11), Some(RotationDirection::Clockwise));
    }

    #[test]
    fn emits_counterclockwise_after_one_negative_detent() {
        let mut state = EncoderState::new(0b00);

        assert_eq!(state.sample(0b01), None);
        assert_eq!(
            state.sample(0b11),
            Some(RotationDirection::CounterClockwise)
        );
    }
}
