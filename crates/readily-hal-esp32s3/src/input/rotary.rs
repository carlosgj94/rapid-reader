use embedded_hal::digital::InputPin;

use readily_core::input::{InputEvent, InputProvider};

// Quadrature transition table for previous_state/current_state (AB).
// Index: (prev << 2) | curr, values are +1/-1 for valid transitions.
const QUADRATURE_TRANSITIONS: [i8; 16] = [0, -1, 1, 0, 1, 0, 0, -1, -1, 0, 0, 1, 0, 1, -1, 0];

#[derive(Debug, Clone, Copy)]
pub struct RotaryConfig {
    direction_inverted: bool,
    button_active_low: bool,
    button_debounce_polls: u8,
    transitions_per_step: u8,
}

impl Default for RotaryConfig {
    fn default() -> Self {
        Self {
            direction_inverted: false,
            button_active_low: true,
            button_debounce_polls: 3,
            transitions_per_step: 4,
        }
    }
}

impl RotaryConfig {
    pub const fn with_direction_inverted(mut self, direction_inverted: bool) -> Self {
        self.direction_inverted = direction_inverted;
        self
    }

    pub const fn with_button_active_low(mut self, button_active_low: bool) -> Self {
        self.button_active_low = button_active_low;
        self
    }

    pub const fn with_button_debounce_polls(mut self, button_debounce_polls: u8) -> Self {
        self.button_debounce_polls = button_debounce_polls;
        self
    }

    pub const fn with_transitions_per_step(mut self, transitions_per_step: u8) -> Self {
        self.transitions_per_step = transitions_per_step;
        self
    }
}

#[derive(Debug)]
pub enum RotaryInputError<ClkErr, DtErr, SwErr> {
    Clk(ClkErr),
    Dt(DtErr),
    Sw(SwErr),
}

type RotaryResult<ClkErr, DtErr, SwErr, T> = Result<T, RotaryInputError<ClkErr, DtErr, SwErr>>;

#[derive(Debug)]
pub struct RotaryInput<CLK, DT, SW> {
    clk: CLK,
    dt: DT,
    sw: SW,
    config: RotaryConfig,
    prev_ab: u8,
    transition_accum: i8,
    button_raw: bool,
    button_stable: bool,
    button_stable_count: u8,
    pending_event: Option<InputEvent>,
}

impl<CLK, DT, SW> RotaryInput<CLK, DT, SW>
where
    CLK: InputPin,
    DT: InputPin,
    SW: InputPin,
{
    pub fn new(
        mut clk: CLK,
        mut dt: DT,
        mut sw: SW,
        config: RotaryConfig,
    ) -> RotaryResult<CLK::Error, DT::Error, SW::Error, Self> {
        let clk_high = clk.is_high().map_err(RotaryInputError::Clk)?;
        let dt_high = dt.is_high().map_err(RotaryInputError::Dt)?;
        let sw_high = sw.is_high().map_err(RotaryInputError::Sw)?;

        let prev_ab = ab_from_levels(clk_high, dt_high);
        let button_pressed = button_pressed_from_level(sw_high, config.button_active_low);

        Ok(Self {
            clk,
            dt,
            sw,
            config,
            prev_ab,
            transition_accum: 0,
            button_raw: button_pressed,
            button_stable: button_pressed,
            button_stable_count: 0,
            pending_event: None,
        })
    }

    fn poll_button(
        &mut self,
    ) -> RotaryResult<CLK::Error, DT::Error, SW::Error, Option<InputEvent>> {
        let sw_high = self.sw.is_high().map_err(RotaryInputError::Sw)?;
        let pressed = button_pressed_from_level(sw_high, self.config.button_active_low);

        if pressed == self.button_raw {
            self.button_stable_count = self.button_stable_count.saturating_add(1);
        } else {
            self.button_raw = pressed;
            self.button_stable_count = 0;
        }

        let debounce_threshold = self.config.button_debounce_polls.max(1);
        if self.button_stable_count >= debounce_threshold && self.button_stable != self.button_raw {
            self.button_stable = self.button_raw;
            if self.button_stable {
                return Ok(Some(InputEvent::Press));
            }
        }

        Ok(None)
    }

    fn poll_rotation(
        &mut self,
    ) -> RotaryResult<CLK::Error, DT::Error, SW::Error, Option<InputEvent>> {
        let clk_high = self.clk.is_high().map_err(RotaryInputError::Clk)?;
        let dt_high = self.dt.is_high().map_err(RotaryInputError::Dt)?;
        let curr_ab = ab_from_levels(clk_high, dt_high);

        if curr_ab == self.prev_ab {
            return Ok(None);
        }

        let transition_idx = ((self.prev_ab << 2) | curr_ab) as usize;
        self.prev_ab = curr_ab;
        self.transition_accum = self
            .transition_accum
            .saturating_add(QUADRATURE_TRANSITIONS[transition_idx]);

        let threshold = self.config.transitions_per_step.max(1) as i8;
        if self.transition_accum >= threshold {
            self.transition_accum = 0;
            return Ok(Some(self.rotation_event(true)));
        }
        if self.transition_accum <= -threshold {
            self.transition_accum = 0;
            return Ok(Some(self.rotation_event(false)));
        }

        Ok(None)
    }

    fn rotation_event(&self, positive_step: bool) -> InputEvent {
        let clockwise = if self.config.direction_inverted {
            !positive_step
        } else {
            positive_step
        };

        if clockwise {
            InputEvent::RotateCw
        } else {
            InputEvent::RotateCcw
        }
    }
}

impl<CLK, DT, SW> InputProvider for RotaryInput<CLK, DT, SW>
where
    CLK: InputPin,
    DT: InputPin,
    SW: InputPin,
{
    type Error = RotaryInputError<CLK::Error, DT::Error, SW::Error>;

    fn poll_event(&mut self) -> Result<Option<InputEvent>, Self::Error> {
        if let Some(event) = self.pending_event.take() {
            return Ok(Some(event));
        }

        let button_event = self.poll_button()?;
        let rotation_event = self.poll_rotation()?;

        match (button_event, rotation_event) {
            (Some(button), Some(rotation)) => {
                self.pending_event = Some(rotation);
                Ok(Some(button))
            }
            (Some(button), None) => Ok(Some(button)),
            (None, Some(rotation)) => Ok(Some(rotation)),
            (None, None) => Ok(None),
        }
    }
}

#[inline]
fn ab_from_levels(clk_high: bool, dt_high: bool) -> u8 {
    ((clk_high as u8) << 1) | (dt_high as u8)
}

#[inline]
fn button_pressed_from_level(sw_high: bool, active_low: bool) -> bool {
    if active_low { !sw_high } else { sw_high }
}
