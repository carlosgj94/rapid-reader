//! In-memory framebuffer for LS027B7DH01.

use core::convert::TryFrom;

use crate::{
    DirtyRows,
    protocol::{BUFFER_SIZE, HEIGHT, LINE_BYTES, WIDTH},
};

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

    /// Inverts the framebuffer in place.
    pub fn invert(&mut self) {
        for byte in &mut self.bytes {
            *byte = !*byte;
        }
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

    /// Returns a zero-based row payload.
    pub fn row(&self, row: usize) -> Option<&[u8; LINE_BYTES]> {
        if row >= HEIGHT {
            return None;
        }

        let start = row * LINE_BYTES;
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

    /// Copies only the dirty rows from `other`.
    pub fn copy_dirty_rows_from(&mut self, other: &Self, dirty_rows: &DirtyRows) {
        for span in dirty_rows.iter_spans() {
            let start = span.start_row * LINE_BYTES;
            let end = (span.end_row + 1) * LINE_BYTES;
            self.bytes[start..end].copy_from_slice(&other.bytes[start..end]);
        }
    }

    /// Fills a clipped horizontal span.
    pub fn fill_span(&mut self, x: i32, y: i32, width: i32, on: bool) {
        if width <= 0 || y < 0 || y >= HEIGHT as i32 {
            return;
        }

        let start_x = x.max(0) as usize;
        let end_x = (x + width).min(WIDTH as i32).max(0) as usize;
        if start_x >= end_x {
            return;
        }

        let row_start = y as usize * LINE_BYTES;
        let start_byte = start_x / 8;
        let end_byte = (end_x - 1) / 8;
        let start_mask = 0xFFu8 >> (start_x % 8);
        let end_mask = match end_x % 8 {
            0 => 0xFF,
            remainder => 0xFFu8 << (8 - remainder),
        };

        if start_byte == end_byte {
            let mask = start_mask & end_mask;
            if on {
                self.bytes[row_start + start_byte] |= mask;
            } else {
                self.bytes[row_start + start_byte] &= !mask;
            }
            return;
        }

        if on {
            self.bytes[row_start + start_byte] |= start_mask;
        } else {
            self.bytes[row_start + start_byte] &= !start_mask;
        }

        for byte_index in (start_byte + 1)..end_byte {
            self.bytes[row_start + byte_index] = if on { 0xFF } else { 0x00 };
        }

        if on {
            self.bytes[row_start + end_byte] |= end_mask;
        } else {
            self.bytes[row_start + end_byte] &= !end_mask;
        }
    }

    /// Fills a clipped rectangle.
    pub fn fill_rect(&mut self, x: i32, y: i32, width: i32, height: i32, on: bool) {
        if width <= 0 || height <= 0 {
            return;
        }

        let start_y = y.max(0) as usize;
        let end_y = (y + height).min(HEIGHT as i32).max(0) as usize;
        for row in start_y..end_y {
            self.fill_span(x, row as i32, width, on);
        }
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

    #[test]
    fn fill_span_sets_expected_bit_range() {
        let mut fb = FrameBuffer::new();

        fb.fill_span(3, 0, 10, true);

        let line = fb.row(0).unwrap();
        assert_eq!(line[0], 0b0001_1111);
        assert_eq!(line[1], 0b1111_1000);
    }

    #[test]
    fn copy_dirty_rows_updates_only_selected_rows() {
        let mut source = FrameBuffer::new();
        let mut target = FrameBuffer::new();
        let mut dirty = DirtyRows::new();

        source.fill_rect(0, 4, 16, 2, true);
        let _ = dirty.mark_row(4);
        let _ = dirty.mark_row(5);
        target.copy_dirty_rows_from(&source, &dirty);

        assert_eq!(target.row(4), source.row(4));
        assert_eq!(target.row(5), source.row(5));
        assert_eq!(target.row(6).unwrap(), &[0u8; LINE_BYTES]);
    }
}
