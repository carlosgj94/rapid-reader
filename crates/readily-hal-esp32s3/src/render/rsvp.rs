use core::str;

use ls027b7dh01::{
    FrameBuffer,
    protocol::{HEIGHT, WIDTH},
};
use readily_core::render::{
    AnimationFrame, AnimationKind, FontFamily, FontSize, MenuItemKind, MenuItemView, Screen,
    SettingRowView, SettingValue, VisualStyle,
};

use crate::network::ConnectivitySnapshot;

use super::{FrameRenderer, book_font};

const SETTINGS_ROWS_VISIBLE: usize = 5;
const MENU_ROW_HEIGHT: usize = 28;
const MENU_LIST_TOP: usize = 42;
const MENU_MARKER_X: usize = 14;
const MENU_TEXT_X: usize = 30;
const LIB_MAIN_W: usize = 192;
const LIB_MAIN_H: usize = 192;
const LIB_SIDE_W: usize = 74;
const LIB_SIDE_H: usize = 104;
const SELECTOR_TEXT_BUF: usize = 96;
const COVER_THUMB_SLOTS: usize = 16;
const COVER_THUMB_MAX_W: usize = 56;
const COVER_THUMB_MAX_H: usize = 76;
const COVER_THUMB_MAX_BYTES: usize = COVER_THUMB_MAX_W.div_ceil(8) * COVER_THUMB_MAX_H;

#[derive(Clone, Copy, Debug)]
struct CoverThumbSlot {
    loaded: bool,
    width: u16,
    height: u16,
    bytes: [u8; COVER_THUMB_MAX_BYTES],
}

impl CoverThumbSlot {
    const EMPTY: Self = Self {
        loaded: false,
        width: 0,
        height: 0,
        bytes: [0u8; COVER_THUMB_MAX_BYTES],
    };
}

/// Renderer for countdown, RSVP reading, and status screens.
#[derive(Debug, Clone)]
pub struct RsvpRenderer {
    orp_anchor_percent: usize,
    connectivity: ConnectivitySnapshot,
    cover_thumbs: [CoverThumbSlot; COVER_THUMB_SLOTS],
}

impl Default for RsvpRenderer {
    fn default() -> Self {
        Self {
            orp_anchor_percent: 45,
            connectivity: ConnectivitySnapshot::disconnected(),
            cover_thumbs: [CoverThumbSlot::EMPTY; COVER_THUMB_SLOTS],
        }
    }
}

impl RsvpRenderer {
    pub const fn new(orp_anchor_percent: usize) -> Self {
        Self {
            orp_anchor_percent,
            connectivity: ConnectivitySnapshot::disconnected(),
            cover_thumbs: [CoverThumbSlot::EMPTY; COVER_THUMB_SLOTS],
        }
    }

    pub fn set_connectivity(&mut self, connectivity: ConnectivitySnapshot) {
        self.connectivity = connectivity;
    }

    pub const fn cover_thumb_target_size() -> (u16, u16) {
        (COVER_THUMB_MAX_W as u16, COVER_THUMB_MAX_H as u16)
    }

    pub fn set_cover_thumbnail(
        &mut self,
        slot: u16,
        width: u16,
        height: u16,
        bytes: &[u8],
    ) -> bool {
        let idx = slot as usize;
        if idx >= self.cover_thumbs.len() {
            return false;
        }
        if width == 0
            || height == 0
            || width as usize > COVER_THUMB_MAX_W
            || height as usize > COVER_THUMB_MAX_H
        {
            return false;
        }

        let row_bytes = (width as usize).div_ceil(8);
        let needed = row_bytes.saturating_mul(height as usize);
        if needed == 0 || needed > self.cover_thumbs[idx].bytes.len() || bytes.len() < needed {
            return false;
        }

        let slot_ref = &mut self.cover_thumbs[idx];
        slot_ref.bytes.fill(0);
        slot_ref.bytes[..needed].copy_from_slice(&bytes[..needed]);
        slot_ref.width = width;
        slot_ref.height = height;
        slot_ref.loaded = true;
        true
    }

    fn cover_thumb(&self, slot: usize) -> Option<&CoverThumbSlot> {
        self.cover_thumbs
            .get(slot)
            .filter(|thumb| thumb.loaded && thumb.width > 0 && thumb.height > 0)
    }
}

