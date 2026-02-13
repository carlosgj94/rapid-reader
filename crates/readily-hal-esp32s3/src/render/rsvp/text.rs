use super::*;

pub(super) fn wpm_label(wpm: u16, out: &mut [u8; 16]) -> &str {
    let mut len = write_u16_ascii(wpm, out);
    let suffix = b" wpm";

    if len + suffix.len() > out.len() {
        return "wpm";
    }

    out[len..len + suffix.len()].copy_from_slice(suffix);
    len += suffix.len();

    str::from_utf8(&out[..len]).unwrap_or("wpm")
}

pub(super) fn write_u16_ascii(mut value: u16, out: &mut [u8]) -> usize {
    if out.is_empty() {
        return 0;
    }

    if value == 0 {
        out[0] = b'0';
        return 1;
    }

    let mut tmp = [0u8; 5];
    let mut tmp_len = 0usize;
    while value > 0 {
        if tmp_len >= tmp.len() {
            break;
        }
        tmp[tmp_len] = b'0' + (value % 10) as u8;
        tmp_len += 1;
        value /= 10;
    }

    let len = core::cmp::min(tmp_len, out.len());
    for i in 0..len {
        out[i] = tmp[tmp_len - 1 - i];
    }

    len
}

pub(super) fn draw_paragraph_progress(
    frame: &mut FrameBuffer,
    current_word: usize,
    total_words: usize,
    on: bool,
) {
    let bar_x = 12;
    let bar_y = HEIGHT - 18;
    let bar_w = WIDTH - 24;
    let bar_h = 10;

    draw_rect(frame, bar_x, bar_y, bar_w, bar_h, on);

    let clamped_total = total_words.max(1);
    let clamped_current = current_word.min(clamped_total);
    let fill_w = ((bar_w - 2) * clamped_current) / clamped_total;

    if fill_w > 0 {
        draw_filled_rect(frame, bar_x + 1, bar_y + 1, fill_w, bar_h - 2, on);
    }
}

pub(super) fn draw_paragraph_progress_current_marker(
    frame: &mut FrameBuffer,
    current_word: usize,
    total_words: usize,
    on: bool,
) {
    let bar_x = 12;
    let bar_y = HEIGHT - 18;
    let bar_w = WIDTH - 24;
    let clamped_total = total_words.max(1);
    let clamped_current = current_word.min(clamped_total);
    let marker = ((bar_w - 2) * clamped_current) / clamped_total;
    let marker_x = bar_x + 1 + marker;

    draw_filled_rect(
        frame,
        marker_x.saturating_sub(1),
        bar_y.saturating_sub(3),
        3,
        2,
        on,
    );
}

pub(super) fn offset_pos(base: usize, delta: isize) -> usize {
    if delta >= 0 {
        base.saturating_add(delta as usize)
    } else {
        base.saturating_sub((-delta) as usize)
    }
}

pub(super) fn choose_word_scale(word: &str, max_width: usize, size: FontSize) -> usize {
    let candidates: &[usize] = match size {
        FontSize::Small => &[2, 1],
        FontSize::Medium => &[3, 2, 1],
        FontSize::Large => &[5, 4, 3, 2, 1],
    };

    for &scale in candidates {
        if rsvp_word_pixel_width(word, scale) <= max_width {
            return scale;
        }
    }
    1
}

pub(super) struct BookWordRender {
    pub(super) scale: usize,
    pub(super) stride: usize,
}

pub(super) fn rsvp_word_pixel_width(word: &str, scale: usize) -> usize {
    let mut total_cols = 0usize;
    let mut trailing_gap = 0usize;

    for c in word.chars() {
        let normalized = normalize_glyph_char(c);
        let glyph = glyph_5x7(normalized);
        let metrics = glyph_metrics(normalized, &glyph);

        total_cols += metrics.advance;
        trailing_gap = metrics.advance.saturating_sub(metrics.width);
    }

    if total_cols == 0 {
        0
    } else {
        (total_cols - trailing_gap) * scale
    }
}

