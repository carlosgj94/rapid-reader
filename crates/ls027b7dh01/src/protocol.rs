//! Wire-level protocol helpers for LS027B7DH01.

/// Panel width in pixels.
pub const WIDTH: usize = 400;
/// Panel height in pixels.
pub const HEIGHT: usize = 240;
/// Number of bytes in one display line.
pub const LINE_BYTES: usize = WIDTH / 8;
/// Total framebuffer size in bytes.
pub const BUFFER_SIZE: usize = LINE_BYTES * HEIGHT;

/// Packet size for a one-line write command.
///
/// Layout:
/// - 1 byte mode + dummy
/// - 1 byte gate address
/// - 50 bytes pixel payload
/// - 2 bytes transfer dummy
pub const WRITE_LINE_PACKET_SIZE: usize = 1 + 1 + LINE_BYTES + 2;

/// Packet size for all-clear.
///
/// Layout:
/// - 1 byte mode + dummy
/// - >=13 dummy bits (sent as 2 bytes)
pub const CLEAR_PACKET_SIZE: usize = 3;

#[inline]
const fn mode_byte(m0: bool, m1: bool, m2: bool) -> u8 {
    ((m0 as u8) << 7) | ((m1 as u8) << 6) | ((m2 as u8) << 5)
}

/// Builds a wire address byte (`AG0..AG7`) for line 1..=240.
///
/// Returns `None` for invalid line numbers.
#[inline]
pub fn encode_line_address(line: u16) -> Option<u8> {
    if !(1..=HEIGHT as u16).contains(&line) {
        return None;
    }

    Some((line as u8).reverse_bits())
}

/// Builds the all-clear command packet.
#[inline]
pub fn build_clear_packet(m1_high: bool) -> [u8; CLEAR_PACKET_SIZE] {
    [mode_byte(false, m1_high, true), 0x00, 0x00]
}

/// Builds a one-line update command packet.
///
/// Returns `None` when `line` is out of range.
#[inline]
pub fn build_write_line_packet(
    line: u16,
    line_data: &[u8; LINE_BYTES],
    m1_high: bool,
) -> Option<[u8; WRITE_LINE_PACKET_SIZE]> {
    let address = encode_line_address(line)?;

    let mut packet = [0u8; WRITE_LINE_PACKET_SIZE];
    packet[0] = mode_byte(true, m1_high, false);
    packet[1] = address;
    packet[2..2 + LINE_BYTES].copy_from_slice(line_data);

    Some(packet)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_bits_match_expected_bytes() {
        assert_eq!(
            build_write_line_packet(1, &[0; LINE_BYTES], false).unwrap()[0],
            0x80
        );
        assert_eq!(build_clear_packet(false)[0], 0x20);
        assert_eq!(
            build_write_line_packet(1, &[0; LINE_BYTES], true).unwrap()[0],
            0xC0
        );
    }

    #[test]
    fn line_address_encoding_matches_datasheet_table() {
        assert_eq!(encode_line_address(1), Some(0x80));
        assert_eq!(encode_line_address(2), Some(0x40));
        assert_eq!(encode_line_address(3), Some(0xC0));
        assert_eq!(encode_line_address(238), Some(0x77));
        assert_eq!(encode_line_address(239), Some(0xF7));
        assert_eq!(encode_line_address(240), Some(0x0F));
    }

    #[test]
    fn invalid_line_is_rejected() {
        assert_eq!(encode_line_address(0), None);
        assert_eq!(encode_line_address(241), None);
    }

    #[test]
    fn write_line_packet_shape_is_fixed() {
        let mut data = [0u8; LINE_BYTES];
        data[0] = 0xAA;
        data[LINE_BYTES - 1] = 0x55;

        let packet = build_write_line_packet(10, &data, false).unwrap();
        assert_eq!(packet.len(), WRITE_LINE_PACKET_SIZE);
        assert_eq!(packet[0], 0x80);
        assert_eq!(packet[1], (10u8).reverse_bits());
        assert_eq!(packet[2], 0xAA);
        assert_eq!(packet[2 + LINE_BYTES - 1], 0x55);
        assert_eq!(packet[WRITE_LINE_PACKET_SIZE - 2], 0x00);
        assert_eq!(packet[WRITE_LINE_PACKET_SIZE - 1], 0x00);
    }
}