impl FrameRenderer for RsvpRenderer {
    fn render(&mut self, screen: Screen<'_>, frame: &mut FrameBuffer) {
        match screen {
            Screen::Library {
                title: _,
                subtitle: _,
                items,
                cursor,
                style,
                animation,
            } => {
                let (bg_on, fg_on) = palette(style);
                clear_frame(frame, bg_on);

                let motion = library_motion(animation);
                render_library_header(frame, items, cursor, self.connectivity, fg_on);

                draw_library_shelf(frame, items, cursor, fg_on, motion, &self.cover_thumbs);
            }
            Screen::Settings {
                title,
                subtitle,
                rows,
                cursor,
                editing,
                style,
                animation: _,
            } => {
                let (bg_on, fg_on) = palette(style);
                clear_frame(frame, bg_on);
                draw_rect(frame, 0, 0, WIDTH, HEIGHT, fg_on);
                render_header_text(frame, title, subtitle, self.connectivity, fg_on);
                draw_settings_rows(frame, rows, cursor, editing, fg_on);
                let hint = if editing {
                    "Rotate to edit, press done"
                } else {
                    "Rotate to navigate, press edit"
                };
                draw_footer_hint(frame, hint, fg_on);
            }
            Screen::Countdown {
                title,
                cover_slot,
                has_cover,
                wpm,
                remaining,
                style,
                animation,
            } => {
                let (bg_on, fg_on) = palette(style);
                clear_frame(frame, bg_on);
                draw_rect(frame, 0, 0, WIDTH, HEIGHT, fg_on);
                render_header_wpm(frame, title, wpm, self.connectivity, fg_on);
                draw_countdown_stage(
                    frame,
                    title,
                    cover_slot,
                    has_cover,
                    self.cover_thumb(cover_slot as usize),
                    remaining,
                    animation,
                    fg_on,
                );
            }
            Screen::Reading {
                title,
                wpm,
                word,
                paragraph_word_index,
                paragraph_word_total,
                paused,
                paused_elapsed_ms,
                pause_chapter_label,
                style,
                animation: _,
            } => {
                let (bg_on, fg_on) = palette(style);
                clear_frame(frame, bg_on);
                draw_rect(frame, 0, 0, WIDTH, HEIGHT, fg_on);
                render_header_wpm_custom(frame, title, wpm, 2, 1, self.connectivity, fg_on);

                let (use_serif, word_scale, stride) = match style.font_family {
                    FontFamily::Serif => {
                        let render = choose_word_scale_book(word, WIDTH - 20, style.font_size);
                        (true, render.scale, render.stride)
                    }
                    FontFamily::Pixel => {
                        let scale = choose_word_scale(word, WIDTH - 20, style.font_size);
                        (false, scale, 1)
                    }
                };

                draw_rsvp_word(
                    frame,
                    RsvpWordSpec {
                        y: 104,
                        word,
                        scale: word_scale,
                        orp_anchor_percent: self.orp_anchor_percent,
                        serif_word: use_serif,
                        stride,
                    },
                    fg_on,
                );
                draw_paragraph_progress(
                    frame,
                    paragraph_word_index as usize,
                    paragraph_word_total as usize,
                    fg_on,
                );

                if paused {
                    draw_pause_overlay(frame, title, pause_chapter_label, paused_elapsed_ms, fg_on);
                }
            }
            Screen::NavigateChapters {
                title,
                wpm,
                current_chapter,
                target_chapter,
                chapter_total,
                current_label,
                target_label,
                current_secondary,
                target_secondary,
                style,
                animation,
            } => {
                let (bg_on, fg_on) = palette(style);
                clear_frame(frame, bg_on);
                draw_rect(frame, 0, 0, WIDTH, HEIGHT, fg_on);
                render_header_wpm(frame, title, wpm, self.connectivity, fg_on);
                draw_navigation_selector(
                    frame,
                    NavigationSelectorSpec {
                        mode_label: "CHAPTER SELECT",
                        context_label: "",
                        current_primary: current_label,
                        current_secondary,
                        target_primary: target_label,
                        target_secondary,
                        target_index: target_chapter as usize,
                        total: chapter_total as usize,
                        animation,
                    },
                    fg_on,
                );
                draw_paragraph_progress_current_marker(
                    frame,
                    current_chapter as usize,
                    chapter_total as usize,
                    fg_on,
                );
            }
            Screen::NavigateParagraphs {
                title,
                wpm,
                chapter_label,
                current_preview,
                target_preview,
                current_secondary,
                target_secondary,
                target_index_in_chapter,
                paragraph_total_in_chapter,
                style,
                animation,
            } => {
                let (bg_on, fg_on) = palette(style);
                clear_frame(frame, bg_on);
                draw_rect(frame, 0, 0, WIDTH, HEIGHT, fg_on);
                render_header_wpm(frame, title, wpm, self.connectivity, fg_on);
                draw_navigation_selector(
                    frame,
                    NavigationSelectorSpec {
                        mode_label: "PARAGRAPH SELECT",
                        context_label: chapter_label,
                        current_primary: current_preview,
                        current_secondary,
                        target_primary: target_preview,
                        target_secondary,
                        target_index: target_index_in_chapter as usize,
                        total: paragraph_total_in_chapter as usize,
                        animation,
                    },
                    fg_on,
                );
            }
            Screen::Status {
                title,
                wpm,
                line1,
                line2,
                style,
                animation: _,
            } => {
                let (bg_on, fg_on) = palette(style);
                clear_frame(frame, bg_on);
                draw_rect(frame, 0, 0, WIDTH, HEIGHT, fg_on);
                render_header_wpm(frame, title, wpm, self.connectivity, fg_on);
                draw_text(frame, 12, 84, line1, 2, fg_on);
                draw_text(frame, 12, 120, line2, 2, fg_on);
                draw_paragraph_progress(frame, 0, 1, fg_on);
            }
        }
    }
}

