//! In-memory framebuffer for LS027B7DH01.

use core::convert::TryFrom;

use crate::protocol::{BUFFER_SIZE, HEIGHT, LINE_BYTES, WIDTH};

/// 1bpp framebuffer for the panel.
///
/// Bit mapping within one line byte: bit 7 is the first pixel in that byte.
#[derive(Clone)]
pub struct FrameBuffer {
    bytes: [u8; BUFFER_SIZE],
}

impl Default for FrameBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameBuffer {
    /// Creates a new white framebuffer.
    pub const fn new() -> Self {
        Self {
            bytes: [0u8; BUFFER_SIZE],
        }
    }

    /// Returns the underlying framebuffer bytes.
    pub fn bytes(&self) -> &[u8; BUFFER_SIZE] {
        &self.bytes
    }

    /// Returns mutable framebuffer bytes.
    pub fn bytes_mut(&mut self) -> &mut [u8; BUFFER_SIZE] {
        &mut self.bytes
    }

    /// Clears framebuffer to white (`on = false`) or black (`on = true`).
    pub fn clear(&mut self, on: bool) {
        self.bytes.fill(if on { 0xFF } else { 0x00 });
    }

    /// Sets a pixel state.
    ///
    /// Returns `true` when pixel is in bounds, `false` otherwise.
    pub fn set_pixel(&mut self, x: usize, y: usize, on: bool) -> bool {
        if x >= WIDTH || y >= HEIGHT {
            return false;
        }

        let byte_index = y * LINE_BYTES + (x / 8);
        let bit_mask = 1u8 << (7 - (x % 8));

        if on {
            self.bytes[byte_index] |= bit_mask;
        } else {
            self.bytes[byte_index] &= !bit_mask;
        }

        true
    }

    /// Reads a pixel state.
    pub fn pixel(&self, x: usize, y: usize) -> Option<bool> {
        if x >= WIDTH || y >= HEIGHT {
            return None;
        }

        let byte_index = y * LINE_BYTES + (x / 8);
        let bit_mask = 1u8 << (7 - (x % 8));
        Some((self.bytes[byte_index] & bit_mask) != 0)
    }

    /// Returns a line payload for line 1..=240.
    pub fn line(&self, line: u16) -> Option<&[u8; LINE_BYTES]> {
        if !(1..=HEIGHT as u16).contains(&line) {
            return None;
        }

        let start = (line as usize - 1) * LINE_BYTES;
        let end = start + LINE_BYTES;
        <&[u8; LINE_BYTES]>::try_from(&self.bytes[start..end]).ok()
    }

    /// Overwrites a line payload for line 1..=240.
    pub fn set_line(&mut self, line: u16, data: &[u8; LINE_BYTES]) -> bool {
        if !(1..=HEIGHT as u16).contains(&line) {
            return false;
        }

        let start = (line as usize - 1) * LINE_BYTES;
        let end = start + LINE_BYTES;
        self.bytes[start..end].copy_from_slice(data);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixel_bit_mapping_is_msb_first_within_byte() {
        let mut fb = FrameBuffer::new();

        assert!(fb.set_pixel(0, 0, true));
        assert!(fb.set_pixel(7, 0, true));
        assert!(fb.set_pixel(8, 0, true));

        let line1 = fb.line(1).unwrap();
        assert_eq!(line1[0], 0b1000_0001);
        assert_eq!(line1[1], 0b1000_0000);
    }

    #[test]
    fn out_of_bounds_pixel_is_ignored() {
        let mut fb = FrameBuffer::new();

        assert!(!fb.set_pixel(WIDTH, 0, true));
        assert!(!fb.set_pixel(0, HEIGHT, true));
        assert_eq!(fb.bytes()[0], 0x00);
    }

    #[test]
    fn set_and_read_last_pixel() {
        let mut fb = FrameBuffer::new();

        assert!(fb.set_pixel(WIDTH - 1, HEIGHT - 1, true));
        assert_eq!(fb.pixel(WIDTH - 1, HEIGHT - 1), Some(true));
        assert_eq!(fb.pixel(WIDTH, HEIGHT), None);
    }
}
