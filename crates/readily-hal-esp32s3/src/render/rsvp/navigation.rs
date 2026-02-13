use super::*;

pub(super) struct NavigationSelectorSpec<'a> {
    pub(super) mode_label: &'a str,
    pub(super) context_label: &'a str,
    pub(super) current_primary: &'a str,
    pub(super) current_secondary: &'a str,
    pub(super) target_primary: &'a str,
    pub(super) target_secondary: &'a str,
    pub(super) target_index: usize,
    pub(super) total: usize,
    pub(super) animation: Option<AnimationFrame>,
}

pub(super) fn draw_navigation_selector(
    frame: &mut FrameBuffer,
    spec: NavigationSelectorSpec<'_>,
    on: bool,
) {
    let motion = selector_motion_dx(spec.animation);
    let paragraph_mode = spec.mode_label == "PARAGRAPH SELECT";
    draw_text(frame, 12, 44, spec.mode_label, 1, on);
    if !spec.context_label.is_empty() {
        let mut context_buf = [0u8; SELECTOR_TEXT_BUF];
        let context_fit = fit_text_in_width(
            spec.context_label,
            WIDTH.saturating_sub(24),
            1,
            &mut context_buf,
        );
        draw_text(frame, 12, 56, context_fit, 1, on);
    }

    if paragraph_mode {
        let focus = paragraph_focus_pct(spec.animation);
        let side_w = 74usize.saturating_sub((focus * 56) / 100);
        let side_h = 44usize.saturating_sub((focus * 24) / 100);
        if side_w >= 26 && side_h >= 20 {
            let side_x = clamp_cover_x(
                offset_pos(8, motion - ((focus as isize * 24) / 100)),
                side_w,
                0,
            );
            draw_selector_card(
                frame,
                SelectorCardSpec {
                    x: side_x,
                    y: 78,
                    w: side_w,
                    h: side_h,
                    primary: spec.current_primary,
                    secondary: "",
                    emphasized: false,
                    primary_lines: 1,
                    primary_scale_hint: 2,
                },
                on,
            );
        }

        let base_w = WIDTH.saturating_sub(98);
        let target_w = core::cmp::min(
            WIDTH.saturating_sub(16),
            base_w.saturating_add((focus * 44) / 100),
        );
        let target_h = 82usize.saturating_add((focus * 18) / 100);
        let base_x = WIDTH.saturating_sub(target_w + 8);
        let target_x = clamp_cover_x(offset_pos(base_x, motion / 4), target_w, 6);
        draw_selector_card(
            frame,
            SelectorCardSpec {
                x: target_x,
                y: 80,
                w: target_w,
                h: target_h,
                primary: spec.target_primary,
                secondary: spec.target_secondary,
                emphasized: true,
                primary_lines: 4,
                primary_scale_hint: 2,
            },
            on,
        );
    } else {
        draw_selector_card(
            frame,
            SelectorCardSpec {
                x: 12,
                y: 64,
                w: 90,
                h: 50,
                primary: spec.current_primary,
                secondary: spec.current_secondary,
                emphasized: false,
                primary_lines: 1,
                primary_scale_hint: 0,
            },
            on,
        );

        let target_w = WIDTH.saturating_sub(122);
        let target_h = 68usize;
        let base_x = WIDTH.saturating_sub(target_w + 14);
        let target_x = clamp_cover_x(offset_pos(base_x, motion), target_w, 8);
        draw_selector_card(
            frame,
            SelectorCardSpec {
                x: target_x,
                y: 56,
                w: target_w,
                h: target_h,
                primary: spec.target_primary,
                secondary: spec.target_secondary,
                emphasized: true,
                primary_lines: 1,
                primary_scale_hint: 0,
            },
            on,
        );
    }

    let timeline_y = if paragraph_mode { 188 } else { 136 };
    draw_timeline_strip(frame, spec.target_index, spec.total, motion, timeline_y, on);
    let footer_y = if paragraph_mode {
        HEIGHT.saturating_sub(30)
    } else {
        HEIGHT - 34
    };
    draw_text_centered(frame, footer_y, "Rotate browse  Press confirm", 1, on);
}