fn clear_frame(frame: &mut FrameBuffer, on: bool) {
    frame.clear(on);
}

fn palette(style: VisualStyle) -> (bool, bool) {
    if style.inverted {
        (true, false)
    } else {
        (false, true)
    }
}

const WIFI_ICON_SIZE: usize = 14;
const WIFI_ICON_RIGHT_PAD: usize = 12;
const WIFI_ICON_TEXT_GAP: usize = 8;

fn wifi_icon_xy() -> (usize, usize) {
    (
        WIDTH.saturating_sub(WIFI_ICON_RIGHT_PAD + WIFI_ICON_SIZE),
        11,
    )
}

fn header_right_text_x(right_width: usize) -> usize {
    let (icon_x, _) = wifi_icon_xy();
    let anchor = icon_x.saturating_sub(WIFI_ICON_TEXT_GAP);
    anchor.saturating_sub(right_width)
}

fn draw_header_wifi_icon(frame: &mut FrameBuffer, connectivity: ConnectivitySnapshot, on: bool) {
    let (x, y) = wifi_icon_xy();
    draw_wifi_icon(
        frame,
        x,
        y,
        WIFI_ICON_SIZE,
        connectivity.icon_connected(),
        on,
    );
}

fn render_header_wpm(
    frame: &mut FrameBuffer,
    title: &str,
    wpm: u16,
    connectivity: ConnectivitySnapshot,
    on: bool,
) {
    render_header_wpm_custom(frame, title, wpm, 2, 2, connectivity, on);
}

fn render_header_wpm_custom(
    frame: &mut FrameBuffer,
    title: &str,
    wpm: u16,
    title_scale: usize,
    wpm_scale: usize,
    connectivity: ConnectivitySnapshot,
    on: bool,
) {
    draw_text(frame, 12, 12, title, title_scale, on);

    let mut wpm_buf = [0u8; 16];
    let wpm_label = wpm_label(wpm, &mut wpm_buf);
    let right_width = text_pixel_width(wpm_label, wpm_scale);
    let right_x = header_right_text_x(right_width);
    let wpm_y = if wpm_scale < title_scale { 14 } else { 12 };
    draw_text(frame, right_x, wpm_y, wpm_label, wpm_scale, on);
    draw_header_wifi_icon(frame, connectivity, on);
}