pub(super) fn draw_text_centered(
    frame: &mut FrameBuffer,
    y: usize,
    text: &str,
    scale: usize,
    on: bool,
) {
    let width = text_pixel_width(text, scale);
    let x = WIDTH.saturating_sub(width) / 2;
    draw_text(frame, x, y, text, scale, on);
}

pub(super) fn text_pixel_width(text: &str, scale: usize) -> usize {
    let chars = text.chars().count();
    if chars == 0 {
        0
    } else {
        chars * (6 * scale) - scale
    }
}

pub(super) fn draw_text(
    frame: &mut FrameBuffer,
    x: usize,
    y: usize,
    text: &str,
    scale: usize,
    on: bool,
) {
    let mut cursor_x = x;

    for c in text.chars() {
        let glyph = glyph_5x7(normalize_glyph_char(c));
        draw_glyph_5x7(frame, cursor_x, y, &glyph, scale, on);
        cursor_x += 6 * scale;
    }
}

pub(super) struct RsvpWordSpec<'a> {
    pub(super) y: usize,
    pub(super) word: &'a str,
    pub(super) scale: usize,
    pub(super) orp_anchor_percent: usize,
    pub(super) serif_word: bool,
    pub(super) stride: usize,
}

pub(super) fn draw_rsvp_word(frame: &mut FrameBuffer, spec: RsvpWordSpec<'_>, on: bool) {
    if spec.serif_word {
        draw_rsvp_word_book(
            frame,
            spec.y,
            spec.word,
            spec.scale,
            spec.stride,
            spec.orp_anchor_percent,
            on,
        );
        return;
    }

    let glyph_count = spec.word.chars().count();
    if glyph_count == 0 {
        return;
    }

    let orp_char_index = rsvp_orp_char_index(spec.word);
    let scale_i = spec.scale as isize;

    let mut cols_before_orp = 0usize;
    let mut orp_width_cols = 1usize;
    let mut cursor_cols = 0usize;

    for (idx, c) in spec.word.chars().enumerate() {
        let normalized = normalize_glyph_char(c);
        let glyph = glyph_5x7(normalized);
        let metrics = glyph_metrics(normalized, &glyph);

        if idx == orp_char_index {
            cols_before_orp = cursor_cols;
            orp_width_cols = metrics.width.max(1);
        }

        cursor_cols += metrics.advance;
    }

    let glyph_width = (orp_width_cols * spec.scale) as isize;
    let orp_anchor_x = ((WIDTH * spec.orp_anchor_percent) / 100) as isize;
    let orp_left = orp_anchor_x - (glyph_width / 2);
    let start_x = orp_left - (cols_before_orp as isize * scale_i);
    let y_signed = spec.y as isize;

    let mut draw_cursor_cols = 0usize;
    for c in spec.word.chars() {
        let normalized = normalize_glyph_char(c);
        let glyph = glyph_5x7(normalized);
        let metrics = glyph_metrics(normalized, &glyph);

        let x = start_x + (draw_cursor_cols as isize * scale_i) - (metrics.left as isize * scale_i);
        draw_glyph_5x7_signed(frame, x, y_signed, &glyph, spec.scale, on);

        draw_cursor_cols += metrics.advance;
    }

    // RSVP fixation marker under the ORP letter.
    let underline_y = spec.y as isize + (7 * spec.scale) as isize + 1;
    let underline_h = core::cmp::max(1, spec.scale / 2) as isize;
    draw_filled_rect_signed(frame, orp_left, underline_y, glyph_width, underline_h, on);
}

pub(super) fn choose_word_scale_book(
    word: &str,
    max_width: usize,
    size: FontSize,
) -> BookWordRender {
    let candidates: &[(usize, usize)] = match size {
        FontSize::Small => &[(1, 2), (1, 1)],
        FontSize::Medium => &[(1, 1)],
        FontSize::Large => &[(2, 1), (1, 1)],
    };

    for &(scale, stride) in candidates {
        if rsvp_word_pixel_width_book(word, scale, stride) <= max_width {
            return BookWordRender { scale, stride };
        }
    }

    BookWordRender {
        scale: 1,
        stride: 1,
    }
}