#[derive(Clone, Copy, Debug)]
pub(super) struct SelectorCardSpec<'a> {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    primary: &'a str,
    secondary: &'a str,
    emphasized: bool,
    primary_lines: usize,
    primary_scale_hint: usize,
}

pub(super) fn draw_selector_card(frame: &mut FrameBuffer, spec: SelectorCardSpec<'_>, on: bool) {
    let x = spec.x;
    let y = spec.y;
    let w = spec.w;
    let h = spec.h;
    if w < 8 || h < 8 {
        return;
    }

    draw_rect(frame, x, y, w, h, on);
    if spec.emphasized && w > 4 && h > 4 {
        draw_rect(frame, x + 2, y + 2, w - 4, h - 4, on);
    }

    let side_pad = if spec.emphasized && spec.primary_lines > 1 {
        4
    } else {
        8
    };
    let text_w = w.saturating_sub(side_pad * 2);
    let primary_scale = if spec.primary_lines <= 1 {
        if spec.primary_scale_hint > 0
            && text_pixel_width(spec.primary, spec.primary_scale_hint) <= text_w
        {
            spec.primary_scale_hint
        } else if spec.emphasized && text_pixel_width(spec.primary, 2) <= text_w {
            2
        } else {
            1
        }
    } else {
        spec.primary_scale_hint.max(1)
    };
    let mut secondary_buf = [0u8; SELECTOR_TEXT_BUF];
    let secondary_fit = fit_text_in_width(spec.secondary, text_w, 1, &mut secondary_buf);

    if spec.primary_lines <= 1 {
        let line_y = y + h / 2;
        let sep_pad = side_pad + 2;
        if w > sep_pad * 2 {
            draw_filled_rect(frame, x + sep_pad, line_y, w - sep_pad * 2, 1, on);
        }

        let mut primary_buf = [0u8; SELECTOR_TEXT_BUF];
        let primary_fit = fit_text_in_width(spec.primary, text_w, primary_scale, &mut primary_buf);

        draw_text_centered_in_box(
            frame,
            BoxTextSpec {
                x: x + side_pad,
                y: y + 6,
                w: text_w,
                h: 14,
                text: primary_fit,
                scale: primary_scale,
            },
            on,
        );
        draw_text_centered_in_box(
            frame,
            BoxTextSpec {
                x: x + side_pad,
                y: line_y + 5,
                w: text_w,
                h: h.saturating_sub(line_y.saturating_sub(y) + 10),
                text: secondary_fit,
                scale: 1,
            },
            on,
        );
        return;
    }

    let lines_cap = spec.primary_lines.clamp(2, 4);
    let mut line_raw = [[0u8; SELECTOR_TEXT_BUF]; 4];
    let mut line_fit = [[0u8; SELECTOR_TEXT_BUF]; 4];
    let mut line_len = [0usize; 4];
    let lines_used = wrap_preview_lines(
        spec.primary,
        lines_cap,
        text_w,
        primary_scale,
        &mut line_raw,
        &mut line_fit,
        &mut line_len,
    );

    let line_h = 7 * primary_scale;
    let gap = 2usize;
    let first_y = y + 6;

    for i in 0..lines_used {
        let line = str::from_utf8(&line_fit[i][..line_len[i]]).unwrap_or("");
        let ly = first_y + i * (line_h + gap);
        draw_text_centered_in_box(
            frame,
            BoxTextSpec {
                x: x + side_pad,
                y: ly,
                w: text_w,
                h: line_h,
                text: line,
                scale: primary_scale,
            },
            on,
        );
    }

    let sep_y = y + h.saturating_sub(18);
    let sep_pad = side_pad + 2;
    if w > sep_pad * 2 {
        draw_filled_rect(frame, x + sep_pad, sep_y, w - sep_pad * 2, 1, on);
    }
    draw_text_centered_in_box(
        frame,
        BoxTextSpec {
            x: x + side_pad,
            y: sep_y + 3,
            w: text_w,
            h: 12,
            text: secondary_fit,
            scale: 1,
        },
        on,
    );
}