fn render_header_text(
    frame: &mut FrameBuffer,
    title: &str,
    right: &str,
    connectivity: ConnectivitySnapshot,
    on: bool,
) {
    render_header_text_scaled(frame, title, right, 2, connectivity, on);
}

fn render_header_text_scaled(
    frame: &mut FrameBuffer,
    title: &str,
    right: &str,
    scale: usize,
    connectivity: ConnectivitySnapshot,
    on: bool,
) {
    draw_text(frame, 12, 12, title, scale, on);
    let right_width = text_pixel_width(right, scale);
    let right_x = header_right_text_x(right_width);
    draw_text(frame, right_x, 12, right, scale, on);
    draw_header_wifi_icon(frame, connectivity, on);
}

fn render_library_header(
    frame: &mut FrameBuffer,
    items: &[MenuItemView<'_>],
    cursor: usize,
    connectivity: ConnectivitySnapshot,
    on: bool,
) {
    let selected = items
        .get(cursor.min(items.len().saturating_sub(1)))
        .map(|item| item.label)
        .unwrap_or("Library");
    let mut title_buf = [0u8; SELECTOR_TEXT_BUF];
    let title_max_w = header_right_text_x(0).saturating_sub(12);
    let title_fit = fit_text_in_width(selected, title_max_w, 2, &mut title_buf);

    draw_text(frame, 12, 12, title_fit, 2, on);
    draw_filled_rect(frame, 12, 34, WIDTH.saturating_sub(24), 1, on);
    draw_header_wifi_icon(frame, connectivity, on);
}

fn draw_wifi_icon(
    frame: &mut FrameBuffer,
    x: usize,
    y: usize,
    size: usize,
    connected: bool,
    on: bool,
) {
    let s = icon_scale(size);

    // Dot
    draw_filled_rect(frame, x + 7 * s, y + 12 * s, 2 * s, 2 * s, on);

    // Inner arc
    draw_filled_rect(frame, x + 5 * s, y + 10 * s, 6 * s, s, on);
    draw_filled_rect(frame, x + 4 * s, y + 9 * s, s, s, on);
    draw_filled_rect(frame, x + 11 * s, y + 9 * s, s, s, on);

    // Middle arc
    draw_filled_rect(frame, x + 3 * s, y + 7 * s, 10 * s, s, on);
    draw_filled_rect(frame, x + 2 * s, y + 6 * s, s, s, on);
    draw_filled_rect(frame, x + 13 * s, y + 6 * s, s, s, on);

    // Outer arc
    draw_filled_rect(frame, x + s, y + 4 * s, 14 * s, s, on);
    draw_filled_rect(frame, x, y + 3 * s, s, s, on);
    draw_filled_rect(frame, x + 15 * s, y + 3 * s, s, s, on);

    if !connected {
        // Connection cut slash.
        for i in 0..=(13 * s) {
            let px = x + i;
            let py = y + i;
            set_pixel(frame, px, py, on);
            set_pixel(frame, px, py.saturating_add(1), on);
        }
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "countdown rendering keeps params explicit to avoid heap-backed structs in hot path"
)]
fn draw_countdown_stage(
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

fn draw_countdown_cover_card(
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

fn draw_countdown_ticks(frame: &mut FrameBuffer, remaining: u8, center_x: usize, on: bool) {
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

fn countdown_pulse_px(animation: Option<AnimationFrame>) -> usize {
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

fn draw_pause_overlay(
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

#[derive(Clone, Copy, Debug)]
struct NavigationSelectorSpec<'a> {
    mode_label: &'a str,
    context_label: &'a str,
    current_primary: &'a str,
    current_secondary: &'a str,
    target_primary: &'a str,
    target_secondary: &'a str,
    target_index: usize,
    total: usize,
    animation: Option<AnimationFrame>,
}

fn draw_navigation_selector(frame: &mut FrameBuffer, spec: NavigationSelectorSpec<'_>, on: bool) {
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
struct SelectorCardSpec<'a> {
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

fn draw_selector_card(frame: &mut FrameBuffer, spec: SelectorCardSpec<'_>, on: bool) {
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

fn wrap_preview_lines(
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

fn append_ellipsis_in_place(
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

fn join_words_into_buf<'a>(words: &[&str], out: &'a mut [u8; SELECTOR_TEXT_BUF]) -> &'a str {
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

fn fit_text_in_width<'a>(
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

fn draw_timeline_strip(
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

fn paragraph_focus_pct(animation: Option<AnimationFrame>) -> usize {
    let Some(animation) = animation else {
        return 100;
    };

    match animation.kind {
        AnimationKind::SlideLeft | AnimationKind::SlideRight => animation.progress_pct as usize,
        AnimationKind::Fade | AnimationKind::Pulse => 100,
    }
}

fn selector_motion_dx(animation: Option<AnimationFrame>) -> isize {
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

fn draw_ring(
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

fn draw_library_shelf(
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
struct CoverCardSpec<'a> {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    item_index: usize,
    item: MenuItemView<'a>,
    selected: bool,
    cover_thumb: Option<&'a CoverThumbSlot>,
}

fn draw_cover_card(frame: &mut FrameBuffer, spec: CoverCardSpec<'_>, on: bool) {
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
struct BoxTextSpec<'a> {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    text: &'a str,
    scale: usize,
}

fn draw_text_centered_in_box(frame: &mut FrameBuffer, spec: BoxTextSpec<'_>, on: bool) {
    if spec.w == 0 || spec.h == 0 || spec.text.is_empty() {
        return;
    }

    let tw = text_pixel_width(spec.text, spec.scale);
    let th = 7 * spec.scale;
    let tx = spec.x + spec.w.saturating_sub(tw) / 2;
    let ty = spec.y + spec.h.saturating_sub(th) / 2;
    draw_text(frame, tx, ty, spec.text, spec.scale, on);
}

fn cover_symbol(item_index: usize, item: MenuItemView<'_>) -> &'static str {
    if matches!(item.kind, MenuItemKind::Settings) {
        return "⚙";
    }

    const SYMBOLS: [&str; 10] = ["📘", "📗", "📕", "📙", "📖", "🔖", "⭐", "🌙", "⚡", "📚"];
    SYMBOLS[item_index % SYMBOLS.len()]
}

#[derive(Clone, Copy, Debug)]
struct CoverSymbolSpec<'a> {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    symbol: &'a str,
    icon_size: usize,
}

fn draw_cover_symbol_in_box(frame: &mut FrameBuffer, spec: CoverSymbolSpec<'_>, on: bool) {
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

fn draw_cover_thumbnail_in_box(
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

fn draw_icon_book(frame: &mut FrameBuffer, x: usize, y: usize, size: usize, on: bool) {
    let s = icon_scale(size);
    draw_rect(frame, x + 2 * s, y + 2 * s, 12 * s, 12 * s, on);
    draw_filled_rect(frame, x + 5 * s, y + 2 * s, s, 12 * s, on);
    draw_filled_rect(frame, x + 8 * s, y + 5 * s, 4 * s, s, on);
    draw_filled_rect(frame, x + 8 * s, y + 8 * s, 3 * s, s, on);
    draw_filled_rect(frame, x + 8 * s, y + 11 * s, 4 * s, s, on);
}

fn draw_icon_open_book(frame: &mut FrameBuffer, x: usize, y: usize, size: usize, on: bool) {
    let s = icon_scale(size);
    draw_rect(frame, x + s, y + 3 * s, 6 * s, 10 * s, on);
    draw_rect(frame, x + 9 * s, y + 3 * s, 6 * s, 10 * s, on);
    draw_filled_rect(frame, x + 7 * s, y + 3 * s, 2 * s, 10 * s, on);
    draw_filled_rect(frame, x + 3 * s, y + 6 * s, 3 * s, s, on);
    draw_filled_rect(frame, x + 10 * s, y + 6 * s, 3 * s, s, on);
}

fn draw_icon_bookmark(frame: &mut FrameBuffer, x: usize, y: usize, size: usize, on: bool) {
    let s = icon_scale(size);
    draw_rect(frame, x + 3 * s, y + 2 * s, 10 * s, 12 * s, on);
    draw_filled_rect(frame, x + 7 * s, y + 3 * s, 2 * s, 7 * s, on);
    draw_filled_rect(frame, x + 6 * s, y + 10 * s, 4 * s, s, on);
    draw_filled_rect(frame, x + 5 * s, y + 11 * s, 2 * s, s, on);
    draw_filled_rect(frame, x + 9 * s, y + 11 * s, 2 * s, s, on);
}

fn draw_icon_gear(frame: &mut FrameBuffer, x: usize, y: usize, size: usize, on: bool) {
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

fn draw_icon_star(frame: &mut FrameBuffer, x: usize, y: usize, size: usize, on: bool) {
    let s = icon_scale(size);
    draw_filled_rect(frame, x + 7 * s, y + s, 2 * s, 12 * s, on);
    draw_filled_rect(frame, x + s, y + 7 * s, 14 * s, 2 * s, on);
    draw_filled_rect(frame, x + 3 * s, y + 3 * s, 2 * s, 2 * s, on);
    draw_filled_rect(frame, x + 11 * s, y + 3 * s, 2 * s, 2 * s, on);
    draw_filled_rect(frame, x + 3 * s, y + 11 * s, 2 * s, 2 * s, on);
    draw_filled_rect(frame, x + 11 * s, y + 11 * s, 2 * s, 2 * s, on);
}

fn draw_icon_moon(frame: &mut FrameBuffer, x: usize, y: usize, size: usize, on: bool) {
    let s = icon_scale(size) as isize;
    let cx = (x + 8 * s as usize) as isize;
    let cy = (y + 8 * s as usize) as isize;
    let r = 6 * s;
    draw_disk(frame, cx, cy, r, on);
    draw_disk(frame, cx + 3 * s, cy - s, r - s, !on);
}

fn draw_icon_bolt(frame: &mut FrameBuffer, x: usize, y: usize, size: usize, on: bool) {
    let s = icon_scale(size);
    draw_filled_rect(frame, x + 8 * s, y + s, 2 * s, 5 * s, on);
    draw_filled_rect(frame, x + 6 * s, y + 5 * s, 4 * s, 2 * s, on);
    draw_filled_rect(frame, x + 8 * s, y + 7 * s, 2 * s, 6 * s, on);
    draw_filled_rect(frame, x + 10 * s, y + 7 * s, 2 * s, 2 * s, on);
    draw_filled_rect(frame, x + 6 * s, y + 11 * s, 4 * s, 2 * s, on);
}

fn draw_icon_stack(frame: &mut FrameBuffer, x: usize, y: usize, size: usize, on: bool) {
    let s = icon_scale(size);
    draw_rect(frame, x + 2 * s, y + 3 * s, 9 * s, 4 * s, on);
    draw_rect(frame, x + 4 * s, y + 7 * s, 9 * s, 4 * s, on);
    draw_rect(frame, x + 6 * s, y + 11 * s, 8 * s, 4 * s, on);
}

fn icon_scale(size: usize) -> usize {
    core::cmp::max(1, size / 16)
}

#[derive(Clone, Copy, Debug)]
struct DiagSpokeSpec {
    cx: isize,
    cy: isize,
    len: isize,
    thickness: isize,
    dx: isize,
    dy: isize,
}

fn draw_diag_spoke(frame: &mut FrameBuffer, spec: DiagSpokeSpec, on: bool) {
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

fn draw_disk(frame: &mut FrameBuffer, cx: isize, cy: isize, r: isize, on: bool) {
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

fn clamp_cover_x(x: usize, width: usize, pad: usize) -> usize {
    let min_x = pad;
    let max_x = WIDTH.saturating_sub(width + pad);
    x.clamp(min_x, max_x)
}

fn draw_settings_rows(
    frame: &mut FrameBuffer,
    rows: &[SettingRowView<'_>],
    cursor: usize,
    editing: bool,
    on: bool,
) {
    if rows.is_empty() {
        return;
    }

    let start = cursor.saturating_sub(SETTINGS_ROWS_VISIBLE.saturating_sub(1));
    let end = core::cmp::min(rows.len(), start + SETTINGS_ROWS_VISIBLE);

    for (row_idx, data_idx) in (start..end).enumerate() {
        let y = MENU_LIST_TOP + row_idx * MENU_ROW_HEIGHT;
        if data_idx == cursor {
            draw_text(frame, MENU_MARKER_X, y + 4, ">", 2, on);
        }

        draw_text(frame, MENU_TEXT_X, y + 4, rows[data_idx].key, 2, on);
        let highlight = editing && data_idx == cursor;
        match rows[data_idx].value {
            SettingValue::Label(v) => {
                let value = if highlight {
                    highlighted_value(v)
                } else {
                    ValueLabel::Plain(v)
                };
                draw_right_label(frame, y + 4, value, on);
            }
            SettingValue::Toggle(v) => {
                let base = if v { "Black/White" } else { "White/Black" };
                let value = if highlight {
                    highlighted_value(base)
                } else {
                    ValueLabel::Plain(base)
                };
                draw_right_label(frame, y + 4, value, on);
            }
            SettingValue::Number(v) => {
                let mut buf = [0u8; 16];
                let wpm = wpm_label(v, &mut buf);
                let value = if highlight {
                    highlighted_value(wpm)
                } else {
                    ValueLabel::Plain(wpm)
                };
                draw_right_label(frame, y + 4, value, on);
            }
            SettingValue::Action(v) => {
                let value = if highlight {
                    highlighted_value(v)
                } else {
                    ValueLabel::Plain(v)
                };
                draw_right_label(frame, y + 4, value, on);
            }
        }
    }
}

#[derive(Clone, Copy)]
enum ValueLabel<'a> {
    Plain(&'a str),
    Wrapped {
        prefix: &'a str,
        core: &'a str,
        suffix: &'a str,
    },
}

fn highlighted_value<'a>(core: &'a str) -> ValueLabel<'a> {
    ValueLabel::Wrapped {
        prefix: "[",
        core,
        suffix: "]",
    }
}

fn draw_right_label(frame: &mut FrameBuffer, y: usize, label: ValueLabel<'_>, on: bool) {
    match label {
        ValueLabel::Plain(text) => {
            let right_width = text_pixel_width(text, 2);
            let right_x = WIDTH.saturating_sub(18 + right_width);
            draw_text(frame, right_x, y, text, 2, on);
        }
        ValueLabel::Wrapped {
            prefix,
            core,
            suffix,
        } => {
            let char_count = prefix.chars().count() + core.chars().count() + suffix.chars().count();
            let width = if char_count == 0 {
                0
            } else {
                char_count * 12 - 2
            };
            let mut x = WIDTH.saturating_sub(18 + width);
            draw_text(frame, x, y, prefix, 2, on);
            x += prefix.chars().count() * 12;
            draw_text(frame, x, y, core, 2, on);
            x += core.chars().count() * 12;
            draw_text(frame, x, y, suffix, 2, on);
        }
    }
}

fn draw_footer_hint(frame: &mut FrameBuffer, text: &str, on: bool) {
    draw_text(frame, 12, HEIGHT - 30, text, 1, on);
}

#[derive(Clone, Copy, Debug, Default)]
struct LibraryMotion {
    list_dx: isize,
    list_dy: isize,
    selected_nudge: isize,
}

fn library_motion(animation: Option<AnimationFrame>) -> LibraryMotion {
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

fn wpm_label(wpm: u16, out: &mut [u8; 16]) -> &str {
    let mut len = write_u16_ascii(wpm, out);
    let suffix = b" wpm";

    if len + suffix.len() > out.len() {
        return "wpm";
    }

    out[len..len + suffix.len()].copy_from_slice(suffix);
    len += suffix.len();

    str::from_utf8(&out[..len]).unwrap_or("wpm")
}

fn write_u16_ascii(mut value: u16, out: &mut [u8]) -> usize {
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

fn draw_paragraph_progress(
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

fn draw_paragraph_progress_current_marker(
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

fn offset_pos(base: usize, delta: isize) -> usize {
    if delta >= 0 {
        base.saturating_add(delta as usize)
    } else {
        base.saturating_sub((-delta) as usize)
    }
}

fn choose_word_scale(word: &str, max_width: usize, size: FontSize) -> usize {
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

#[derive(Clone, Copy, Debug)]
struct BookWordRender {
    scale: usize,
    stride: usize,
}

fn rsvp_word_pixel_width(word: &str, scale: usize) -> usize {
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

fn draw_text_centered(frame: &mut FrameBuffer, y: usize, text: &str, scale: usize, on: bool) {
    let width = text_pixel_width(text, scale);
    let x = WIDTH.saturating_sub(width) / 2;
    draw_text(frame, x, y, text, scale, on);
}

fn text_pixel_width(text: &str, scale: usize) -> usize {
    let chars = text.chars().count();
    if chars == 0 {
        0
    } else {
        chars * (6 * scale) - scale
    }
}

fn draw_text(frame: &mut FrameBuffer, x: usize, y: usize, text: &str, scale: usize, on: bool) {
    let mut cursor_x = x;

    for c in text.chars() {
        let glyph = glyph_5x7(normalize_glyph_char(c));
        draw_glyph_5x7(frame, cursor_x, y, &glyph, scale, on);
        cursor_x += 6 * scale;
    }
}

#[derive(Clone, Copy, Debug)]
struct RsvpWordSpec<'a> {
    y: usize,
    word: &'a str,
    scale: usize,
    orp_anchor_percent: usize,
    serif_word: bool,
    stride: usize,
}

fn draw_rsvp_word(frame: &mut FrameBuffer, spec: RsvpWordSpec<'_>, on: bool) {
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

fn choose_word_scale_book(word: &str, max_width: usize, size: FontSize) -> BookWordRender {
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

fn rsvp_word_pixel_width_book(word: &str, scale: usize, stride: usize) -> usize {
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

fn draw_rsvp_word_book(
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

fn rsvp_orp_char_index(word: &str) -> usize {
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

fn rsvp_orp_letter_index(letter_count: usize) -> usize {
    match letter_count {
        0 | 1 => 0,
        2..=5 => 1,
        6..=9 => 2,
        10..=13 => 3,
        _ => 4,
    }
}

fn is_rsvp_letter(c: char) -> bool {
    normalize_glyph_char(c).is_ascii_alphanumeric()
}

#[derive(Clone, Copy, Debug)]
struct GlyphMetrics {
    left: usize,
    width: usize,
    advance: usize,
}

fn glyph_metrics(c: char, glyph: &[u8; 5]) -> GlyphMetrics {
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

fn glyph_spacing(c: char) -> usize {
    match c {
        '.' | ',' | ';' | ':' | '!' | '?' | '\'' => 1,
        _ => 1,
    }
}

fn draw_glyph_5x7(
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

fn draw_glyph_5x7_signed(
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

fn draw_scaled_pixel_signed(frame: &mut FrameBuffer, x: isize, y: isize, scale: usize, on: bool) {
    let scale_i = scale as isize;

    for dy in 0..scale_i {
        for dx in 0..scale_i {
            set_pixel_signed(frame, x + dx, y + dy, on);
        }
    }
}

fn draw_book_glyph_signed(
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

fn draw_rect(frame: &mut FrameBuffer, x: usize, y: usize, w: usize, h: usize, on: bool) {
    if w == 0 || h == 0 {
        return;
    }

    for px in x..(x + w) {
        set_pixel(frame, px, y, on);
        set_pixel(frame, px, y + h - 1, on);
    }
    for py in y..(y + h) {
        set_pixel(frame, x, py, on);
        set_pixel(frame, x + w - 1, py, on);
    }
}

fn draw_filled_rect(frame: &mut FrameBuffer, x: usize, y: usize, w: usize, h: usize, on: bool) {
    for py in y..(y + h) {
        for px in x..(x + w) {
            set_pixel(frame, px, py, on);
        }
    }
}

fn draw_filled_rect_signed(
    frame: &mut FrameBuffer,
    x: isize,
    y: isize,
    w: isize,
    h: isize,
    on: bool,
) {
    if w <= 0 || h <= 0 {
        return;
    }

    for py in 0..h {
        for px in 0..w {
            set_pixel_signed(frame, x + px, y + py, on);
        }
    }
}

fn set_pixel(frame: &mut FrameBuffer, x: usize, y: usize, on: bool) {
    let _ = frame.set_pixel(x, y, on);
}

fn set_pixel_signed(frame: &mut FrameBuffer, x: isize, y: isize, on: bool) {
    if x < 0 || y < 0 {
        return;
    }

    set_pixel(frame, x as usize, y as usize, on);
}

fn normalize_glyph_char(c: char) -> char {
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

fn glyph_5x7(c: char) -> [u8; 5] {
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
