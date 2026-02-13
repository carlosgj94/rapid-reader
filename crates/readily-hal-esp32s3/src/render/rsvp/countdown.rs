use super::*;

#[allow(
    clippy::too_many_arguments,
    reason = "countdown rendering keeps params explicit to avoid heap-backed structs in hot path"
)]
pub(super) fn draw_countdown_stage(
    frame: &mut FrameBuffer,
    title: &str,
    cover_slot: u16,
    has_cover: bool,
    cover_thumb: Option<&CoverThumbSlot>,
    remaining: u8,
    animation: Option<AnimationFrame>,
    on: bool,
) {
    let pulse = countdown_pulse_px(animation);
    draw_text_centered(frame, 42, "GET READY", 2, on);

    let wide_layout = WIDTH >= 320;
    if wide_layout {
        draw_countdown_cover_card(frame, title, cover_slot, has_cover, cover_thumb, on);
    }

    let cx = if wide_layout {
        WIDTH.saturating_sub(102) as isize
    } else {
        (WIDTH / 2) as isize
    };
    let cy = 108isize;
    let outer = if wide_layout { 38isize } else { 42isize } + pulse as isize;
    let inner = (outer - 7).max(10);
    draw_ring(frame, cx, cy, outer, inner, on);

    let countdown_text = match remaining {
        0 => "0",
        1 => "1",
        2 => "2",
        3 => "3",
        _ => "3",
    };
    let count_scale = 8usize;
    let count_width = text_pixel_width(countdown_text, count_scale);
    let center_x = cx.max(0) as usize;
    let count_x = center_x.saturating_sub(count_width / 2);
    draw_text(frame, count_x, 84, countdown_text, count_scale, on);

    draw_countdown_ticks(frame, remaining, center_x, on);
}

pub(super) fn draw_countdown_cover_card(
    frame: &mut FrameBuffer,
    title: &str,
    cover_slot: u16,
    has_cover: bool,
    cover_thumb: Option<&CoverThumbSlot>,
    on: bool,
) {
    let card_w = 116usize;
    let card_h = 150usize;
    let x = 18usize;
    let y = 56usize;

    draw_rect(frame, x, y, card_w, card_h, on);
    if card_w > 4 && card_h > 4 {
        draw_rect(frame, x + 2, y + 2, card_w - 4, card_h - 4, on);
    }

    if card_h > 10 {
        draw_filled_rect(frame, x + 5, y + 6, 2, card_h - 12, on);
    }
    for i in 0..4usize {
        let mark_y = y + 18 + i * 16;
        if mark_y + 2 < y + card_h {
            draw_filled_rect(frame, x + 4, mark_y, 4, 2, on);
        }
    }

    let icon_x = x + 10;
    let icon_y = y + 10;
    let icon_w = card_w.saturating_sub(20);
    let icon_h = card_h.saturating_sub(42);
    let icon_size = core::cmp::min(icon_w, icon_h).saturating_mul(90) / 100;
    let drew_thumb = cover_thumb.is_some_and(|thumb| {
        draw_cover_thumbnail_in_box(frame, icon_x, icon_y, icon_w, icon_h, thumb, on)
    });
    if !drew_thumb {
        let item = MenuItemView {
            label: title,
            kind: MenuItemKind::Text,
        };
        let symbol = if has_cover {
            cover_symbol(cover_slot as usize, item)
        } else {
            "📘"
        };
        draw_cover_symbol_in_box(
            frame,
            CoverSymbolSpec {
                x: icon_x,
                y: icon_y,
                w: icon_w,
                h: icon_h,
                symbol,
                icon_size,
            },
            on,
        );
    }

    let mut title_buf = [0u8; SELECTOR_TEXT_BUF];
    let title_fit = fit_text_in_width(title, card_w.saturating_sub(16), 1, &mut title_buf);
    draw_text_centered_in_box(
        frame,
        BoxTextSpec {
            x: x + 8,
            y: y + card_h.saturating_sub(24),
            w: card_w.saturating_sub(16),
            h: 14,
            text: title_fit,
            scale: 1,
        },
        on,
    );
}