pub(super) fn wrap_preview_lines(
    source: &str,
    lines_cap: usize,
    max_width: usize,
    scale: usize,
    raw_lines: &mut [[u8; SELECTOR_TEXT_BUF]; 4],
    fit_lines: &mut [[u8; SELECTOR_TEXT_BUF]; 4],
    line_lens: &mut [usize; 4],
) -> usize {
    let mut words = [""; 48];
    let mut word_count = 0usize;
    for word in source.split_whitespace() {
        if word_count >= words.len() {
            break;
        }
        words[word_count] = word;
        word_count += 1;
    }

    if word_count == 0 || lines_cap == 0 {
        return 0;
    }

    let mut line_idx = 0usize;
    let mut cursor = 0usize;

    while line_idx < lines_cap && cursor < word_count {
        let start = cursor;
        let mut end = start;
        let mut best_end = start;

        while end < word_count {
            let candidate = join_words_into_buf(&words[start..=end], &mut raw_lines[line_idx]);
            if text_pixel_width(candidate, scale) <= max_width {
                best_end = end + 1;
                end += 1;
            } else {
                break;
            }
        }

        if best_end == start {
            // Single word too wide: hard-fit it to one line.
            let fit = fit_text_in_width(words[cursor], max_width, scale, &mut fit_lines[line_idx]);
            line_lens[line_idx] = fit.len();
            cursor += 1;
            line_idx += 1;
            continue;
        }

        let raw = join_words_into_buf(&words[start..best_end], &mut raw_lines[line_idx]);
        let fit = fit_text_in_width(raw, max_width, scale, &mut fit_lines[line_idx]);
        line_lens[line_idx] = fit.len();
        cursor = best_end;
        line_idx += 1;
    }

    if cursor < word_count && line_idx > 0 {
        let last = line_idx - 1;
        line_lens[last] =
            append_ellipsis_in_place(&mut fit_lines[last], line_lens[last], max_width, scale);
    }

    line_idx
}

pub(super) fn append_ellipsis_in_place(
    buf: &mut [u8; SELECTOR_TEXT_BUF],
    mut len: usize,
    max_width: usize,
    scale: usize,
) -> usize {
    const ELLIPSIS: &[u8; 3] = b"...";
    let ellipsis_width = text_pixel_width("...", scale);
    if ellipsis_width > max_width || len + ELLIPSIS.len() > buf.len() {
        return len;
    }

    while len > 0 {
        let current = str::from_utf8(&buf[..len]).unwrap_or("");
        if text_pixel_width(current, scale) + ellipsis_width <= max_width {
            break;
        }

        let mut next_len = len - 1;
        while next_len > 0 && (buf[next_len] & 0b1100_0000) == 0b1000_0000 {
            next_len -= 1;
        }
        len = next_len;
    }

    if len + ELLIPSIS.len() <= buf.len() {
        buf[len..len + ELLIPSIS.len()].copy_from_slice(ELLIPSIS);
        len += ELLIPSIS.len();
    }

    len
}

pub(super) fn join_words_into_buf<'a>(
    words: &[&str],
    out: &'a mut [u8; SELECTOR_TEXT_BUF],
) -> &'a str {
    if words.is_empty() || out.is_empty() {
        return "";
    }

    let mut len = 0usize;
    for (idx, word) in words.iter().enumerate() {
        if idx > 0 {
            if len + 1 > out.len() {
                break;
            }
            out[len] = b' ';
            len += 1;
        }

        for ch in word.chars() {
            let mut utf8 = [0u8; 4];
            let encoded = ch.encode_utf8(&mut utf8).as_bytes();
            if len + encoded.len() > out.len() {
                break;
            }
            out[len..len + encoded.len()].copy_from_slice(encoded);
            len += encoded.len();
        }
    }

    str::from_utf8(&out[..len]).unwrap_or("")
}

