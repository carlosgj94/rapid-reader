use super::*;

pub(super) struct GlyphMetrics {
    pub(super) left: usize,
    pub(super) width: usize,
    pub(super) advance: usize,
}

pub(super) fn glyph_metrics(c: char, glyph: &[u8; 5]) -> GlyphMetrics {
    if c == ' ' {
        return GlyphMetrics {
            left: 0,
            width: 0,
            advance: 3,
        };
    }

    let mut left = 5usize;
    let mut right = 0usize;

    for (col, bits) in glyph.iter().enumerate() {
        if *bits != 0 {
            left = left.min(col);
            right = right.max(col);
        }
    }

    if left > right {
        return GlyphMetrics {
            left: 0,
            width: 1,
            advance: 2,
        };
    }

    let width = right - left + 1;
    let advance = width + glyph_spacing(c);

    GlyphMetrics {
        left,
        width,
        advance,
    }
}

pub(super) fn glyph_spacing(c: char) -> usize {
    match c {
        '.' | ',' | ';' | ':' | '!' | '?' | '\'' => 1,
        _ => 1,
    }
}

pub(super) fn draw_glyph_5x7(
    frame: &mut FrameBuffer,
    x: usize,
    y: usize,
    glyph: &[u8; 5],
    scale: usize,
    on: bool,
) {
    for (col, bits) in glyph.iter().enumerate() {
        for row in 0..7 {
            if (bits & (1 << row)) != 0 {
                let px = x + col * scale;
                let py = y + row * scale;
                draw_filled_rect(frame, px, py, scale, scale, on);
            }
        }
    }
}

pub(super) fn draw_glyph_5x7_signed(
    frame: &mut FrameBuffer,
    x: isize,
    y: isize,
    glyph: &[u8; 5],
    scale: usize,
    on: bool,
) {
    let scale_i = scale as isize;

    for (col, bits) in glyph.iter().enumerate() {
        for row in 0..7 {
            if (bits & (1 << row)) != 0 {
                let base_x = x + col as isize * scale_i;
                let base_y = y + row as isize * scale_i;

                for dy in 0..scale_i {
                    for dx in 0..scale_i {
                        set_pixel_signed(frame, base_x + dx, base_y + dy, on);
                    }
                }
            }
        }
    }
}

pub(super) fn draw_scaled_pixel_signed(
    frame: &mut FrameBuffer,
    x: isize,
    y: isize,
    scale: usize,
    on: bool,
) {
    let scale_i = scale as isize;

    for dy in 0..scale_i {
        for dx in 0..scale_i {
            set_pixel_signed(frame, x + dx, y + dy, on);
        }
    }
}

pub(super) fn draw_book_glyph_signed(
    frame: &mut FrameBuffer,
    x: isize,
    y: isize,
    glyph: &book_font::SerifGlyph,
    scale: usize,
    stride: usize,
    on: bool,
) {
    let scale_i = scale as isize;
    let stride = stride.max(1);
    let width = glyph.width as usize;

    for (out_row, row_idx) in (0..glyph.rows.len()).step_by(stride).enumerate() {
        let bits = glyph.rows[row_idx];
        let py = y + (out_row as isize * scale_i);

        for (out_col, col_idx) in (0..width).step_by(stride).enumerate() {
            if (bits & (1u64 << col_idx)) != 0 {
                let px = x + (out_col as isize * scale_i);
                draw_scaled_pixel_signed(frame, px, py, scale, on);
            }
        }
    }
}

pub(super) fn normalize_glyph_char(c: char) -> char {
    match c {
        'á' | 'à' | 'ä' | 'â' | 'ã' => 'a',
        'Á' | 'À' | 'Ä' | 'Â' | 'Ã' => 'A',
        'é' | 'è' | 'ë' | 'ê' => 'e',
        'É' | 'È' | 'Ë' | 'Ê' => 'E',
        'í' | 'ì' | 'ï' | 'î' => 'i',
        'Í' | 'Ì' | 'Ï' | 'Î' => 'I',
        'ó' | 'ò' | 'ö' | 'ô' | 'õ' => 'o',
        'Ó' | 'Ò' | 'Ö' | 'Ô' | 'Õ' => 'O',
        'ú' | 'ù' | 'ü' | 'û' => 'u',
        'Ú' | 'Ù' | 'Ü' | 'Û' => 'U',
        'ñ' => 'n',
        'Ñ' => 'N',
        'ç' => 'c',
        'Ç' => 'C',
        '\'' | '’' | '‘' | '‚' | '‛' | 'ʼ' | 'ʻ' | '´' | '`' => '\'',
        '"' | '“' | '”' | '„' | '‟' => '"',
        '-' | '‐' | '‑' | '‒' | '–' | '—' | '―' => '-',
        '…' => '.',
        _ => c,
    }
}