pub(super) fn rsvp_word_pixel_width_book(word: &str, scale: usize, stride: usize) -> usize {
    let stride = stride.max(1);
    let mut total_cols = 0usize;
    let mut trailing_gap = 0usize;

    for c in word.chars() {
        let glyph = book_font::glyph(normalize_glyph_char(c));
        let advance = (glyph.advance as usize).div_ceil(stride);
        let width = (glyph.width as usize).div_ceil(stride);
        total_cols += advance;
        trailing_gap = advance.saturating_sub(width);
    }

    if total_cols == 0 {
        0
    } else {
        (total_cols - trailing_gap) * scale
    }
}

pub(super) fn draw_rsvp_word_book(
    frame: &mut FrameBuffer,
    y: usize,
    word: &str,
    scale: usize,
    stride: usize,
    orp_anchor_percent: usize,
    on: bool,
) {
    let glyph_count = word.chars().count();
    if glyph_count == 0 {
        return;
    }

    let stride = stride.max(1);
    let orp_char_index = rsvp_orp_char_index(word);
    let scale_i = scale as isize;
    let stride_i = stride as isize;

    let mut cols_before_orp = 0usize;
    let mut orp_width_cols = 1usize;
    let mut cursor_cols = 0usize;

    for (idx, c) in word.chars().enumerate() {
        let glyph = book_font::glyph(normalize_glyph_char(c));
        let advance = (glyph.advance as usize).div_ceil(stride);
        let width = (glyph.width as usize).div_ceil(stride);

        if idx == orp_char_index {
            cols_before_orp = cursor_cols;
            orp_width_cols = width.max(1);
        }

        cursor_cols += advance;
    }

    let glyph_width = (orp_width_cols * scale) as isize;
    let orp_anchor_x = ((WIDTH * orp_anchor_percent) / 100) as isize;
    let orp_left = orp_anchor_x - (glyph_width / 2);
    let start_x = orp_left - (cols_before_orp as isize * scale_i);
    let y_signed = y as isize;

    let mut draw_cursor_cols = 0usize;
    for c in word.chars() {
        let glyph = book_font::glyph(normalize_glyph_char(c));
        let left = (glyph.left as isize) / stride_i;
        let advance = (glyph.advance as usize).div_ceil(stride);
        let x = start_x + (draw_cursor_cols as isize * scale_i) + (left * scale_i);
        draw_book_glyph_signed(frame, x, y_signed, glyph, scale, stride, on);
        draw_cursor_cols += advance;
    }

    let scaled_height = book_font::FONT_HEIGHT.div_ceil(stride) as isize;
    let underline_y = y as isize + (scaled_height * scale_i) + 1;
    let underline_h = core::cmp::max(1, scale / 2) as isize;
    draw_filled_rect_signed(frame, orp_left, underline_y, glyph_width, underline_h, on);
}

pub(super) fn rsvp_orp_char_index(word: &str) -> usize {
    let mut total_chars = 0usize;
    let mut letter_chars = 0usize;

    for c in word.chars() {
        total_chars += 1;
        if is_rsvp_letter(c) {
            letter_chars += 1;
        }
    }

    if total_chars == 0 {
        return 0;
    }

    if letter_chars == 0 {
        return total_chars.saturating_sub(1) / 2;
    }

    let target_letter = core::cmp::min(rsvp_orp_letter_index(letter_chars), letter_chars - 1);
    let mut current_letter = 0usize;

    for (current_char, c) in word.chars().enumerate() {
        if is_rsvp_letter(c) {
            if current_letter == target_letter {
                return current_char;
            }
            current_letter += 1;
        }
    }

    total_chars.saturating_sub(1) / 2
}

pub(super) fn rsvp_orp_letter_index(letter_count: usize) -> usize {
    match letter_count {
        0 | 1 => 0,
        2..=5 => 1,
        6..=9 => 2,
        10..=13 => 3,
        _ => 4,
    }
}

pub(super) fn is_rsvp_letter(c: char) -> bool {
    normalize_glyph_char(c).is_ascii_alphanumeric()
}