pub(super) fn fit_text_in_width<'a>(
    source: &str,
    max_width: usize,
    scale: usize,
    out: &'a mut [u8; SELECTOR_TEXT_BUF],
) -> &'a str {
    if source.is_empty() || out.is_empty() || max_width == 0 {
        return "";
    }

    let mut len = 0usize;
    let mut char_count = 0usize;
    let mut truncated = false;
    let mut char_ends = [0usize; SELECTOR_TEXT_BUF];
    let mut stored_chars = 0usize;

    for ch in source.chars() {
        let mut utf8 = [0u8; 4];
        let encoded = ch.encode_utf8(&mut utf8).as_bytes();
        let next_chars = char_count + 1;
        let next_width = if next_chars == 0 {
            0
        } else {
            next_chars * (6 * scale) - scale
        };

        if next_width > max_width || len + encoded.len() > out.len() {
            truncated = true;
            break;
        }

        out[len..len + encoded.len()].copy_from_slice(encoded);
        len += encoded.len();
        char_count = next_chars;

        if stored_chars < char_ends.len() {
            char_ends[stored_chars] = len;
            stored_chars += 1;
        } else {
            truncated = true;
            break;
        }
    }

    if !truncated {
        return str::from_utf8(&out[..len]).unwrap_or("");
    }

    let ellipsis = "...";
    let ellipsis_width = text_pixel_width(ellipsis, scale);
    if ellipsis_width > max_width || len + ellipsis.len() > out.len() {
        return str::from_utf8(&out[..len]).unwrap_or("");
    }

    while stored_chars > 0 {
        let with_ellipsis = stored_chars * (6 * scale) - scale + ellipsis_width;
        if with_ellipsis <= max_width {
            break;
        }
        stored_chars -= 1;
    }

    len = if stored_chars == 0 {
        0
    } else {
        char_ends[stored_chars - 1]
    };

    if len + ellipsis.len() <= out.len() {
        out[len..len + ellipsis.len()].copy_from_slice(ellipsis.as_bytes());
        len += ellipsis.len();
    }

    str::from_utf8(&out[..len]).unwrap_or("")
}

pub(super) fn draw_timeline_strip(
    frame: &mut FrameBuffer,
    target_index: usize,
    total: usize,
    motion_dx: isize,
    track_y: usize,
    on: bool,
) {
    let track_x = 16usize;
    let track_w = WIDTH.saturating_sub(32);
    let track_h = 18usize;

    draw_rect(frame, track_x, track_y, track_w, track_h, on);

    let center_x = track_x + track_w / 2;
    for step in -4isize..=4isize {
        let tick_x = center_x as isize + step * 20 + motion_dx / 2;
        if tick_x < track_x as isize + 2 || tick_x >= (track_x + track_w - 2) as isize {
            continue;
        }

        let h = if step == 0 { 14usize } else { 8usize };
        let y = track_y + (track_h.saturating_sub(h)) / 2;
        draw_filled_rect_signed(frame, tick_x, y as isize, 1, h as isize, on);
    }

    draw_filled_rect(
        frame,
        center_x.saturating_sub(1),
        track_y.saturating_sub(4),
        3,
        3,
        on,
    );
    draw_paragraph_progress(frame, target_index, total, on);
}

pub(super) fn paragraph_focus_pct(animation: Option<AnimationFrame>) -> usize {
    let Some(animation) = animation else {
        return 100;
    };

    match animation.kind {
        AnimationKind::SlideLeft | AnimationKind::SlideRight => animation.progress_pct as usize,
        AnimationKind::Fade | AnimationKind::Pulse => 100,
    }
}

pub(super) fn selector_motion_dx(animation: Option<AnimationFrame>) -> isize {
    let Some(animation) = animation else {
        return 0;
    };

    let remaining = (100u8.saturating_sub(animation.progress_pct)) as isize;
    let slide = (remaining * 22) / 100;
    match animation.kind {
        AnimationKind::SlideLeft => slide,
        AnimationKind::SlideRight => -slide,
        AnimationKind::Fade | AnimationKind::Pulse => 0,
    }
}

pub(super) fn draw_ring(
    frame: &mut FrameBuffer,
    cx: isize,
    cy: isize,
    outer_r: isize,
    inner_r: isize,
    on: bool,
) {
    if outer_r <= 0 || inner_r >= outer_r {
        return;
    }

    draw_disk(frame, cx, cy, outer_r, on);
    draw_disk(frame, cx, cy, inner_r.max(0), !on);
}