pub(super) fn glyph_5x7(c: char) -> [u8; 5] {
    match c {
        'A' => [0x7E, 0x11, 0x11, 0x11, 0x7E],
        'B' => [0x7F, 0x49, 0x49, 0x49, 0x36],
        'C' => [0x3E, 0x41, 0x41, 0x41, 0x22],
        'D' => [0x7F, 0x41, 0x41, 0x22, 0x1C],
        'E' => [0x7F, 0x49, 0x49, 0x49, 0x41],
        'F' => [0x7F, 0x09, 0x09, 0x09, 0x01],
        'G' => [0x3E, 0x41, 0x49, 0x49, 0x7A],
        'H' => [0x7F, 0x08, 0x08, 0x08, 0x7F],
        'I' => [0x00, 0x41, 0x7F, 0x41, 0x00],
        'J' => [0x20, 0x40, 0x41, 0x3F, 0x01],
        'K' => [0x7F, 0x08, 0x14, 0x22, 0x41],
        'L' => [0x7F, 0x40, 0x40, 0x40, 0x40],
        'M' => [0x7F, 0x02, 0x0C, 0x02, 0x7F],
        'N' => [0x7F, 0x04, 0x08, 0x10, 0x7F],
        'O' => [0x3E, 0x41, 0x41, 0x41, 0x3E],
        'P' => [0x7F, 0x09, 0x09, 0x09, 0x06],
        'Q' => [0x3E, 0x41, 0x51, 0x21, 0x5E],
        'R' => [0x7F, 0x09, 0x19, 0x29, 0x46],
        'S' => [0x46, 0x49, 0x49, 0x49, 0x31],
        'T' => [0x01, 0x01, 0x7F, 0x01, 0x01],
        'U' => [0x3F, 0x40, 0x40, 0x40, 0x3F],
        'V' => [0x1F, 0x20, 0x40, 0x20, 0x1F],
        'W' => [0x7F, 0x20, 0x18, 0x20, 0x7F],
        'X' => [0x63, 0x14, 0x08, 0x14, 0x63],
        'Y' => [0x03, 0x04, 0x78, 0x04, 0x03],
        'Z' => [0x61, 0x51, 0x49, 0x45, 0x43],
        'a' => [0x20, 0x54, 0x54, 0x54, 0x78],
        'b' => [0x7F, 0x48, 0x44, 0x44, 0x38],
        'c' => [0x38, 0x44, 0x44, 0x44, 0x20],
        'd' => [0x38, 0x44, 0x44, 0x48, 0x7F],
        'e' => [0x38, 0x54, 0x54, 0x54, 0x18],
        'f' => [0x08, 0x7E, 0x09, 0x01, 0x02],
        'g' => [0x08, 0x14, 0x54, 0x54, 0x3C],
        'h' => [0x7F, 0x08, 0x04, 0x04, 0x78],
        'i' => [0x00, 0x44, 0x7D, 0x40, 0x00],
        'j' => [0x20, 0x40, 0x44, 0x3D, 0x00],
        'k' => [0x7F, 0x10, 0x28, 0x44, 0x00],
        'l' => [0x00, 0x41, 0x7F, 0x40, 0x00],
        'm' => [0x7C, 0x04, 0x18, 0x04, 0x78],
        'n' => [0x7C, 0x08, 0x04, 0x04, 0x78],
        'o' => [0x38, 0x44, 0x44, 0x44, 0x38],
        'p' => [0x7C, 0x14, 0x14, 0x14, 0x08],
        'q' => [0x08, 0x14, 0x14, 0x18, 0x7C],
        'r' => [0x7C, 0x08, 0x04, 0x04, 0x08],
        's' => [0x48, 0x54, 0x54, 0x54, 0x20],
        't' => [0x04, 0x3F, 0x44, 0x40, 0x20],
        'u' => [0x3C, 0x40, 0x40, 0x20, 0x7C],
        'v' => [0x1C, 0x20, 0x40, 0x20, 0x1C],
        'w' => [0x3C, 0x40, 0x30, 0x40, 0x3C],
        'x' => [0x44, 0x28, 0x10, 0x28, 0x44],
        'y' => [0x0C, 0x50, 0x50, 0x50, 0x3C],
        'z' => [0x44, 0x64, 0x54, 0x4C, 0x44],
        '0' => [0x3E, 0x51, 0x49, 0x45, 0x3E],
        '1' => [0x00, 0x42, 0x7F, 0x40, 0x00],
        '2' => [0x42, 0x61, 0x51, 0x49, 0x46],
        '3' => [0x21, 0x41, 0x45, 0x4B, 0x31],
        '4' => [0x18, 0x14, 0x12, 0x7F, 0x10],
        '5' => [0x27, 0x45, 0x45, 0x45, 0x39],
        '6' => [0x3C, 0x4A, 0x49, 0x49, 0x30],
        '7' => [0x01, 0x71, 0x09, 0x05, 0x03],
        '8' => [0x36, 0x49, 0x49, 0x49, 0x36],
        '9' => [0x06, 0x49, 0x49, 0x29, 0x1E],
        '.' => [0x00, 0x60, 0x60, 0x00, 0x00],
        ',' => [0x00, 0x80, 0x60, 0x00, 0x00],
        ';' => [0x00, 0x80, 0x66, 0x00, 0x00],
        '/' => [0x20, 0x10, 0x08, 0x04, 0x02],
        '<' => [0x08, 0x14, 0x22, 0x41, 0x00],
        '>' => [0x00, 0x41, 0x22, 0x14, 0x08],
        '[' => [0x00, 0x7F, 0x41, 0x41, 0x00],
        ']' => [0x00, 0x41, 0x41, 0x7F, 0x00],
        '-' => [0x08, 0x08, 0x08, 0x08, 0x08],
        ':' => [0x00, 0x36, 0x36, 0x00, 0x00],
        ' ' => [0x00, 0x00, 0x00, 0x00, 0x00],
        _ => [0x00, 0x00, 0x5F, 0x00, 0x00],
    }
}
