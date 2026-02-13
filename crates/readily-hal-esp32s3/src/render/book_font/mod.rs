// Auto-generated serif bitmap font for RSVP word rendering.
// Source font: NimbusRoman-Regular.otf
// Raster size: 42px

pub const FONT_HEIGHT: usize = 43;

#[derive(Clone, Copy)]
pub struct SerifGlyph {
    pub left: i8,
    pub width: u8,
    pub advance: u8,
    pub rows: [u64; FONT_HEIGHT],
}

mod data;

use data::GLYPHS;

pub fn glyph(c: char) -> &'static SerifGlyph {
    let idx = c as usize;
    if (32..=126).contains(&idx) {
        &GLYPHS[idx - 32]
    } else {
        &GLYPHS[(b'?' - 32) as usize]
    }
}
