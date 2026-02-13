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

mod countdown;
mod glyph;
mod header;
mod library;
mod loading;
mod navigation;
mod primitives;
mod settings;
mod text;

#[allow(unused_imports)]
use self::{
    countdown::*, glyph::*, header::*, library::*, loading::*, navigation::*, primitives::*,
    settings::*, text::*,
};

pub use self::loading::LoadingView;

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

    pub fn render_loading(&mut self, view: LoadingView<'_>, frame: &mut FrameBuffer) {
        let (bg_on, fg_on) = palette(VisualStyle::default());
        clear_frame(frame, bg_on);
        draw_loading_stage(frame, view, self.connectivity, fg_on);
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
