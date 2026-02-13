use super::*;

pub(super) fn draw_library_shelf(
    frame: &mut FrameBuffer,
    items: &[MenuItemView<'_>],
    cursor: usize,
    on: bool,
    motion: LibraryMotion,
    cover_thumbs: &[CoverThumbSlot; COVER_THUMB_SLOTS],
) {
    if items.is_empty() {
        draw_text_centered(frame, 118, "No titles", 3, on);
        return;
    }

    let cursor = cursor.min(items.len().saturating_sub(1));
    let main_x = offset_pos((WIDTH.saturating_sub(LIB_MAIN_W)) / 2, motion.list_dx);
    let main_y = offset_pos(38, motion.list_dy);
    let main_x = clamp_cover_x(offset_pos(main_x, motion.selected_nudge / 2), LIB_MAIN_W, 8);

    let side_y = main_y + (LIB_MAIN_H.saturating_sub(LIB_SIDE_H)) / 2;
    let left_x = clamp_cover_x(offset_pos(10, motion.list_dx - 8), LIB_SIDE_W, 6);
    let right_x = clamp_cover_x(
        offset_pos(
            WIDTH.saturating_sub(LIB_SIDE_W + 10),
            motion.list_dx.saturating_add(8),
        ),
        LIB_SIDE_W,
        6,
    );

    if cursor > 0 {
        draw_cover_card(
            frame,
            CoverCardSpec {
                x: left_x,
                y: side_y,
                w: LIB_SIDE_W,
                h: LIB_SIDE_H,
                item_index: cursor - 1,
                item: items[cursor - 1],
                selected: false,
                cover_thumb: cover_thumbs
                    .get(cursor - 1)
                    .filter(|thumb| thumb.loaded && thumb.width > 0 && thumb.height > 0),
            },
            on,
        );
    }

    if cursor + 1 < items.len() {
        draw_cover_card(
            frame,
            CoverCardSpec {
                x: right_x,
                y: side_y,
                w: LIB_SIDE_W,
                h: LIB_SIDE_H,
                item_index: cursor + 1,
                item: items[cursor + 1],
                selected: false,
                cover_thumb: cover_thumbs
                    .get(cursor + 1)
                    .filter(|thumb| thumb.loaded && thumb.width > 0 && thumb.height > 0),
            },
            on,
        );
    }

    draw_cover_card(
        frame,
        CoverCardSpec {
            x: main_x,
            y: main_y,
            w: LIB_MAIN_W,
            h: LIB_MAIN_H,
            item_index: cursor,
            item: items[cursor],
            selected: true,
            cover_thumb: cover_thumbs
                .get(cursor)
                .filter(|thumb| thumb.loaded && thumb.width > 0 && thumb.height > 0),
        },
        on,
    );
}

#[derive(Clone, Copy, Debug)]
pub(super) struct CoverCardSpec<'a> {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    item_index: usize,
    item: MenuItemView<'a>,
    selected: bool,
    cover_thumb: Option<&'a CoverThumbSlot>,
}