pub(super) fn draw_countdown_ticks(
    frame: &mut FrameBuffer,
    remaining: u8,
    center_x: usize,
    on: bool,
) {
    let total = 3usize;
    let lit = remaining.min(total as u8) as usize;
    let dot = 8usize;
    let gap = 10usize;
    let block_w = total * dot + (total - 1) * gap;
    let start_x = center_x.saturating_sub(block_w / 2);
    let y = HEIGHT.saturating_sub(56);

    for i in 0..total {
        let x = start_x + i * (dot + gap);
        if i < lit {
            draw_filled_rect(frame, x, y, dot, dot, on);
        } else {
            draw_rect(frame, x, y, dot, dot, on);
        }
    }
}

pub(super) fn countdown_pulse_px(animation: Option<AnimationFrame>) -> usize {
    let Some(animation) = animation else {
        return 0;
    };

    if !matches!(animation.kind, AnimationKind::Pulse) {
        return 0;
    }

    let p = animation.progress_pct as usize;
    let tri = if p <= 50 { p } else { 100 - p };
    (tri * 5) / 50
}

pub(super) fn draw_pause_overlay(
    frame: &mut FrameBuffer,
    title: &str,
    chapter_label: &str,
    elapsed_ms: u32,
    on: bool,
) {
    let phase = ((elapsed_ms / 180) % 8) as usize;
    let breathe = if phase <= 4 { phase } else { 8 - phase };

    let x = 16usize.saturating_sub(breathe / 2);
    let y = 48usize.saturating_sub(breathe / 3);
    let w = WIDTH.saturating_sub(32).saturating_add(breathe);
    let h = 100usize.saturating_add(breathe / 2);

    // Make the overlay opaque to keep paused controls legible over the RSVP word.
    if w > 2 && h > 2 {
        draw_filled_rect(frame, x + 1, y + 1, w - 2, h - 2, !on);
    }

    draw_rect(frame, x, y, w, h, on);
    if w > 4 && h > 4 {
        draw_rect(frame, x + 2, y + 2, w - 4, h - 4, on);
    }

    if w > 2 {
        draw_filled_rect(frame, x + 1, y + 1, w - 2, 14, on);
    }

    draw_text_centered_in_box(
        frame,
        BoxTextSpec {
            x: x + 4,
            y: y + 2,
            w: w.saturating_sub(8),
            h: 10,
            text: "PAUSED",
            scale: 2,
        },
        !on,
    );

    let text_w = w.saturating_sub(20);
    let mut title_buf = [0u8; SELECTOR_TEXT_BUF];
    let mut chapter_buf = [0u8; SELECTOR_TEXT_BUF];
    let title_fit = fit_text_in_width(title, text_w, 1, &mut title_buf);
    let chapter_fit = fit_text_in_width(chapter_label, text_w, 1, &mut chapter_buf);

    draw_text_centered_in_box(
        frame,
        BoxTextSpec {
            x: x + 10,
            y: y + 24,
            w: text_w,
            h: 12,
            text: title_fit,
            scale: 1,
        },
        on,
    );
    draw_text_centered_in_box(
        frame,
        BoxTextSpec {
            x: x + 10,
            y: y + 38,
            w: text_w,
            h: 12,
            text: chapter_fit,
            scale: 1,
        },
        on,
    );

    draw_filled_rect(frame, x + 10, y + 54, w.saturating_sub(20), 1, on);
    draw_text_centered_in_box(
        frame,
        BoxTextSpec {
            x: x + 10,
            y: y + 62,
            w: text_w,
            h: 10,
            text: "Press resume",
            scale: 1,
        },
        on,
    );
    draw_text_centered_in_box(
        frame,
        BoxTextSpec {
            x: x + 10,
            y: y + 74,
            w: text_w,
            h: 10,
            text: "Rotate for timeline",
            scale: 1,
        },
        on,
    );
    draw_text_centered_in_box(
        frame,
        BoxTextSpec {
            x: x + 10,
            y: y + 86,
            w: text_w,
            h: 10,
            text: "Double press home",
            scale: 1,
        },
        on,
    );
}