pub(super) fn draw_cover_card(frame: &mut FrameBuffer, spec: CoverCardSpec<'_>, on: bool) {
    let x = spec.x;
    let y = spec.y;
    let w = spec.w;
    let h = spec.h;
    draw_rect(frame, x, y, w, h, on);

    if spec.selected && w > 4 && h > 4 {
        draw_rect(frame, x + 2, y + 2, w - 4, h - 4, on);
    }

    if h > 8 {
        draw_filled_rect(frame, x + 5, y + 5, 2, h - 10, on);
    }

    // Decorative spine marks to suggest a physical cover.
    for i in 0..3usize {
        let mark_y = y + 16 + i * 12;
        if mark_y + 2 < y + h {
            draw_filled_rect(frame, x + 4, mark_y, 4, 2, on);
        }
    }

    let symbol = cover_symbol(spec.item_index, spec.item);
    let icon_box_w = w.saturating_sub(12);
    let icon_box_h = h.saturating_sub(10);
    let icon_limit = core::cmp::min(icon_box_w, icon_box_h);
    let icon_size = if spec.selected {
        icon_limit.saturating_mul(95) / 100
    } else {
        icon_limit.saturating_mul(90) / 100
    };
    let drew_thumb = spec.cover_thumb.is_some_and(|thumb| {
        draw_cover_thumbnail_in_box(
            frame,
            x + 7,
            y + 4,
            w.saturating_sub(12),
            h.saturating_sub(8),
            thumb,
            on,
        )
    });
    if !drew_thumb {
        draw_cover_symbol_in_box(
            frame,
            CoverSymbolSpec {
                x: x + 7,
                y: y + 4,
                w: w.saturating_sub(12),
                h: h.saturating_sub(8),
                symbol,
                icon_size,
            },
            on,
        );
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct BoxTextSpec<'a> {
    pub(super) x: usize,
    pub(super) y: usize,
    pub(super) w: usize,
    pub(super) h: usize,
    pub(super) text: &'a str,
    pub(super) scale: usize,
}

pub(super) fn draw_text_centered_in_box(frame: &mut FrameBuffer, spec: BoxTextSpec<'_>, on: bool) {
    if spec.w == 0 || spec.h == 0 || spec.text.is_empty() {
        return;
    }

    let tw = text_pixel_width(spec.text, spec.scale);
    let th = 7 * spec.scale;
    let tx = spec.x + spec.w.saturating_sub(tw) / 2;
    let ty = spec.y + spec.h.saturating_sub(th) / 2;
    draw_text(frame, tx, ty, spec.text, spec.scale, on);
}

pub(super) fn cover_symbol(item_index: usize, item: MenuItemView<'_>) -> &'static str {
    if matches!(item.kind, MenuItemKind::Settings) {
        return "⚙";
    }

    const SYMBOLS: [&str; 10] = ["📘", "📗", "📕", "📙", "📖", "🔖", "⭐", "🌙", "⚡", "📚"];
    SYMBOLS[item_index % SYMBOLS.len()]
}

#[derive(Clone, Copy, Debug)]
pub(super) struct CoverSymbolSpec<'a> {
    pub(super) x: usize,
    pub(super) y: usize,
    pub(super) w: usize,
    pub(super) h: usize,
    pub(super) symbol: &'a str,
    pub(super) icon_size: usize,
}

pub(super) fn draw_cover_symbol_in_box(
    frame: &mut FrameBuffer,
    spec: CoverSymbolSpec<'_>,
    on: bool,
) {
    let size = spec.icon_size.max(10);
    let icon_x = spec.x + spec.w.saturating_sub(size) / 2;
    let icon_y = spec.y + spec.h.saturating_sub(size) / 2;

    match spec.symbol {
        "⚙" => draw_icon_gear(frame, icon_x, icon_y, size, on),
        "📘" | "📗" | "📕" | "📙" => draw_icon_book(frame, icon_x, icon_y, size, on),
        "📖" => draw_icon_open_book(frame, icon_x, icon_y, size, on),
        "🔖" => draw_icon_bookmark(frame, icon_x, icon_y, size, on),
        "⭐" => draw_icon_star(frame, icon_x, icon_y, size, on),
        "🌙" => draw_icon_moon(frame, icon_x, icon_y, size, on),
        "⚡" => draw_icon_bolt(frame, icon_x, icon_y, size, on),
        "📚" => draw_icon_stack(frame, icon_x, icon_y, size, on),
        _ => {
            let scale = if size >= 28 { 6 } else { 4 };
            draw_text_centered_in_box(
                frame,
                BoxTextSpec {
                    x: spec.x,
                    y: spec.y,
                    w: spec.w,
                    h: spec.h,
                    text: spec.symbol,
                    scale,
                },
                on,
            );
        }
    }
}

pub(super) fn draw_cover_thumbnail_in_box(
    frame: &mut FrameBuffer,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    thumb: &CoverThumbSlot,
    on: bool,
) -> bool {
    if w == 0 || h == 0 || !thumb.loaded || thumb.width == 0 || thumb.height == 0 {
        return false;
    }

    let src_w = thumb.width as usize;
    let src_h = thumb.height as usize;
    if src_w == 0 || src_h == 0 {
        return false;
    }

    let mut draw_w = w;
    let mut draw_h = (draw_w.saturating_mul(src_h)).max(1) / src_w.max(1);
    if draw_h == 0 {
        draw_h = 1;
    }
    if draw_h > h {
        draw_h = h;
        draw_w = (draw_h.saturating_mul(src_w)).max(1) / src_h.max(1);
        if draw_w == 0 {
            draw_w = 1;
        }
    }
    draw_w = draw_w.max(1).min(w);
    draw_h = draw_h.max(1).min(h);

    let origin_x = x + (w.saturating_sub(draw_w) / 2);
    let origin_y = y + (h.saturating_sub(draw_h) / 2);
    let src_row_bytes = src_w.div_ceil(8);
    for dy in 0..draw_h {
        let sy = dy.saturating_mul(src_h) / draw_h;
        for dx in 0..draw_w {
            let sx = dx.saturating_mul(src_w) / draw_w;
            let src_idx = sy.saturating_mul(src_row_bytes).saturating_add(sx / 8);
            if src_idx >= thumb.bytes.len() {
                continue;
            }
            let src_mask = 1u8 << (7 - (sx % 8));
            if (thumb.bytes[src_idx] & src_mask) != 0 {
                let _ = frame.set_pixel(origin_x + dx, origin_y + dy, on);
            }
        }
    }

    true
}

pub(super) fn draw_icon_book(frame: &mut FrameBuffer, x: usize, y: usize, size: usize, on: bool) {
    let s = icon_scale(size);
    draw_rect(frame, x + 2 * s, y + 2 * s, 12 * s, 12 * s, on);
    draw_filled_rect(frame, x + 5 * s, y + 2 * s, s, 12 * s, on);
    draw_filled_rect(frame, x + 8 * s, y + 5 * s, 4 * s, s, on);
    draw_filled_rect(frame, x + 8 * s, y + 8 * s, 3 * s, s, on);
    draw_filled_rect(frame, x + 8 * s, y + 11 * s, 4 * s, s, on);
}

pub(super) fn draw_icon_open_book(
    frame: &mut FrameBuffer,
    x: usize,
    y: usize,
    size: usize,
    on: bool,
) {
    let s = icon_scale(size);
    draw_rect(frame, x + s, y + 3 * s, 6 * s, 10 * s, on);
    draw_rect(frame, x + 9 * s, y + 3 * s, 6 * s, 10 * s, on);
    draw_filled_rect(frame, x + 7 * s, y + 3 * s, 2 * s, 10 * s, on);
    draw_filled_rect(frame, x + 3 * s, y + 6 * s, 3 * s, s, on);
    draw_filled_rect(frame, x + 10 * s, y + 6 * s, 3 * s, s, on);
}

pub(super) fn draw_icon_bookmark(
    frame: &mut FrameBuffer,
    x: usize,
    y: usize,
    size: usize,
    on: bool,
) {
    let s = icon_scale(size);
    draw_rect(frame, x + 3 * s, y + 2 * s, 10 * s, 12 * s, on);
    draw_filled_rect(frame, x + 7 * s, y + 3 * s, 2 * s, 7 * s, on);
    draw_filled_rect(frame, x + 6 * s, y + 10 * s, 4 * s, s, on);
    draw_filled_rect(frame, x + 5 * s, y + 11 * s, 2 * s, s, on);
    draw_filled_rect(frame, x + 9 * s, y + 11 * s, 2 * s, s, on);
}

pub(super) fn draw_icon_gear(frame: &mut FrameBuffer, x: usize, y: usize, size: usize, on: bool) {
    let size_i = size as isize;
    let cx = x as isize + size_i / 2;
    let cy = y as isize + size_i / 2;

    let outer_r = core::cmp::max(7, size_i / 2 - 1);
    let tooth_depth = core::cmp::max(2, size_i / 8);
    let rim_outer = core::cmp::max(5, outer_r - tooth_depth);
    let rim_inner = core::cmp::max(3, rim_outer * 58 / 100);

    // Main toothed ring.
    draw_disk(frame, cx, cy, rim_outer, on);
    draw_disk(frame, cx, cy, rim_inner, !on);

    let tooth_w = core::cmp::max(2, size_i / 5);

    // Cardinal teeth.
    draw_filled_rect_signed(
        frame,
        cx - tooth_w / 2,
        cy - outer_r,
        tooth_w,
        tooth_depth,
        on,
    );
    draw_filled_rect_signed(
        frame,
        cx - tooth_w / 2,
        cy + outer_r - tooth_depth + 1,
        tooth_w,
        tooth_depth,
        on,
    );
    draw_filled_rect_signed(
        frame,
        cx - outer_r,
        cy - tooth_w / 2,
        tooth_depth,
        tooth_w,
        on,
    );
    draw_filled_rect_signed(
        frame,
        cx + outer_r - tooth_depth + 1,
        cy - tooth_w / 2,
        tooth_depth,
        tooth_w,
        on,
    );

    // Diagonal teeth.
    let diag = (outer_r * 707) / 1000;
    let tooth_sq = core::cmp::max(2, (tooth_w * 3) / 4);
    for (sx, sy) in [(-1isize, -1isize), (1, -1), (-1, 1), (1, 1)] {
        draw_filled_rect_signed(
            frame,
            cx + sx * diag - tooth_sq / 2,
            cy + sy * diag - tooth_sq / 2,
            tooth_sq,
            tooth_sq,
            on,
        );
    }

    // Spokes from hub into the ring.
    let spoke_t = core::cmp::max(1, size_i / 14);
    let spoke_half = spoke_t / 2;
    let spoke_len = core::cmp::max(1, rim_outer - rim_inner / 6);

    draw_filled_rect_signed(
        frame,
        cx - spoke_len,
        cy - spoke_half,
        spoke_len * 2 + 1,
        spoke_t,
        on,
    );
    draw_filled_rect_signed(
        frame,
        cx - spoke_half,
        cy - spoke_len,
        spoke_t,
        spoke_len * 2 + 1,
        on,
    );

    let diag_len = core::cmp::max(1, (spoke_len * 707) / 1000);
    draw_diag_spoke(
        frame,
        DiagSpokeSpec {
            cx,
            cy,
            len: diag_len,
            thickness: spoke_t,
            dx: 1,
            dy: 1,
        },
        on,
    );
    draw_diag_spoke(
        frame,
        DiagSpokeSpec {
            cx,
            cy,
            len: diag_len,
            thickness: spoke_t,
            dx: 1,
            dy: -1,
        },
        on,
    );
    draw_diag_spoke(
        frame,
        DiagSpokeSpec {
            cx,
            cy,
            len: diag_len,
            thickness: spoke_t,
            dx: -1,
            dy: 1,
        },
        on,
    );
    draw_diag_spoke(
        frame,
        DiagSpokeSpec {
            cx,
            cy,
            len: diag_len,
            thickness: spoke_t,
            dx: -1,
            dy: -1,
        },
        on,
    );

    // Ring notches for added detail.
    let notch_r = core::cmp::max(1, size_i / 24);
    let notch_offset = (rim_inner + rim_outer) / 2;
    for (dx, dy) in [(-1isize, 0isize), (1, 0), (0, -1), (0, 1)] {
        draw_disk(
            frame,
            cx + dx * notch_offset,
            cy + dy * notch_offset,
            notch_r,
            !on,
        );
    }

    // Central hub.
    let hub_outer = core::cmp::max(2, rim_inner * 45 / 100);
    let hub_inner = core::cmp::max(1, hub_outer * 45 / 100);
    draw_disk(frame, cx, cy, hub_outer, on);
    draw_disk(frame, cx, cy, hub_inner, !on);
    if hub_inner > 2 {
        draw_disk(frame, cx, cy, hub_inner / 2, on);
    }
}

pub(super) fn draw_icon_star(frame: &mut FrameBuffer, x: usize, y: usize, size: usize, on: bool) {
    let s = icon_scale(size);
    draw_filled_rect(frame, x + 7 * s, y + s, 2 * s, 12 * s, on);
    draw_filled_rect(frame, x + s, y + 7 * s, 14 * s, 2 * s, on);
    draw_filled_rect(frame, x + 3 * s, y + 3 * s, 2 * s, 2 * s, on);
    draw_filled_rect(frame, x + 11 * s, y + 3 * s, 2 * s, 2 * s, on);
    draw_filled_rect(frame, x + 3 * s, y + 11 * s, 2 * s, 2 * s, on);
    draw_filled_rect(frame, x + 11 * s, y + 11 * s, 2 * s, 2 * s, on);
}

pub(super) fn draw_icon_moon(frame: &mut FrameBuffer, x: usize, y: usize, size: usize, on: bool) {
    let s = icon_scale(size) as isize;
    let cx = (x + 8 * s as usize) as isize;
    let cy = (y + 8 * s as usize) as isize;
    let r = 6 * s;
    draw_disk(frame, cx, cy, r, on);
    draw_disk(frame, cx + 3 * s, cy - s, r - s, !on);
}

pub(super) fn draw_icon_bolt(frame: &mut FrameBuffer, x: usize, y: usize, size: usize, on: bool) {
    let s = icon_scale(size);
    draw_filled_rect(frame, x + 8 * s, y + s, 2 * s, 5 * s, on);
    draw_filled_rect(frame, x + 6 * s, y + 5 * s, 4 * s, 2 * s, on);
    draw_filled_rect(frame, x + 8 * s, y + 7 * s, 2 * s, 6 * s, on);
    draw_filled_rect(frame, x + 10 * s, y + 7 * s, 2 * s, 2 * s, on);
    draw_filled_rect(frame, x + 6 * s, y + 11 * s, 4 * s, 2 * s, on);
}

pub(super) fn draw_icon_stack(frame: &mut FrameBuffer, x: usize, y: usize, size: usize, on: bool) {
    let s = icon_scale(size);
    draw_rect(frame, x + 2 * s, y + 3 * s, 9 * s, 4 * s, on);
    draw_rect(frame, x + 4 * s, y + 7 * s, 9 * s, 4 * s, on);
    draw_rect(frame, x + 6 * s, y + 11 * s, 8 * s, 4 * s, on);
}

pub(super) fn icon_scale(size: usize) -> usize {
    core::cmp::max(1, size / 16)
}

#[derive(Clone, Copy, Debug)]
pub(super) struct DiagSpokeSpec {
    cx: isize,
    cy: isize,
    len: isize,
    thickness: isize,
    dx: isize,
    dy: isize,
}

pub(super) fn draw_diag_spoke(frame: &mut FrameBuffer, spec: DiagSpokeSpec, on: bool) {
    if spec.len <= 0 || spec.thickness <= 0 {
        return;
    }

    let half = spec.thickness / 2;
    for step in 0..=spec.len {
        let px = spec.cx + spec.dx * step;
        let py = spec.cy + spec.dy * step;
        draw_filled_rect_signed(
            frame,
            px - half,
            py - half,
            spec.thickness,
            spec.thickness,
            on,
        );
    }
}

pub(super) fn draw_disk(frame: &mut FrameBuffer, cx: isize, cy: isize, r: isize, on: bool) {
    if r <= 0 {
        return;
    }

    let rr = r * r;
    for dy in -r..=r {
        for dx in -r..=r {
            if dx * dx + dy * dy <= rr {
                set_pixel_signed(frame, cx + dx, cy + dy, on);
            }
        }
    }
}

pub(super) fn clamp_cover_x(x: usize, width: usize, pad: usize) -> usize {
    let min_x = pad;
    let max_x = WIDTH.saturating_sub(width + pad);
    x.clamp(min_x, max_x)
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct LibraryMotion {
    list_dx: isize,
    list_dy: isize,
    selected_nudge: isize,
}

pub(super) fn library_motion(animation: Option<AnimationFrame>) -> LibraryMotion {
    let Some(animation) = animation else {
        return LibraryMotion::default();
    };

    let remaining = (100u8.saturating_sub(animation.progress_pct)) as isize;
    let slide = (remaining * 34) / 100;
    let lift = (remaining * 10) / 100;
    let nudge = (remaining * 10) / 100;

    match animation.kind {
        AnimationKind::SlideLeft => LibraryMotion {
            list_dx: (slide * 2) / 3,
            list_dy: lift / 2,
            selected_nudge: nudge,
        },
        AnimationKind::SlideRight => LibraryMotion {
            list_dx: -(slide * 2) / 3,
            list_dy: lift / 2,
            selected_nudge: -nudge,
        },
        AnimationKind::Fade => LibraryMotion {
            list_dx: 0,
            list_dy: lift,
            selected_nudge: 0,
        },
        AnimationKind::Pulse => LibraryMotion::default(),
    }
}
