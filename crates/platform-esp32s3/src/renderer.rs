use core::convert::Infallible;

use app_runtime::{
    AnimationDescriptor, MotionDirection, PreparedScreen, Screen, ScreenUpdate, TransitionPlan,
    components::{
        ContentListShell, ContentRow, DashboardShell, ParagraphNavigationShell, PauseModal,
        ReaderShell, SettingsShell, TopicPreferenceGrid,
    },
};
use domain::formatter::StageFont;
use domain::settings::AppearanceMode;
use domain::ui::TopicRegion;
use embedded_graphics::{
    mono_font::{
        MonoFont, MonoTextStyleBuilder,
        iso_8859_1::{FONT_6X10, FONT_8X13, FONT_8X13_BOLD, FONT_10X20},
    },
    pixelcolor::BinaryColor,
    prelude::*,
    text::{Alignment, Baseline, Text, TextStyleBuilder},
};
use heapless::String as HeaplessString;
use ls027b7dh01::FrameBuffer;

pub const UI_TICK_MS: u64 = 160;
const NORMALIZED_TEXT_MAX_BYTES: usize = 192;
const ELLIPSIS: &str = "...";
const RSVP_STAGE_CENTER_X: i32 = 170;
const RSVP_STAGE_LEFT_ANCHOR_X: i32 = 169;
const RSVP_STAGE_RIGHT_ANCHOR_X: i32 = 173;
const RSVP_STAGE_SCALED_LEFT_ANCHOR_X: i32 = 168;
const RSVP_STAGE_SCALED_RIGHT_ANCHOR_X: i32 = 172;
const LARGE_RAIL_SCALE: u32 = 2;
const LARGE_RAIL_RIGHT_EDGE_X: i32 = 399;
const LARGE_RAIL_Y: i32 = 18;
const DASHBOARD_TEXT_RIGHT_EDGE_X: i32 = 296;
const DASHBOARD_SLOT_TRAVEL_PX: i32 = 14;
const DASHBOARD_TOP_SLOT_Y: i32 = 42;
const DASHBOARD_TOP_SLOT_HEIGHT: i32 = 40;
const DASHBOARD_SELECTED_SLOT_Y: i32 = 82;
const DASHBOARD_SELECTED_SLOT_HEIGHT: i32 = 60;
const DASHBOARD_BOTTOM_SLOT_Y: i32 = 149;
const DASHBOARD_BOTTOM_SLOT_HEIGHT: i32 = 42;
const COLLECTION_TEXT_RIGHT_EDGE_X: i32 = 316;
const COLLECTION_LIST_STEP_TRAVEL_PX: i32 = 16;
const COLLECTION_TOP_SLOT_Y: i32 = 42;
const COLLECTION_TOP_SLOT_HEIGHT: i32 = 42;
const COLLECTION_SELECTED_SLOT_Y: i32 = 106;
const COLLECTION_SELECTED_SLOT_HEIGHT: i32 = 68;
const COLLECTION_BOTTOM_SLOT_Y: i32 = 179;
const COLLECTION_BOTTOM_SLOT_HEIGHT: i32 = 42;
const COLLECTION_SPINNER_CENTER_X: i32 = 350;
const COLLECTION_SPINNER_CLIP_RIGHT_PAD: i32 = 24;
const PARAGRAPH_SLOT_TRAVEL_PX: i32 = 18;
const PARAGRAPH_TOP_SLOT_Y: i32 = 52;
const PARAGRAPH_TOP_SLOT_HEIGHT: i32 = 24;
const PARAGRAPH_SELECTED_SLOT_Y: i32 = 88;
const PARAGRAPH_SELECTED_SLOT_HEIGHT: i32 = 82;
const PARAGRAPH_BOTTOM_SLOT_Y: i32 = 184;
const PARAGRAPH_BOTTOM_SLOT_HEIGHT: i32 = 44;
const PARAGRAPH_CARD_X: i32 = 20;
const PARAGRAPH_CARD_Y: i32 = 88;
const PARAGRAPH_CARD_WIDTH: i32 = 304;
const PARAGRAPH_CARD_HEIGHT: i32 = 82;
const PARAGRAPH_CARD_LABEL_Y: i32 = 106;
const PARAGRAPH_CARD_EXCERPT_Y: i32 = 126;
const PARAGRAPH_CARD_HINT_X: i32 = 236;
const PARAGRAPH_CARD_HINT_Y: i32 = 146;
const PARAGRAPH_CARD_HINT_WIDTH: i32 = 72;
const PARAGRAPH_CARD_HINT_HEIGHT: i32 = 16;
const PARAGRAPH_FOOTER_Y: i32 = 231;
const PAUSE_MODAL_CENTER_X: i32 = 200;
const PAUSE_MODAL_CENTER_Y: i32 = 118;
const PAUSE_MODAL_MIN_WIDTH: u32 = 112;
const PAUSE_MODAL_MIN_HEIGHT: u32 = 52;
const PAUSE_MODAL_MAX_WIDTH: u32 = 286;
const PAUSE_MODAL_MAX_HEIGHT: u32 = 166;
const PAUSE_MODAL_CONTENT_OFFSET_PX: i32 = 8;
const READER_TEXT_LEFT_X: i32 = 20;
const READER_TEXT_RIGHT_X: i32 = 380;
const READER_TITLE_MAX_WIDTH_PX: i32 = READER_TEXT_RIGHT_X - READER_TEXT_LEFT_X;
const READER_FOOTER_WPM_GAP_PX: i32 = 16;
const READER_PREVIEW_Y: i32 = 214;

#[derive(Debug, Clone, Copy, PartialEq)]
struct StageTextSpec {
    font: &'static MonoFont<'static>,
    y: i32,
    scale: u32,
    left_anchor_x: i32,
    right_anchor_x: i32,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct ClipRect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct CollectionRowSlot {
    text_x: i32,
    meta_y: i32,
    title_y: i32,
    color: BinaryColor,
    clip: ClipRect,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct ClippedTextSpec {
    position: Point,
    color: BinaryColor,
    alignment: Alignment,
    max_width_px: i32,
}

#[derive(Debug, Clone, Copy)]
struct TextPairSlotSpec {
    slot: CollectionRowSlot,
    font: &'static MonoFont<'static>,
    max_width_px: i32,
}

fn ui_font_small() -> &'static MonoFont<'static> {
    &FONT_6X10
}

fn ui_font_body() -> &'static MonoFont<'static> {
    &FONT_8X13
}

fn ui_font_title() -> &'static MonoFont<'static> {
    &FONT_10X20
}

fn stage_font_spec(font: StageFont) -> StageTextSpec {
    match font {
        StageFont::Large => StageTextSpec {
            font: ui_font_title(),
            y: 102,
            scale: 2,
            left_anchor_x: RSVP_STAGE_SCALED_LEFT_ANCHOR_X,
            right_anchor_x: RSVP_STAGE_SCALED_RIGHT_ANCHOR_X,
        },
        StageFont::Medium => StageTextSpec {
            font: &FONT_8X13_BOLD,
            y: 108,
            scale: 2,
            left_anchor_x: RSVP_STAGE_SCALED_LEFT_ANCHOR_X,
            right_anchor_x: RSVP_STAGE_SCALED_RIGHT_ANCHOR_X,
        },
        StageFont::Small => StageTextSpec {
            font: ui_font_title(),
            y: 112,
            scale: 1,
            left_anchor_x: RSVP_STAGE_LEFT_ANCHOR_X,
            right_anchor_x: RSVP_STAGE_RIGHT_ANCHOR_X,
        },
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct AnimationPlayback {
    pub from: PreparedScreen,
    pub to: PreparedScreen,
    pub screen: Screen,
    pub plan: TransitionPlan,
    pub step: u8,
}

impl AnimationPlayback {
    pub const fn new(from: PreparedScreen, update: ScreenUpdate) -> Self {
        Self {
            from,
            to: update.prepared,
            screen: update.screen,
            plan: update.transition,
            step: 1,
        }
    }

    pub const fn advance(mut self) -> Self {
        self.step = self.step.saturating_add(1);
        self
    }

    pub const fn is_complete(&self) -> bool {
        self.step >= self.plan.steps
    }

    pub const fn target_screen(&self) -> PreparedScreen {
        self.to
    }
}

pub fn draw_prepared_screen(frame: &mut FrameBuffer, screen: &PreparedScreen) {
    frame.clear(false);

    match screen {
        PreparedScreen::Dashboard(shell) => draw_dashboard(frame, shell, 1, 1),
        PreparedScreen::Collection(shell) => draw_collection(frame, shell, 1, 1, 0),
        PreparedScreen::Reader(shell) => draw_reader(frame, shell, 1, 1),
        PreparedScreen::ParagraphNavigation(shell) => draw_paragraph_navigation(frame, shell, 1, 1),
        PreparedScreen::Settings(shell) => draw_settings(frame, shell, 1, 1),
    }

    apply_theme(frame, screen.appearance());
}

pub fn draw_transition_frame(frame: &mut FrameBuffer, playback: &AnimationPlayback) {
    frame.clear(false);

    match playback.plan.animation {
        AnimationDescriptor::None => draw_prepared_screen(frame, &playback.to),
        AnimationDescriptor::BandReveal(direction) => match playback.to {
            PreparedScreen::Dashboard(shell) => {
                if let PreparedScreen::Dashboard(from_shell) = playback.from {
                    draw_dashboard_band_transition(
                        frame,
                        &from_shell,
                        &shell,
                        direction,
                        playback.step,
                        playback.plan.steps,
                    );
                } else {
                    draw_dashboard(frame, &shell, playback.step, playback.plan.steps);
                }
            }
            PreparedScreen::Settings(shell) => {
                draw_settings(frame, &shell, playback.step, playback.plan.steps)
            }
            PreparedScreen::Collection(shell) => {
                draw_collection(frame, &shell, playback.step, playback.plan.steps, 0)
            }
            _ => draw_prepared_screen(frame, &playback.to),
        },
        AnimationDescriptor::ListStep(direction) => {
            if let (PreparedScreen::Collection(from_shell), PreparedScreen::Collection(to_shell)) =
                (playback.from, playback.to)
            {
                draw_collection_list_step(
                    frame,
                    &from_shell,
                    &to_shell,
                    direction,
                    playback.step,
                    playback.plan.steps,
                );
            } else {
                draw_prepared_screen(frame, &playback.to);
            }
        }
        AnimationDescriptor::ReaderEnter => {
            if let PreparedScreen::Reader(shell) = playback.to {
                draw_reader(frame, &shell, playback.step, playback.plan.steps);
            } else {
                draw_prepared_screen(frame, &playback.to);
            }
        }
        AnimationDescriptor::ReaderExit => {
            draw_prepared_screen(frame, &playback.to);
        }
        AnimationDescriptor::ModalReveal => {
            if let (PreparedScreen::Reader(from_shell), PreparedScreen::Reader(to_shell)) =
                (playback.from, playback.to)
            {
                draw_reader_modal_transition(
                    frame,
                    &from_shell,
                    &to_shell,
                    true,
                    playback.step,
                    playback.plan.steps,
                );
            } else if let PreparedScreen::Reader(shell) = playback.to {
                draw_reader(frame, &shell, playback.step, playback.plan.steps);
            } else {
                draw_prepared_screen(frame, &playback.to);
            }
        }
        AnimationDescriptor::ModalHide => {
            if let (PreparedScreen::Reader(from_shell), PreparedScreen::Reader(to_shell)) =
                (playback.from, playback.to)
            {
                draw_reader_modal_transition(
                    frame,
                    &from_shell,
                    &to_shell,
                    false,
                    playback.step,
                    playback.plan.steps,
                );
            } else if let PreparedScreen::Reader(shell) = playback.to {
                draw_reader(frame, &shell, playback.step, playback.plan.steps);
            } else {
                draw_prepared_screen(frame, &playback.to);
            }
        }
        AnimationDescriptor::ParagraphTickMove(direction) => {
            if let (
                PreparedScreen::ParagraphNavigation(from_shell),
                PreparedScreen::ParagraphNavigation(to_shell),
            ) = (playback.from, playback.to)
            {
                draw_paragraph_navigation_transition(
                    frame,
                    &from_shell,
                    &to_shell,
                    direction,
                    playback.step,
                    playback.plan.steps,
                );
            } else {
                draw_prepared_screen(frame, &playback.to);
            }
        }
        AnimationDescriptor::SettingsValuePulse => {
            if let PreparedScreen::Settings(shell) = playback.to {
                draw_settings(frame, &shell, 1, 1);
                draw_value_pulse(frame, playback.step, playback.plan.steps);
            } else {
                draw_prepared_screen(frame, &playback.to);
            }
        }
        AnimationDescriptor::AppearanceFlip => {
            if let PreparedScreen::Settings(shell) = playback.to {
                draw_settings(frame, &shell, 1, 1);
                draw_row_flash(frame, 88 + 1, 30, playback.step, playback.plan.steps);
            } else {
                draw_prepared_screen(frame, &playback.to);
            }
        }
        AnimationDescriptor::RefreshPulse => {
            if let PreparedScreen::Settings(shell) = playback.to {
                draw_settings(frame, &shell, 1, 1);
                draw_refresh_pulse(frame, playback.step, playback.plan.steps);
            } else {
                draw_prepared_screen(frame, &playback.to);
            }
        }
    }

    apply_theme(frame, playback.to.appearance());
}

fn draw_dashboard(frame: &mut FrameBuffer, shell: &DashboardShell, step: u8, total_steps: u8) {
    draw_dashboard_chrome(frame, shell);
    draw_dashboard_row_at(frame, shell.items[0].label, dashboard_top_slot());
    draw_selection_band(
        frame,
        16,
        shell.band.y,
        320,
        shell.band.height as i32,
        step,
        total_steps,
    );
    draw_dashboard_row_at(frame, shell.items[1].label, dashboard_selected_slot());

    if step >= total_steps {
        draw_dashboard_selected_band_accent(frame);
    }

    draw_dashboard_row_at(frame, shell.items[2].label, dashboard_bottom_slot());
}

fn draw_dashboard_sync_indicator(frame: &mut FrameBuffer, label: &str, spinner_phase: u8) {
    draw_text(
        frame,
        sync_spinner_frame(spinner_phase),
        Point::new(314, 220),
        ui_font_small(),
        BinaryColor::On,
        Alignment::Left,
    );
    draw_text_right(
        frame,
        label,
        Point::new(382, 220),
        ui_font_small(),
        BinaryColor::On,
    );
}

fn draw_dashboard_band_transition(
    frame: &mut FrameBuffer,
    from: &DashboardShell,
    to: &DashboardShell,
    direction: MotionDirection,
    step: u8,
    total_steps: u8,
) {
    if step >= total_steps {
        draw_dashboard(frame, to, 1, 1);
        return;
    }

    draw_dashboard_chrome(frame, to);
    draw_dashboard_slot_transition(
        frame,
        from.items[0].label,
        to.items[0].label,
        dashboard_top_slot(),
        direction,
        step,
        total_steps,
    );
    fill_rect(
        frame,
        16,
        DASHBOARD_SELECTED_SLOT_Y,
        320,
        DASHBOARD_SELECTED_SLOT_HEIGHT,
        BinaryColor::On,
    );
    draw_selection_band(
        frame,
        16,
        DASHBOARD_SELECTED_SLOT_Y,
        320,
        DASHBOARD_SELECTED_SLOT_HEIGHT,
        1,
        1,
    );
    draw_dashboard_slot_transition(
        frame,
        from.items[1].label,
        to.items[1].label,
        dashboard_selected_slot(),
        direction,
        step,
        total_steps,
    );
    draw_dashboard_selected_band_accent(frame);
    draw_dashboard_slot_transition(
        frame,
        from.items[2].label,
        to.items[2].label,
        dashboard_bottom_slot(),
        direction,
        step,
        total_steps,
    );
}

fn draw_dashboard_chrome(frame: &mut FrameBuffer, shell: &DashboardShell) {
    draw_status_cluster(
        frame,
        shell.status.battery_percent,
        shell.status.wifi_online,
    );
    draw_vertical_rail_scaled(
        frame,
        shell.rail.text,
        LARGE_RAIL_RIGHT_EDGE_X,
        LARGE_RAIL_Y,
        LARGE_RAIL_SCALE,
    );

    if let Some(sync_indicator) = shell.sync_indicator {
        draw_dashboard_sync_indicator(frame, sync_indicator.label, sync_indicator.spinner_phase);
    }
}

fn draw_dashboard_row_at(frame: &mut FrameBuffer, label: &str, slot: CollectionRowSlot) {
    draw_text_ellipsized(
        frame,
        label,
        Point::new(slot.text_x, slot.meta_y),
        ui_font_title(),
        slot.color,
        Alignment::Left,
        DASHBOARD_TEXT_RIGHT_EDGE_X - slot.text_x,
    );
}

fn draw_dashboard_slot_transition(
    frame: &mut FrameBuffer,
    from: &str,
    to: &str,
    slot: CollectionRowSlot,
    direction: MotionDirection,
    step: u8,
    total_steps: u8,
) {
    let incoming_offset = slide_offset(direction, step, total_steps, DASHBOARD_SLOT_TRAVEL_PX);
    let outgoing_offset = match direction {
        MotionDirection::Forward => incoming_offset - DASHBOARD_SLOT_TRAVEL_PX,
        MotionDirection::Backward => incoming_offset + DASHBOARD_SLOT_TRAVEL_PX,
    };

    draw_text_ellipsized_clipped(
        frame,
        from,
        ui_font_title(),
        ClippedTextSpec {
            position: Point::new(slot.text_x, slot.meta_y + outgoing_offset),
            color: slot.color,
            alignment: Alignment::Left,
            max_width_px: DASHBOARD_TEXT_RIGHT_EDGE_X - slot.text_x,
        },
        slot.clip,
    );
    draw_text_ellipsized_clipped(
        frame,
        to,
        ui_font_title(),
        ClippedTextSpec {
            position: Point::new(slot.text_x, slot.meta_y + incoming_offset),
            color: slot.color,
            alignment: Alignment::Left,
            max_width_px: DASHBOARD_TEXT_RIGHT_EDGE_X - slot.text_x,
        },
        slot.clip,
    );
}

fn draw_dashboard_selected_band_accent(frame: &mut FrameBuffer) {
    fill_rect(frame, 303, 105, 28, 5, BinaryColor::Off);
    fill_rect(frame, 307, 116, 22, 5, BinaryColor::Off);
}

fn sync_spinner_frame(spinner_phase: u8) -> &'static str {
    match spinner_phase % 4 {
        0 => "|",
        1 => "/",
        2 => "-",
        _ => "\\",
    }
}

const fn collection_slot(
    text_x: i32,
    meta_y: i32,
    title_y: i32,
    color: BinaryColor,
    clip_y: i32,
    clip_height: i32,
) -> CollectionRowSlot {
    CollectionRowSlot {
        text_x,
        meta_y,
        title_y,
        color,
        clip: ClipRect {
            x: 16,
            y: clip_y,
            width: 320,
            height: clip_height,
        },
    }
}

const fn collection_top_slot() -> CollectionRowSlot {
    collection_slot(
        20,
        48,
        66,
        BinaryColor::On,
        COLLECTION_TOP_SLOT_Y,
        COLLECTION_TOP_SLOT_HEIGHT,
    )
}

const fn collection_selected_slot() -> CollectionRowSlot {
    collection_slot(
        28,
        118,
        135,
        BinaryColor::Off,
        COLLECTION_SELECTED_SLOT_Y,
        COLLECTION_SELECTED_SLOT_HEIGHT,
    )
}

const fn collection_bottom_slot() -> CollectionRowSlot {
    collection_slot(
        20,
        185,
        203,
        BinaryColor::On,
        COLLECTION_BOTTOM_SLOT_Y,
        COLLECTION_BOTTOM_SLOT_HEIGHT,
    )
}

const fn dashboard_slot(
    text_x: i32,
    text_y: i32,
    color: BinaryColor,
    clip_y: i32,
    clip_height: i32,
) -> CollectionRowSlot {
    collection_slot(text_x, text_y, text_y, color, clip_y, clip_height)
}

const fn dashboard_top_slot() -> CollectionRowSlot {
    dashboard_slot(
        20,
        54,
        BinaryColor::On,
        DASHBOARD_TOP_SLOT_Y,
        DASHBOARD_TOP_SLOT_HEIGHT,
    )
}

const fn dashboard_selected_slot() -> CollectionRowSlot {
    dashboard_slot(
        30,
        92,
        BinaryColor::Off,
        DASHBOARD_SELECTED_SLOT_Y,
        DASHBOARD_SELECTED_SLOT_HEIGHT,
    )
}

const fn dashboard_bottom_slot() -> CollectionRowSlot {
    dashboard_slot(
        20,
        161,
        BinaryColor::On,
        DASHBOARD_BOTTOM_SLOT_Y,
        DASHBOARD_BOTTOM_SLOT_HEIGHT,
    )
}

const fn paragraph_top_slot() -> CollectionRowSlot {
    collection_slot(
        20,
        58,
        58,
        BinaryColor::On,
        PARAGRAPH_TOP_SLOT_Y,
        PARAGRAPH_TOP_SLOT_HEIGHT,
    )
}

const fn paragraph_bottom_slot() -> CollectionRowSlot {
    collection_slot(
        20,
        188,
        210,
        BinaryColor::On,
        PARAGRAPH_BOTTOM_SLOT_Y,
        PARAGRAPH_BOTTOM_SLOT_HEIGHT,
    )
}

fn paragraph_top_slot_spec() -> TextPairSlotSpec {
    TextPairSlotSpec {
        slot: paragraph_top_slot(),
        font: ui_font_body(),
        max_width_px: 320,
    }
}

fn paragraph_bottom_slot_spec() -> TextPairSlotSpec {
    TextPairSlotSpec {
        slot: paragraph_bottom_slot(),
        font: ui_font_body(),
        max_width_px: 320,
    }
}

fn draw_collection(
    frame: &mut FrameBuffer,
    shell: &ContentListShell,
    step: u8,
    total_steps: u8,
    slide_offset: i32,
) {
    draw_collection_chrome(frame, shell);
    draw_collection_row_at(
        frame,
        &shell.rows[0],
        Point::new(20, 48 + slide_offset),
        Point::new(20, 66 + slide_offset),
        BinaryColor::On,
    );

    draw_selection_band(
        frame,
        16,
        shell.band.y,
        320,
        shell.band.height as i32,
        step,
        total_steps,
    );
    draw_collection_row_at(
        frame,
        &shell.rows[1],
        Point::new(28, 118 + slide_offset),
        Point::new(28, 135 + slide_offset),
        BinaryColor::Off,
    );

    if step >= total_steps {
        draw_collection_selected_band_accent(frame);
    }

    draw_collection_row_at(
        frame,
        &shell.rows[2],
        Point::new(20, 185 + slide_offset),
        Point::new(20, 203 + slide_offset),
        BinaryColor::On,
    );
}

fn draw_collection_list_step(
    frame: &mut FrameBuffer,
    from: &ContentListShell,
    to: &ContentListShell,
    direction: MotionDirection,
    step: u8,
    total_steps: u8,
) {
    if step >= total_steps {
        draw_collection(frame, to, 1, 1, 0);
        return;
    }

    draw_collection_chrome(frame, to);
    draw_collection_list_step_slots(frame, from, to, direction, step, total_steps);
}

fn draw_collection_chrome(frame: &mut FrameBuffer, shell: &ContentListShell) {
    draw_status_cluster(
        frame,
        shell.status.battery_percent,
        shell.status.wifi_online,
    );
    draw_text(
        frame,
        shell.help.text,
        Point::new(38, 13),
        ui_font_small(),
        BinaryColor::On,
        Alignment::Left,
    );
    draw_back_chevron(frame, 20, 12);
    if shell.large_rail {
        draw_vertical_rail_scaled(
            frame,
            shell.rail.text,
            LARGE_RAIL_RIGHT_EDGE_X,
            LARGE_RAIL_Y,
            LARGE_RAIL_SCALE,
        );
    } else {
        draw_vertical_rail(frame, shell.rail.text, 398, 26);
    }
}

fn draw_collection_list_step_slots(
    frame: &mut FrameBuffer,
    from: &ContentListShell,
    to: &ContentListShell,
    direction: MotionDirection,
    step: u8,
    total_steps: u8,
) {
    let incoming_offset =
        slide_offset(direction, step, total_steps, COLLECTION_LIST_STEP_TRAVEL_PX);
    let outgoing_offset = match direction {
        MotionDirection::Forward => incoming_offset - COLLECTION_LIST_STEP_TRAVEL_PX,
        MotionDirection::Backward => incoming_offset + COLLECTION_LIST_STEP_TRAVEL_PX,
    };

    draw_collection_row_slot_transition(
        frame,
        &from.rows[0],
        &to.rows[0],
        collection_top_slot(),
        incoming_offset,
        outgoing_offset,
    );
    fill_rect(
        frame,
        16,
        COLLECTION_SELECTED_SLOT_Y,
        320,
        COLLECTION_SELECTED_SLOT_HEIGHT,
        BinaryColor::On,
    );
    draw_selection_band(
        frame,
        16,
        COLLECTION_SELECTED_SLOT_Y,
        320,
        COLLECTION_SELECTED_SLOT_HEIGHT,
        1,
        1,
    );
    draw_collection_row_slot_transition(
        frame,
        &from.rows[1],
        &to.rows[1],
        collection_selected_slot(),
        incoming_offset,
        outgoing_offset,
    );
    draw_collection_selected_band_accent(frame);
    draw_collection_row_slot_transition(
        frame,
        &from.rows[2],
        &to.rows[2],
        collection_bottom_slot(),
        incoming_offset,
        outgoing_offset,
    );
}

fn draw_collection_row_slot_transition(
    frame: &mut FrameBuffer,
    from: &ContentRow,
    to: &ContentRow,
    slot: CollectionRowSlot,
    incoming_offset: i32,
    outgoing_offset: i32,
) {
    draw_collection_row_at_clipped(
        frame,
        from,
        Point::new(slot.text_x, slot.meta_y + outgoing_offset),
        Point::new(slot.text_x, slot.title_y + outgoing_offset),
        slot.color,
        slot.clip,
    );
    draw_collection_row_at_clipped(
        frame,
        to,
        Point::new(slot.text_x, slot.meta_y + incoming_offset),
        Point::new(slot.text_x, slot.title_y + incoming_offset),
        slot.color,
        slot.clip,
    );
}

fn draw_collection_row_at(
    frame: &mut FrameBuffer,
    row: &ContentRow,
    meta_position: Point,
    title_position: Point,
    color: BinaryColor,
) {
    draw_text_ellipsized(
        frame,
        row.meta.as_str(),
        meta_position,
        ui_font_body(),
        color,
        Alignment::Left,
        COLLECTION_TEXT_RIGHT_EDGE_X - meta_position.x,
    );
    draw_text_ellipsized(
        frame,
        row.title.as_str(),
        title_position,
        ui_font_body(),
        color,
        Alignment::Left,
        COLLECTION_TEXT_RIGHT_EDGE_X - title_position.x,
    );

    if let Some(spinner_phase) = row.loading_phase {
        draw_collection_loading_spinner(
            frame,
            spinner_phase,
            collection_spinner_center(meta_position, title_position),
            color,
            None,
        );
    }
}

fn draw_collection_selected_band_accent(frame: &mut FrameBuffer) {
    fill_rect(frame, 303, 129, 28, 5, BinaryColor::Off);
    fill_rect(frame, 307, 140, 22, 5, BinaryColor::Off);
}

fn draw_collection_row_at_clipped(
    frame: &mut FrameBuffer,
    row: &ContentRow,
    meta_position: Point,
    title_position: Point,
    color: BinaryColor,
    clip: ClipRect,
) {
    draw_text_ellipsized_clipped(
        frame,
        row.meta.as_str(),
        ui_font_body(),
        ClippedTextSpec {
            position: meta_position,
            color,
            alignment: Alignment::Left,
            max_width_px: COLLECTION_TEXT_RIGHT_EDGE_X - meta_position.x,
        },
        clip,
    );
    draw_text_ellipsized_clipped(
        frame,
        row.title.as_str(),
        ui_font_body(),
        ClippedTextSpec {
            position: title_position,
            color,
            alignment: Alignment::Left,
            max_width_px: COLLECTION_TEXT_RIGHT_EDGE_X - title_position.x,
        },
        clip,
    );

    if let Some(spinner_phase) = row.loading_phase {
        draw_collection_loading_spinner(
            frame,
            spinner_phase,
            collection_spinner_center(meta_position, title_position),
            color,
            Some(collection_spinner_clip(clip)),
        );
    }
}

fn collection_spinner_center(meta_position: Point, title_position: Point) -> Point {
    Point::new(
        COLLECTION_SPINNER_CENTER_X,
        meta_position.y + ((title_position.y - meta_position.y) / 2) + 2,
    )
}

const fn collection_spinner_clip(clip: ClipRect) -> ClipRect {
    ClipRect {
        x: clip.x,
        y: clip.y,
        width: clip.width + COLLECTION_SPINNER_CLIP_RIGHT_PAD,
        height: clip.height,
    }
}

fn draw_collection_loading_spinner(
    frame: &mut FrameBuffer,
    spinner_phase: u8,
    center: Point,
    color: BinaryColor,
    clip: Option<ClipRect>,
) {
    fill_rect_clipped(frame, center.x - 1, center.y - 1, 3, 3, color, clip);

    for (x, y) in [
        (center.x, center.y - 4),
        (center.x + 4, center.y),
        (center.x, center.y + 4),
        (center.x - 4, center.y),
    ] {
        fill_rect_clipped(frame, x, y, 1, 1, color, clip);
    }

    match spinner_phase % 4 {
        0 => fill_rect_clipped(frame, center.x - 1, center.y - 6, 3, 3, color, clip),
        1 => fill_rect_clipped(frame, center.x + 4, center.y - 1, 3, 3, color, clip),
        2 => fill_rect_clipped(frame, center.x - 1, center.y + 4, 3, 3, color, clip),
        _ => fill_rect_clipped(frame, center.x - 6, center.y - 1, 3, 3, color, clip),
    }
}

fn draw_reader(frame: &mut FrameBuffer, shell: &ReaderShell, step: u8, total_steps: u8) {
    draw_reader_base(frame, shell, step, total_steps);

    if let Some(modal) = shell.pause_modal {
        draw_pause_modal(frame, &modal, step, total_steps);
    }
}

fn draw_reader_base(frame: &mut FrameBuffer, shell: &ReaderShell, step: u8, total_steps: u8) {
    draw_text_ellipsized(
        frame,
        shell.stage.title.as_str(),
        Point::new(READER_TEXT_LEFT_X, 18),
        ui_font_title(),
        BinaryColor::On,
        Alignment::Left,
        READER_TITLE_MAX_WIDTH_PX,
    );

    let stage_ready = step.saturating_mul(2) >= total_steps;
    if stage_ready {
        draw_stage_token(
            frame,
            shell.stage.left_word.as_str(),
            shell.stage.right_word.as_str(),
            shell.stage.font,
        );
        fill_rect(frame, RSVP_STAGE_CENTER_X, 84, 1, 76, BinaryColor::On);
    }

    if step >= total_steps {
        fill_rect(frame, 24, 121, 18, 4, BinaryColor::On);
        fill_rect(frame, 344, 121, 18, 4, BinaryColor::On);
    }

    if let Some(badge) = shell.badge {
        draw_selection_band(frame, 126, 66, 58, 22, step.min(2), 2);
        if step >= 2 {
            draw_text(
                frame,
                badge.label,
                Point::new(140, 69),
                ui_font_small(),
                BinaryColor::Off,
                Alignment::Left,
            );
        }
    }

    draw_text_ellipsized(
        frame,
        shell.stage.preview.as_str(),
        Point::new(READER_TEXT_LEFT_X, READER_PREVIEW_Y),
        ui_font_body(),
        BinaryColor::On,
        Alignment::Left,
        reader_preview_max_width_px(shell.stage.wpm),
    );
    draw_text_right(
        frame,
        wpm_label(shell.stage.wpm),
        Point::new(READER_TEXT_RIGHT_X, READER_PREVIEW_Y),
        ui_font_body(),
        BinaryColor::On,
    );
    fill_rect(
        frame,
        0,
        232,
        shell.stage.progress_width.into(),
        8,
        BinaryColor::On,
    );
}

fn draw_pause_modal(frame: &mut FrameBuffer, modal: &PauseModal, step: u8, total_steps: u8) {
    draw_pause_modal_transition(frame, modal, step, total_steps, true);
}

fn draw_reader_modal_transition(
    frame: &mut FrameBuffer,
    from: &ReaderShell,
    to: &ReaderShell,
    revealing: bool,
    step: u8,
    total_steps: u8,
) {
    draw_reader_base(frame, to, 1, 1);

    let modal = if revealing {
        to.pause_modal
    } else {
        from.pause_modal
    };

    if let Some(modal) = modal {
        draw_pause_modal_transition(frame, &modal, step, total_steps, revealing);
    }
}

fn draw_pause_modal_transition(
    frame: &mut FrameBuffer,
    modal: &PauseModal,
    step: u8,
    total_steps: u8,
    revealing: bool,
) {
    let phase = if revealing {
        step
    } else {
        total_steps.saturating_sub(step).saturating_add(1)
    };
    let content_phase = if revealing {
        phase
    } else {
        total_steps.saturating_sub(step)
    };

    if !revealing && content_phase == 0 {
        return;
    }

    let width = lerp_u32(
        PAUSE_MODAL_MIN_WIDTH,
        PAUSE_MODAL_MAX_WIDTH,
        phase,
        total_steps,
    );
    let height = lerp_u32(
        PAUSE_MODAL_MIN_HEIGHT,
        PAUSE_MODAL_MAX_HEIGHT,
        phase,
        total_steps,
    );
    let x = PAUSE_MODAL_CENTER_X - (width as i32 / 2);
    let y = PAUSE_MODAL_CENTER_Y - (height as i32 / 2);
    let clip = ClipRect {
        x,
        y,
        width: width as i32,
        height: height as i32,
    };
    let content_offset = ((total_steps.saturating_sub(content_phase) as i32)
        * PAUSE_MODAL_CONTENT_OFFSET_PX)
        / total_steps.max(1) as i32;
    let divider_width = lerp_u32(0, width.saturating_sub(32), content_phase, total_steps) as i32;

    fill_rect(frame, x, y, width as i32, height as i32, BinaryColor::On);
    stroke_rect(frame, x, y, width as i32, height as i32, BinaryColor::Off);

    draw_text_ellipsized_clipped(
        frame,
        modal.title,
        ui_font_title(),
        ClippedTextSpec {
            position: Point::new(PAUSE_MODAL_CENTER_X, y + 14 + content_offset),
            color: BinaryColor::Off,
            alignment: Alignment::Center,
            max_width_px: width as i32 - 32,
        },
        clip,
    );

    if divider_width > 0 {
        fill_rect(
            frame,
            PAUSE_MODAL_CENTER_X - (divider_width / 2),
            y + 46,
            divider_width,
            1,
            BinaryColor::Off,
        );
    }

    if content_phase >= 1 {
        draw_pause_modal_row(
            frame,
            &modal.rows[0],
            Point::new(x + 18, y + 60 + content_offset),
            clip,
        );
    }
    if content_phase >= 2 {
        draw_pause_modal_row(
            frame,
            &modal.rows[1],
            Point::new(x + 18, y + 88 + content_offset),
            clip,
        );
        draw_pause_modal_row(
            frame,
            &modal.rows[2],
            Point::new(x + 18, y + 116 + content_offset),
            clip,
        );
    }
}

fn draw_pause_modal_row(
    frame: &mut FrameBuffer,
    row: &app_runtime::components::PauseModalRow,
    position: Point,
    clip: ClipRect,
) {
    draw_text_ellipsized_clipped(
        frame,
        row.label,
        ui_font_small(),
        ClippedTextSpec {
            position,
            color: BinaryColor::Off,
            alignment: Alignment::Left,
            max_width_px: 140,
        },
        clip,
    );
    draw_text_ellipsized_clipped(
        frame,
        row.action,
        ui_font_small(),
        ClippedTextSpec {
            position: Point::new(position.x + 148, position.y),
            color: BinaryColor::Off,
            alignment: Alignment::Left,
            max_width_px: 110,
        },
        clip,
    );
}

fn draw_stage_token(frame: &mut FrameBuffer, left: &str, right: &str, font: StageFont) {
    let spec = stage_font_spec(font);

    if spec.scale == 1 {
        draw_text_right(
            frame,
            left,
            Point::new(spec.left_anchor_x, spec.y),
            spec.font,
            BinaryColor::On,
        );
        draw_text(
            frame,
            right,
            Point::new(spec.right_anchor_x, spec.y),
            spec.font,
            BinaryColor::On,
            Alignment::Left,
        );
    } else {
        draw_text_right_scaled(
            frame,
            left,
            Point::new(spec.left_anchor_x, spec.y),
            spec.font,
            BinaryColor::On,
            spec.scale,
        );
        draw_text_scaled(
            frame,
            right,
            Point::new(spec.right_anchor_x, spec.y),
            spec.font,
            BinaryColor::On,
            Alignment::Left,
            spec.scale,
        );
    }
}

fn draw_paragraph_navigation(
    frame: &mut FrameBuffer,
    shell: &ParagraphNavigationShell,
    _step: u8,
    _total_steps: u8,
) {
    draw_paragraph_navigation_chrome(frame, shell);
    draw_paragraph_body(frame, shell);
    draw_paragraph_map_rail(frame, shell.rail.selected_index, shell.rail.total_ticks);
}

fn draw_paragraph_navigation_transition(
    frame: &mut FrameBuffer,
    from: &ParagraphNavigationShell,
    to: &ParagraphNavigationShell,
    direction: MotionDirection,
    step: u8,
    total_steps: u8,
) {
    if step >= total_steps {
        draw_paragraph_navigation(frame, to, 1, 1);
        return;
    }

    let offsets = slot_transition_offsets(direction, step, total_steps, PARAGRAPH_SLOT_TRAVEL_PX);
    let from_top_line = paragraph_top_line(from);
    let to_top_line = paragraph_top_line(to);
    let from_bottom_primary = paragraph_bottom_primary_line(from);
    let to_bottom_primary = paragraph_bottom_primary_line(to);
    let from_bottom_secondary = paragraph_bottom_secondary_line(from);
    let to_bottom_secondary = paragraph_bottom_secondary_line(to);

    draw_paragraph_navigation_chrome(frame, to);
    draw_text_pair_slot_transition(
        frame,
        from_top_line.as_str(),
        None,
        to_top_line.as_str(),
        None,
        paragraph_top_slot_spec(),
        offsets,
    );
    draw_paragraph_selected_card_transition(frame, from, to, offsets);
    draw_text_pair_slot_transition(
        frame,
        from_bottom_primary.as_str(),
        from_bottom_secondary.as_ref().map(|line| line.as_str()),
        to_bottom_primary.as_str(),
        to_bottom_secondary.as_ref().map(|line| line.as_str()),
        paragraph_bottom_slot_spec(),
        offsets,
    );
    draw_paragraph_map_rail_transition(
        frame,
        from.rail.selected_index,
        to.rail.selected_index,
        to.rail.total_ticks,
        step,
        total_steps,
    );
}

fn draw_paragraph_navigation_chrome(frame: &mut FrameBuffer, shell: &ParagraphNavigationShell) {
    draw_text(
        frame,
        shell.title.as_str(),
        Point::new(20, 18),
        ui_font_title(),
        BinaryColor::On,
        Alignment::Left,
    );
    draw_text_right(
        frame,
        shell.counter.as_str(),
        Point::new(380, 18),
        ui_font_body(),
        BinaryColor::On,
    );
    draw_text(
        frame,
        "TURN BROWSE",
        Point::new(20, PARAGRAPH_FOOTER_Y),
        ui_font_small(),
        BinaryColor::On,
        Alignment::Left,
    );
    draw_text_right(
        frame,
        "HOLD BACK",
        Point::new(324, PARAGRAPH_FOOTER_Y),
        ui_font_small(),
        BinaryColor::On,
    );
}

fn draw_paragraph_body(frame: &mut FrameBuffer, shell: &ParagraphNavigationShell) {
    let top_line = paragraph_top_line(shell);
    let bottom_primary = paragraph_bottom_primary_line(shell);
    let bottom_secondary = paragraph_bottom_secondary_line(shell);

    draw_text_pair_slot(frame, top_line.as_str(), None, paragraph_top_slot_spec());
    draw_paragraph_selected_card(frame, shell);
    draw_text_pair_slot(
        frame,
        bottom_primary.as_str(),
        bottom_secondary.as_ref().map(|line| line.as_str()),
        paragraph_bottom_slot_spec(),
    );
}

fn draw_paragraph_selected_card(frame: &mut FrameBuffer, shell: &ParagraphNavigationShell) {
    fill_rect(
        frame,
        PARAGRAPH_CARD_X,
        PARAGRAPH_CARD_Y,
        PARAGRAPH_CARD_WIDTH,
        PARAGRAPH_CARD_HEIGHT,
        BinaryColor::On,
    );
    stroke_rect(
        frame,
        PARAGRAPH_CARD_X,
        PARAGRAPH_CARD_Y,
        PARAGRAPH_CARD_WIDTH,
        PARAGRAPH_CARD_HEIGHT,
        BinaryColor::Off,
    );
    draw_text(
        frame,
        shell.selected_label.as_str(),
        Point::new(34, PARAGRAPH_CARD_LABEL_Y),
        ui_font_small(),
        BinaryColor::Off,
        Alignment::Left,
    );
    draw_text_ellipsized(
        frame,
        shell.selected_excerpt.as_str(),
        Point::new(34, PARAGRAPH_CARD_EXCERPT_Y),
        ui_font_body(),
        BinaryColor::Off,
        Alignment::Left,
        276,
    );
    draw_paragraph_selected_hint(frame);
}

fn draw_paragraph_selected_card_transition(
    frame: &mut FrameBuffer,
    from: &ParagraphNavigationShell,
    to: &ParagraphNavigationShell,
    offsets: SlotTransitionOffsets,
) {
    let clip = ClipRect {
        x: PARAGRAPH_CARD_X,
        y: PARAGRAPH_CARD_Y,
        width: PARAGRAPH_CARD_WIDTH,
        height: PARAGRAPH_CARD_HEIGHT,
    };

    fill_rect(
        frame,
        PARAGRAPH_CARD_X,
        PARAGRAPH_CARD_Y,
        PARAGRAPH_CARD_WIDTH,
        PARAGRAPH_CARD_HEIGHT,
        BinaryColor::On,
    );
    stroke_rect(
        frame,
        PARAGRAPH_CARD_X,
        PARAGRAPH_CARD_Y,
        PARAGRAPH_CARD_WIDTH,
        PARAGRAPH_CARD_HEIGHT,
        BinaryColor::Off,
    );
    draw_text_ellipsized_clipped(
        frame,
        from.selected_label.as_str(),
        ui_font_small(),
        ClippedTextSpec {
            position: Point::new(34, PARAGRAPH_CARD_LABEL_Y + offsets.outgoing),
            color: BinaryColor::Off,
            alignment: Alignment::Left,
            max_width_px: 180,
        },
        clip,
    );
    draw_text_ellipsized_clipped(
        frame,
        to.selected_label.as_str(),
        ui_font_small(),
        ClippedTextSpec {
            position: Point::new(34, PARAGRAPH_CARD_LABEL_Y + offsets.incoming),
            color: BinaryColor::Off,
            alignment: Alignment::Left,
            max_width_px: 180,
        },
        clip,
    );
    draw_text_ellipsized_clipped(
        frame,
        from.selected_excerpt.as_str(),
        ui_font_body(),
        ClippedTextSpec {
            position: Point::new(34, PARAGRAPH_CARD_EXCERPT_Y + offsets.outgoing),
            color: BinaryColor::Off,
            alignment: Alignment::Left,
            max_width_px: 276,
        },
        clip,
    );
    draw_text_ellipsized_clipped(
        frame,
        to.selected_excerpt.as_str(),
        ui_font_body(),
        ClippedTextSpec {
            position: Point::new(34, PARAGRAPH_CARD_EXCERPT_Y + offsets.incoming),
            color: BinaryColor::Off,
            alignment: Alignment::Left,
            max_width_px: 276,
        },
        clip,
    );
    draw_paragraph_selected_hint(frame);
}

fn draw_paragraph_selected_hint(frame: &mut FrameBuffer) {
    stroke_rect(
        frame,
        PARAGRAPH_CARD_HINT_X,
        PARAGRAPH_CARD_HINT_Y,
        PARAGRAPH_CARD_HINT_WIDTH,
        PARAGRAPH_CARD_HINT_HEIGHT,
        BinaryColor::Off,
    );
    draw_text(
        frame,
        "CLICK JUMP",
        Point::new(PARAGRAPH_CARD_HINT_X + 8, PARAGRAPH_CARD_HINT_Y + 4),
        ui_font_small(),
        BinaryColor::Off,
        Alignment::Left,
    );
}

fn paragraph_top_line(
    shell: &ParagraphNavigationShell,
) -> HeaplessString<NORMALIZED_TEXT_MAX_BYTES> {
    if shell.current_index <= 1 || shell.previous_top.is_empty() {
        let mut line = HeaplessString::new();
        let _ = line.push_str("START OF ARTICLE");
        return line;
    }

    paragraph_preview_line("PREV", shell.current_index - 1, shell.previous_top.as_str())
}

fn paragraph_bottom_primary_line(
    shell: &ParagraphNavigationShell,
) -> HeaplessString<NORMALIZED_TEXT_MAX_BYTES> {
    if shell.current_index >= shell.total || shell.previous_bottom.is_empty() {
        let mut line = HeaplessString::new();
        let _ = line.push_str("END OF ARTICLE");
        return line;
    }

    paragraph_preview_line(
        "NEXT",
        shell.current_index + 1,
        shell.previous_bottom.as_str(),
    )
}

fn paragraph_bottom_secondary_line(
    shell: &ParagraphNavigationShell,
) -> Option<HeaplessString<NORMALIZED_TEXT_MAX_BYTES>> {
    if shell.current_index.saturating_add(1) >= shell.total || shell.final_excerpt.is_empty() {
        return None;
    }

    Some(paragraph_preview_line(
        "THEN",
        shell.current_index + 2,
        shell.final_excerpt.as_str(),
    ))
}

fn paragraph_preview_line(
    prefix: &str,
    paragraph_index: u16,
    preview: &str,
) -> HeaplessString<NORMALIZED_TEXT_MAX_BYTES> {
    let mut line = HeaplessString::new();
    let _ = line.push_str(prefix);
    let _ = line.push(' ');
    push_padded_decimal(&mut line, paragraph_index);
    if !preview.is_empty() {
        let _ = line.push_str("  ");
        let _ = line.push_str(preview);
    }
    line
}

fn push_padded_decimal<const N: usize>(target: &mut HeaplessString<N>, value: u16) {
    let clamped = value.min(999);
    if clamped >= 100 {
        let _ = target.push((b'0' + ((clamped / 100) % 10) as u8) as char);
    } else {
        let _ = target.push('0');
    }
    let _ = target.push((b'0' + ((clamped / 10) % 10) as u8) as char);
    let _ = target.push((b'0' + (clamped % 10) as u8) as char);
}

fn draw_settings(frame: &mut FrameBuffer, shell: &SettingsShell, step: u8, total_steps: u8) {
    if let Some(topic_grid) = shell.topic_preferences {
        draw_topic_preferences(frame, &topic_grid);
        return;
    }

    draw_text(
        frame,
        shell.title,
        Point::new(20, 18),
        ui_font_body(),
        BinaryColor::On,
        Alignment::Left,
    );

    let selected_row = shell.rows.iter().position(|row| row.selected).unwrap_or(0);
    let band_y = settings_band_y(selected_row);
    draw_selection_band(frame, 20, band_y, 320, 34, step, total_steps);

    let mut index = 0;
    while index < shell.rows.len() {
        if index < 5 {
            let separator_y = settings_separator_y(index);
            fill_rect(frame, 20, separator_y, 320, 1, BinaryColor::On);
        }

        let label_y = settings_label_y(index);
        let is_selected = index == selected_row;
        let text_color = if is_selected {
            BinaryColor::Off
        } else {
            BinaryColor::On
        };

        draw_text(
            frame,
            shell.rows[index].label,
            Point::new(if is_selected { 32 } else { 20 }, label_y),
            ui_font_body(),
            text_color,
            Alignment::Left,
        );

        if let Some(value) = shell.rows[index].value {
            draw_text_right(
                frame,
                value,
                Point::new(320, label_y),
                ui_font_small(),
                text_color,
            );
        }

        if shell.rows[index].show_arrow {
            draw_text(
                frame,
                ">",
                Point::new(326, label_y),
                ui_font_body(),
                if is_selected {
                    BinaryColor::Off
                } else {
                    BinaryColor::On
                },
                Alignment::Left,
            );
        }

        index += 1;
    }

    if let (Some(title), Some(body)) = (shell.refresh_title, shell.refresh_body) {
        stroke_rect(frame, 58, 62, 268, 106, BinaryColor::On);
        draw_text(
            frame,
            title,
            Point::new(192, 82),
            ui_font_title(),
            BinaryColor::On,
            Alignment::Center,
        );
        draw_text(
            frame,
            body,
            Point::new(192, 118),
            ui_font_body(),
            BinaryColor::On,
            Alignment::Center,
        );
    }
}

fn draw_topic_preferences(frame: &mut FrameBuffer, grid: &TopicPreferenceGrid) {
    draw_text(
        frame,
        grid.title,
        Point::new(20, 18),
        ui_font_body(),
        BinaryColor::On,
        Alignment::Left,
    );

    let category_positions = [
        (20, 54, 112, 28, 28, 61),
        (20, 90, 112, 28, 28, 97),
        (20, 126, 112, 28, 28, 133),
        (20, 162, 112, 28, 28, 169),
    ];
    let mut category_index = 0;
    while category_index < grid.categories.len() {
        let (x, y, width, height, text_x, text_y) = category_positions[category_index];
        let category = grid.categories[category_index];

        if category.selected {
            draw_pill(frame, x, y, width, height, true);
            draw_text(
                frame,
                category.label,
                Point::new(text_x, text_y),
                ui_font_body(),
                BinaryColor::Off,
                Alignment::Left,
            );
        } else {
            draw_text(
                frame,
                category.label,
                Point::new(text_x, text_y),
                ui_font_body(),
                BinaryColor::On,
                Alignment::Left,
            );
        }

        category_index += 1;
    }

    fill_rect(frame, 148, 54, 1, 150, BinaryColor::On);

    let chip_layout = [
        (164, 54, 80, 24),
        (250, 54, 90, 24),
        (164, 86, 100, 24),
        (270, 86, 70, 24),
        (164, 118, 84, 24),
        (254, 118, 86, 24),
        (164, 150, 72, 24),
    ];

    let mut index = 0;
    while index < grid.chips.len() {
        let (x, y, w, h) = chip_layout[index];
        let chip = grid.chips[index];
        let selected =
            chip.selected || (index == 0 && matches!(grid.focus_region, TopicRegion::Categories));

        if selected {
            draw_pill(frame, x, y, w, h, true);
            draw_text(
                frame,
                chip.label,
                Point::new(x + 10, y + 7),
                ui_font_small(),
                BinaryColor::Off,
                Alignment::Left,
            );
        } else if chip.enabled {
            draw_pill(frame, x, y, w, h, false);
            draw_text(
                frame,
                chip.label,
                Point::new(x + 10, y + 7),
                ui_font_small(),
                BinaryColor::On,
                Alignment::Left,
            );
        } else {
            stroke_rect(frame, x, y, w, h, BinaryColor::On);
            draw_text(
                frame,
                chip.label,
                Point::new(x + 10, y + 7),
                ui_font_small(),
                BinaryColor::On,
                Alignment::Left,
            );
        }

        index += 1;
    }
}

fn draw_status_cluster(frame: &mut FrameBuffer, battery_percent: u8, wifi_online: bool) {
    draw_wifi_icon(frame, 298, 12, wifi_online);
    stroke_rect(frame, 319, 14, 18, 10, BinaryColor::On);
    fill_rect(
        frame,
        321,
        16,
        ((battery_percent.min(100) as u32 * 11) / 100) as i32,
        6,
        BinaryColor::On,
    );
    fill_rect(frame, 337, 17, 2, 4, BinaryColor::On);
    draw_text(
        frame,
        battery_label(battery_percent),
        Point::new(353, 13),
        ui_font_small(),
        BinaryColor::On,
        Alignment::Left,
    );
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct SlotTransitionOffsets {
    incoming: i32,
    outgoing: i32,
}

fn slot_transition_offsets(
    direction: MotionDirection,
    step: u8,
    total_steps: u8,
    amplitude: i32,
) -> SlotTransitionOffsets {
    let incoming = slide_offset(direction, step, total_steps, amplitude);
    let outgoing = match direction {
        MotionDirection::Forward => incoming - amplitude,
        MotionDirection::Backward => incoming + amplitude,
    };

    SlotTransitionOffsets { incoming, outgoing }
}

fn draw_text_pair_slot(
    frame: &mut FrameBuffer,
    primary: &str,
    secondary: Option<&str>,
    spec: TextPairSlotSpec,
) {
    draw_text_ellipsized(
        frame,
        primary,
        Point::new(spec.slot.text_x, spec.slot.meta_y),
        spec.font,
        spec.slot.color,
        Alignment::Left,
        spec.max_width_px,
    );

    if let Some(secondary) = secondary {
        draw_text_ellipsized(
            frame,
            secondary,
            Point::new(spec.slot.text_x, spec.slot.title_y),
            spec.font,
            spec.slot.color,
            Alignment::Left,
            spec.max_width_px,
        );
    }
}

fn draw_text_pair_slot_transition(
    frame: &mut FrameBuffer,
    from_primary: &str,
    from_secondary: Option<&str>,
    to_primary: &str,
    to_secondary: Option<&str>,
    spec: TextPairSlotSpec,
    offsets: SlotTransitionOffsets,
) {
    draw_text_pair_slot_clipped(frame, from_primary, from_secondary, spec, offsets.outgoing);
    draw_text_pair_slot_clipped(frame, to_primary, to_secondary, spec, offsets.incoming);
}

fn draw_text_pair_slot_clipped(
    frame: &mut FrameBuffer,
    primary: &str,
    secondary: Option<&str>,
    spec: TextPairSlotSpec,
    y_offset: i32,
) {
    draw_text_ellipsized_clipped(
        frame,
        primary,
        spec.font,
        ClippedTextSpec {
            position: Point::new(spec.slot.text_x, spec.slot.meta_y + y_offset),
            color: spec.slot.color,
            alignment: Alignment::Left,
            max_width_px: spec.max_width_px,
        },
        spec.slot.clip,
    );

    if let Some(secondary) = secondary {
        draw_text_ellipsized_clipped(
            frame,
            secondary,
            spec.font,
            ClippedTextSpec {
                position: Point::new(spec.slot.text_x, spec.slot.title_y + y_offset),
                color: spec.slot.color,
                alignment: Alignment::Left,
                max_width_px: spec.max_width_px,
            },
            spec.slot.clip,
        );
    }
}

fn draw_vertical_rail(frame: &mut FrameBuffer, text: &str, right_edge: i32, y: i32) {
    draw_text_right(
        frame,
        text,
        Point::new(right_edge, y),
        ui_font_title(),
        BinaryColor::On,
    );
}

fn draw_vertical_rail_scaled(
    frame: &mut FrameBuffer,
    text: &str,
    right_edge: i32,
    y: i32,
    scale: u32,
) {
    draw_text_right_scaled(
        frame,
        text,
        Point::new(right_edge, y),
        ui_font_title(),
        BinaryColor::On,
        scale,
    );
}

const fn paragraph_tick_offset(index: u8) -> i32 {
    match index {
        0 => 0,
        1 => 24,
        2 => 48,
        3 => 76,
        4 => 116,
        5 => 140,
        _ => 162,
    }
}

fn draw_paragraph_map_rail(frame: &mut FrameBuffer, selected_index: u8, total_ticks: u8) {
    draw_paragraph_map_rail_transition(frame, selected_index, selected_index, total_ticks, 1, 1);
}

fn draw_paragraph_map_rail_transition(
    frame: &mut FrameBuffer,
    from_index: u8,
    to_index: u8,
    total_ticks: u8,
    step: u8,
    total_steps: u8,
) {
    let x = 352;
    let y = 40;

    let mut index = 0;
    while index < total_ticks as i32 {
        let tick_y = y + paragraph_tick_offset(index as u8);
        let width = if index == from_index as i32 && index == to_index as i32 {
            12
        } else if index == from_index as i32 {
            lerp_u32(12, 4, step, total_steps) as i32
        } else if index == to_index as i32 {
            lerp_u32(4, 12, step, total_steps) as i32
        } else {
            4
        };
        let height = if index == from_index as i32 || index == to_index as i32 {
            28
        } else {
            14
        };
        let tick_x = x + (12 - width);

        fill_rect(frame, tick_x, tick_y, width, height, BinaryColor::On);

        index += 1;
    }
}

fn draw_refresh_pulse(frame: &mut FrameBuffer, step: u8, total_steps: u8) {
    let sweep_width = lerp_u32(24, 188, step, total_steps) as i32;
    fill_rect(frame, 106, 140, sweep_width, 6, BinaryColor::On);
}

fn draw_value_pulse(frame: &mut FrameBuffer, step: u8, total_steps: u8) {
    let width = lerp_u32(12, 82, step, total_steps) as i32;
    fill_rect(frame, 238, 48, width, 16, BinaryColor::On);
}

fn draw_row_flash(frame: &mut FrameBuffer, y: i32, height: i32, step: u8, total_steps: u8) {
    if step == total_steps / 2 {
        fill_rect(frame, 20, y, 320, height, BinaryColor::On);
    }
}

fn apply_theme(frame: &mut FrameBuffer, appearance: AppearanceMode) {
    if matches!(appearance, AppearanceMode::Dark) {
        invert_frame(frame);
    }
}

fn invert_frame(frame: &mut FrameBuffer) {
    frame.invert();
}

fn draw_wifi_icon(frame: &mut FrameBuffer, x: i32, y: i32, online: bool) {
    if online {
        fill_rect(frame, x + 7, y + 10, 2, 2, BinaryColor::On);
        fill_rect(frame, x + 5, y + 7, 6, 2, BinaryColor::On);
        fill_rect(frame, x + 3, y + 4, 10, 2, BinaryColor::On);
    } else {
        fill_rect(frame, x + 2, y + 2, 12, 2, BinaryColor::On);
        fill_rect(frame, x + 6, y + 4, 2, 10, BinaryColor::On);
    }
}

fn draw_back_chevron(frame: &mut FrameBuffer, x: i32, y: i32) {
    let mut offset = 0;
    while offset < 6 {
        set_pixel(frame, x + offset, y + 5 - offset);
        set_pixel(frame, x + offset, y + 5 + offset);
        offset += 1;
    }
}

fn draw_live_dot(frame: &mut FrameBuffer, x: i32, y: i32, visible: bool) {
    if visible {
        fill_rect(frame, x, y, 6, 6, BinaryColor::On);
    }
}

fn draw_selection_band(
    frame: &mut FrameBuffer,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    step: u8,
    total_steps: u8,
) {
    let revealed = lerp_u32(0, width as u32, step, total_steps) as i32;
    let mut row = 0;
    while row < height {
        let diagonal_right = x + width - ((14 * row) / height.max(1));
        let visible_right = (x + revealed).min(diagonal_right);
        frame.fill_span(x, y + row, visible_right - x, true);
        row += 1;
    }
}

fn draw_pill(frame: &mut FrameBuffer, x: i32, y: i32, width: i32, height: i32, filled: bool) {
    if filled {
        fill_rect(frame, x, y, width, height, BinaryColor::On);
    } else {
        stroke_rect(frame, x, y, width, height, BinaryColor::On);
    }
}

fn fill_rect(frame: &mut FrameBuffer, x: i32, y: i32, width: i32, height: i32, color: BinaryColor) {
    frame.fill_rect(x, y, width, height, color.is_on());
}

fn fill_rect_clipped(
    frame: &mut FrameBuffer,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    color: BinaryColor,
    clip: Option<ClipRect>,
) {
    if width <= 0 || height <= 0 {
        return;
    }

    let Some(clip) = clip else {
        fill_rect(frame, x, y, width, height, color);
        return;
    };

    let left = x.max(clip.x);
    let top = y.max(clip.y);
    let right = (x + width).min(clip.x + clip.width);
    let bottom = (y + height).min(clip.y + clip.height);

    if right <= left || bottom <= top {
        return;
    }

    fill_rect(frame, left, top, right - left, bottom - top, color);
}

fn stroke_rect(
    frame: &mut FrameBuffer,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    color: BinaryColor,
) {
    if width <= 0 || height <= 0 {
        return;
    }

    fill_rect(frame, x, y, width, 1, color);
    fill_rect(frame, x, y + height - 1, width, 1, color);
    fill_rect(frame, x, y, 1, height, color);
    fill_rect(frame, x + width - 1, y, 1, height, color);
}

fn draw_text(
    frame: &mut FrameBuffer,
    text: &str,
    position: Point,
    font: &embedded_graphics::mono_font::MonoFont<'static>,
    color: BinaryColor,
    alignment: Alignment,
) {
    let normalized = normalized_text(text);
    let style = MonoTextStyleBuilder::new()
        .font(font)
        .text_color(color)
        .build();
    let text_style = TextStyleBuilder::new()
        .alignment(alignment)
        .baseline(Baseline::Top)
        .build();

    Text::with_text_style(normalized.as_str(), position, style, text_style)
        .draw(frame)
        .ok();
}

fn draw_text_ellipsized(
    frame: &mut FrameBuffer,
    text: &str,
    position: Point,
    font: &embedded_graphics::mono_font::MonoFont<'static>,
    color: BinaryColor,
    alignment: Alignment,
    max_width_px: i32,
) {
    let clipped = ellipsized_text(text, font, 1, max_width_px);
    draw_text(frame, clipped.as_str(), position, font, color, alignment);
}

fn draw_text_ellipsized_clipped(
    frame: &mut FrameBuffer,
    text: &str,
    font: &embedded_graphics::mono_font::MonoFont<'static>,
    spec: ClippedTextSpec,
    clip: ClipRect,
) {
    let clipped = ellipsized_text(text, font, 1, spec.max_width_px);
    let style = MonoTextStyleBuilder::new()
        .font(font)
        .text_color(spec.color)
        .build();
    let text_style = TextStyleBuilder::new()
        .alignment(spec.alignment)
        .baseline(Baseline::Top)
        .build();
    let mut clipped_frame = ClippedFrameBuffer::new(frame, clip);

    Text::with_text_style(clipped.as_str(), spec.position, style, text_style)
        .draw(&mut clipped_frame)
        .ok();
}

fn draw_text_scaled(
    frame: &mut FrameBuffer,
    text: &str,
    position: Point,
    font: &embedded_graphics::mono_font::MonoFont<'static>,
    color: BinaryColor,
    alignment: Alignment,
    scale: u32,
) {
    let normalized = normalized_text(text);
    let style = MonoTextStyleBuilder::new()
        .font(font)
        .text_color(color)
        .build();
    let text_style = TextStyleBuilder::new()
        .alignment(alignment)
        .baseline(Baseline::Top)
        .build();
    let logical_position = logical_text_position(position, scale);
    let mut scaled_frame = ScaledFrameBuffer::new(frame, scale);

    Text::with_text_style(normalized.as_str(), logical_position, style, text_style)
        .draw(&mut scaled_frame)
        .ok();
}

fn draw_text_right(
    frame: &mut FrameBuffer,
    text: &str,
    position: Point,
    font: &embedded_graphics::mono_font::MonoFont<'static>,
    color: BinaryColor,
) {
    draw_text(frame, text, position, font, color, Alignment::Right);
}

fn draw_text_right_scaled(
    frame: &mut FrameBuffer,
    text: &str,
    position: Point,
    font: &embedded_graphics::mono_font::MonoFont<'static>,
    color: BinaryColor,
    scale: u32,
) {
    draw_text_scaled(frame, text, position, font, color, Alignment::Right, scale);
}

fn normalized_text(text: &str) -> HeaplessString<NORMALIZED_TEXT_MAX_BYTES> {
    let mut normalized = HeaplessString::new();

    for ch in text.chars() {
        if normalized.push(normalize_display_char(ch)).is_err() {
            break;
        }
    }

    normalized
}

fn ellipsized_text(
    text: &str,
    font: &embedded_graphics::mono_font::MonoFont<'static>,
    scale: u32,
    max_width_px: i32,
) -> HeaplessString<NORMALIZED_TEXT_MAX_BYTES> {
    let mut normalized = normalized_text(text);
    let cell_width = (font.character_size.width.saturating_mul(scale.max(1))) as i32;
    if cell_width <= 0 || max_width_px <= 0 {
        return HeaplessString::new();
    }

    let max_chars = (max_width_px / cell_width).max(0) as usize;
    let char_count = normalized.chars().count();
    if char_count <= max_chars {
        return normalized;
    }

    if max_chars == 0 {
        return HeaplessString::new();
    }

    if max_chars <= ELLIPSIS.len() {
        let mut dots = HeaplessString::new();
        for _ in 0..max_chars {
            let _ = dots.push('.');
        }
        return dots;
    }

    let keep_chars = max_chars - ELLIPSIS.len();
    let mut clipped = HeaplessString::new();
    for ch in normalized.chars().take(keep_chars) {
        if clipped.push(ch).is_err() {
            return clipped;
        }
    }
    let _ = clipped.push_str(ELLIPSIS);
    normalized.clear();
    clipped
}

fn normalize_display_char(ch: char) -> char {
    match ch {
        '’' | '‘' => '\'',
        '“' | '”' => '"',
        '–' | '—' => '-',
        '\u{00A0}' => ' ',
        _ => ch,
    }
}

fn logical_text_position(position: Point, scale: u32) -> Point {
    let scale = scale as i32;
    debug_assert_eq!(position.x % scale, 0);
    debug_assert_eq!(position.y % scale, 0);

    Point::new(position.x / scale, position.y / scale)
}

struct ScaledFrameBuffer<'a> {
    frame: &'a mut FrameBuffer,
    scale: u32,
}

impl<'a> ScaledFrameBuffer<'a> {
    fn new(frame: &'a mut FrameBuffer, scale: u32) -> Self {
        Self { frame, scale }
    }
}

impl DrawTarget for ScaledFrameBuffer<'_> {
    type Color = BinaryColor;
    type Error = Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        let scale = self.scale as i32;

        for Pixel(point, color) in pixels {
            if point.x < 0 || point.y < 0 {
                continue;
            }

            let physical_x = point.x * scale;
            let physical_y = point.y * scale;

            let mut dy = 0;
            while dy < scale {
                let mut dx = 0;
                while dx < scale {
                    let _ = self.frame.set_pixel(
                        (physical_x + dx) as usize,
                        (physical_y + dy) as usize,
                        color.is_on(),
                    );
                    dx += 1;
                }
                dy += 1;
            }
        }

        Ok(())
    }
}

impl OriginDimensions for ScaledFrameBuffer<'_> {
    fn size(&self) -> Size {
        let physical = self.frame.size();
        Size::new(
            physical.width / self.scale.max(1),
            physical.height / self.scale.max(1),
        )
    }
}

struct ClippedFrameBuffer<'a> {
    frame: &'a mut FrameBuffer,
    clip_x: i32,
    clip_y: i32,
    clip_width: i32,
    clip_height: i32,
}

impl<'a> ClippedFrameBuffer<'a> {
    fn new(frame: &'a mut FrameBuffer, clip: ClipRect) -> Self {
        Self {
            frame,
            clip_x: clip.x,
            clip_y: clip.y,
            clip_width: clip.width,
            clip_height: clip.height,
        }
    }

    fn contains(&self, point: Point) -> bool {
        point.x >= self.clip_x
            && point.y >= self.clip_y
            && point.x < self.clip_x + self.clip_width
            && point.y < self.clip_y + self.clip_height
    }
}

impl DrawTarget for ClippedFrameBuffer<'_> {
    type Color = BinaryColor;
    type Error = Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(point, color) in pixels {
            if point.x < 0 || point.y < 0 || !self.contains(point) {
                continue;
            }

            let _ = self
                .frame
                .set_pixel(point.x as usize, point.y as usize, color.is_on());
        }

        Ok(())
    }
}

impl OriginDimensions for ClippedFrameBuffer<'_> {
    fn size(&self) -> Size {
        self.frame.size()
    }
}

fn battery_label(percent: u8) -> &'static str {
    match percent {
        0..=9 => "09%",
        10..=19 => "18%",
        20..=29 => "24%",
        30..=39 => "32%",
        40..=49 => "48%",
        50..=59 => "56%",
        60..=69 => "64%",
        70..=79 => "74%",
        80..=89 => "82%",
        90..=99 => "96%",
        _ => "100%",
    }
}

fn wpm_label(wpm: u16) -> &'static str {
    match wpm {
        200 => "200 WPM",
        220 => "220 WPM",
        240 => "240 WPM",
        260 => "260 WPM",
        280 => "280 WPM",
        300 => "300 WPM",
        320 => "320 WPM",
        340 => "340 WPM",
        360 => "360 WPM",
        _ => "260 WPM",
    }
}

fn reader_preview_max_width_px(wpm: u16) -> i32 {
    let wpm_width = mono_text_width_px(wpm_label(wpm), ui_font_body(), 1);
    (READER_TEXT_RIGHT_X - READER_TEXT_LEFT_X - wpm_width - READER_FOOTER_WPM_GAP_PX).max(0)
}

fn mono_text_width_px(
    text: &str,
    font: &embedded_graphics::mono_font::MonoFont<'static>,
    scale: u32,
) -> i32 {
    let char_width = (font.character_size.width.saturating_mul(scale.max(1))) as i32;
    normalized_text(text).chars().count() as i32 * char_width
}

fn slide_offset(direction: MotionDirection, step: u8, total_steps: u8, amplitude: i32) -> i32 {
    let remaining = total_steps.saturating_sub(step) as i32;
    let total = total_steps.max(1) as i32;
    let offset = (amplitude * remaining) / total;

    match direction {
        MotionDirection::Forward => offset,
        MotionDirection::Backward => -offset,
    }
}

const fn lerp_u32(start: u32, end: u32, step: u8, total_steps: u8) -> u32 {
    if total_steps == 0 {
        return end;
    }

    start + (((end - start) * step as u32) / total_steps as u32)
}

const fn settings_band_y(selected_row: usize) -> i32 {
    match selected_row {
        0 => 42,
        1 => 78,
        2 => 114,
        3 => 150,
        4 => 186,
        5 => 204,
        _ => 42,
    }
}

const fn settings_label_y(selected_row: usize) -> i32 {
    match selected_row {
        0 => 51,
        1 => 87,
        2 => 123,
        3 => 159,
        4 => 195,
        5 => 213,
        _ => 51,
    }
}

const fn settings_separator_y(index: usize) -> i32 {
    match index {
        0 => 80,
        1 => 116,
        2 => 152,
        3 => 188,
        4 => 224,
        _ => 224,
    }
}

fn set_pixel(frame: &mut FrameBuffer, x: i32, y: i32) {
    set_pixel_color(frame, x, y, BinaryColor::On);
}

fn set_pixel_color(frame: &mut FrameBuffer, x: i32, y: i32, color: BinaryColor) {
    if x < 0 || y < 0 {
        return;
    }

    let _ = frame.set_pixel(x as usize, y as usize, color.is_on());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::diff_dirty_rows;
    use app_runtime::components::{
        ContentListShell, ContentRow, DashboardItem, HelpHint, ParagraphMapRail, PauseModalRow,
        SelectionBand, StatusCluster, SyncIndicator, VerticalRail,
    };
    use domain::text::InlineText;

    fn make_reader_shell(progress_width: u16) -> ReaderShell {
        make_reader_shell_with_modal(progress_width, None)
    }

    fn make_reader_shell_with_modal(
        progress_width: u16,
        pause_modal: Option<PauseModal>,
    ) -> ReaderShell {
        ReaderShell {
            appearance: AppearanceMode::Light,
            stage: app_runtime::components::RsvpStage {
                title: InlineText::from_slice("TITLE"),
                wpm: 260,
                left_word: InlineText::from_slice("LEFT"),
                right_word: InlineText::from_slice("RIGHT"),
                preview: InlineText::from_slice("preview"),
                font: StageFont::Large,
                progress_width,
            },
            badge: None,
            pause_modal,
        }
    }

    fn make_pause_modal() -> PauseModal {
        PauseModal {
            title: "PAUSED",
            rows: [
                PauseModalRow {
                    label: "RESUME",
                    action: "CLICK",
                },
                PauseModalRow {
                    label: "PARAGRAPH",
                    action: "PRESS",
                },
                PauseModalRow {
                    label: "SPEED",
                    action: "TURN",
                },
            ],
        }
    }

    fn make_paragraph_shell(
        current_index: u16,
        total: u16,
        selected_index: u8,
        top: &'static str,
        label: &'static str,
        excerpt: &'static str,
        bottom: &'static str,
        final_excerpt: &'static str,
    ) -> ParagraphNavigationShell {
        ParagraphNavigationShell {
            appearance: AppearanceMode::Light,
            title: InlineText::from_slice("PARAGRAPHS"),
            current_index,
            total,
            counter: InlineText::from_slice("4 / 12"),
            previous_top: InlineText::from_slice(top),
            selected_label: InlineText::from_slice(label),
            selected_excerpt: InlineText::from_slice(excerpt),
            previous_bottom: InlineText::from_slice(bottom),
            final_excerpt: InlineText::from_slice(final_excerpt),
            rail: ParagraphMapRail {
                selected_index,
                total_ticks: 7,
            },
        }
    }

    fn make_dashboard_shell(spinner_phase: u8) -> DashboardShell {
        make_dashboard_shell_with_labels(spinner_phase, ["INBOX", "SAVED", "RECS"])
    }

    fn make_dashboard_shell_with_labels(
        spinner_phase: u8,
        labels: [&'static str; 3],
    ) -> DashboardShell {
        DashboardShell {
            appearance: AppearanceMode::Light,
            status: StatusCluster {
                battery_percent: 64,
                wifi_online: true,
            },
            sync_indicator: Some(SyncIndicator {
                label: "SYNC",
                spinner_phase,
            }),
            rail: VerticalRail { text: "HOME" },
            items: [
                DashboardItem {
                    label: labels[0],
                    live_dot: true,
                    selected: false,
                },
                DashboardItem {
                    label: labels[1],
                    live_dot: false,
                    selected: true,
                },
                DashboardItem {
                    label: labels[2],
                    live_dot: true,
                    selected: false,
                },
            ],
            band: SelectionBand { y: 82, height: 60 },
        }
    }

    fn make_collection_shell(rows: [(&str, &str); 3]) -> ContentListShell {
        make_collection_shell_with_spinner(rows, None)
    }

    fn make_collection_shell_with_spinner(
        rows: [(&str, &str); 3],
        selected_spinner_phase: Option<u8>,
    ) -> ContentListShell {
        ContentListShell {
            appearance: AppearanceMode::Light,
            status: StatusCluster {
                battery_percent: 64,
                wifi_online: true,
            },
            rail: VerticalRail { text: "SAVED" },
            large_rail: true,
            rows: [
                ContentRow {
                    meta: InlineText::from_slice(rows[0].0),
                    title: InlineText::from_slice(rows[0].1),
                    loading_phase: None,
                    selected: false,
                },
                ContentRow {
                    meta: InlineText::from_slice(rows[1].0),
                    title: InlineText::from_slice(rows[1].1),
                    loading_phase: selected_spinner_phase,
                    selected: true,
                },
                ContentRow {
                    meta: InlineText::from_slice(rows[2].0),
                    title: InlineText::from_slice(rows[2].1),
                    loading_phase: None,
                    selected: false,
                },
            ],
            band: SelectionBand { y: 106, height: 68 },
            help: HelpHint { text: "BACK" },
        }
    }

    #[test]
    fn reader_progress_only_dirties_progress_rows() {
        let mut committed = FrameBuffer::new();
        let mut working = FrameBuffer::new();

        draw_prepared_screen(
            &mut committed,
            &PreparedScreen::Reader(make_reader_shell(0)),
        );
        draw_prepared_screen(&mut working, &PreparedScreen::Reader(make_reader_shell(80)));

        let dirty = diff_dirty_rows(&committed, &working);

        assert!(!dirty.is_empty());
        for row in dirty.iter() {
            assert!((232..240).contains(&row), "unexpected dirty row {row}");
        }
    }

    #[test]
    fn reader_footer_reserves_gap_for_wpm_label() {
        let wpm_width = mono_text_width_px(wpm_label(300), ui_font_body(), 1);

        assert_eq!(READER_TITLE_MAX_WIDTH_PX, 360);
        assert_eq!(reader_preview_max_width_px(300), 288);
        assert_eq!(
            reader_preview_max_width_px(300) + READER_FOOTER_WPM_GAP_PX + wpm_width,
            READER_TITLE_MAX_WIDTH_PX
        );
    }

    #[test]
    fn pause_modal_reveal_frames_stay_within_modal_rows() {
        let modal = make_pause_modal();
        let from = make_reader_shell_with_modal(32, None);
        let to = make_reader_shell_with_modal(32, Some(modal));
        let step_1 = AnimationPlayback {
            from: PreparedScreen::Reader(from),
            to: PreparedScreen::Reader(to),
            screen: Screen::Reader,
            plan: TransitionPlan::new(AnimationDescriptor::ModalReveal, 3, 55),
            step: 1,
        };
        let step_2 = step_1.advance();
        let step_3 = step_2.advance();
        let mut committed = FrameBuffer::new();
        let mut frame_1 = FrameBuffer::new();
        let mut frame_2 = FrameBuffer::new();
        let mut frame_3 = FrameBuffer::new();

        draw_prepared_screen(&mut committed, &PreparedScreen::Reader(from));
        draw_transition_frame(&mut frame_1, &step_1);
        draw_transition_frame(&mut frame_2, &step_2);
        draw_transition_frame(&mut frame_3, &step_3);

        for dirty in [
            diff_dirty_rows(&committed, &frame_1),
            diff_dirty_rows(&frame_1, &frame_2),
            diff_dirty_rows(&frame_2, &frame_3),
        ] {
            assert!(dirty.count() <= 166, "dirty rows={}", dirty.count());
            for row in dirty.iter() {
                assert!((35..202).contains(&row), "unexpected dirty row {row}");
            }
        }
    }

    #[test]
    fn pause_modal_hide_frames_stay_within_modal_rows() {
        let modal = make_pause_modal();
        let from = make_reader_shell_with_modal(32, Some(modal));
        let to = make_reader_shell_with_modal(32, None);
        let step_1 = AnimationPlayback {
            from: PreparedScreen::Reader(from),
            to: PreparedScreen::Reader(to),
            screen: Screen::Reader,
            plan: TransitionPlan::new(AnimationDescriptor::ModalHide, 3, 55),
            step: 1,
        };
        let step_2 = step_1.advance();
        let step_3 = step_2.advance();
        let mut committed = FrameBuffer::new();
        let mut frame_1 = FrameBuffer::new();
        let mut frame_2 = FrameBuffer::new();
        let mut frame_3 = FrameBuffer::new();

        draw_prepared_screen(&mut committed, &PreparedScreen::Reader(from));
        draw_transition_frame(&mut frame_1, &step_1);
        draw_transition_frame(&mut frame_2, &step_2);
        draw_transition_frame(&mut frame_3, &step_3);

        for dirty in [
            diff_dirty_rows(&committed, &frame_1),
            diff_dirty_rows(&frame_1, &frame_2),
            diff_dirty_rows(&frame_2, &frame_3),
        ] {
            assert!(dirty.count() <= 166, "dirty rows={}", dirty.count());
            for row in dirty.iter() {
                assert!((35..202).contains(&row), "unexpected dirty row {row}");
            }
        }
    }

    #[test]
    fn dashboard_spinner_dirty_rows_stay_localized() {
        let mut committed = FrameBuffer::new();
        let mut working = FrameBuffer::new();

        draw_prepared_screen(
            &mut committed,
            &PreparedScreen::Dashboard(make_dashboard_shell(0)),
        );
        draw_prepared_screen(
            &mut working,
            &PreparedScreen::Dashboard(make_dashboard_shell(1)),
        );

        let dirty = diff_dirty_rows(&committed, &working);

        assert!(dirty.count() <= 12);
        for row in dirty.iter() {
            assert!((220..230).contains(&row), "unexpected dirty row {row}");
        }
    }

    #[test]
    fn dashboard_transition_frames_stay_within_visible_rows() {
        let from = make_dashboard_shell_with_labels(0, ["INBOX", "SAVED", "RECS"]);
        let to = make_dashboard_shell_with_labels(0, ["SAVED", "RECS", "SETTINGS"]);
        let step_1 = AnimationPlayback {
            from: PreparedScreen::Dashboard(from),
            to: PreparedScreen::Dashboard(to),
            screen: Screen::Dashboard,
            plan: TransitionPlan::new(
                AnimationDescriptor::BandReveal(MotionDirection::Forward),
                3,
                50,
            ),
            step: 1,
        };
        let step_2 = step_1.advance();
        let step_3 = step_2.advance();
        let mut committed = FrameBuffer::new();
        let mut frame_1 = FrameBuffer::new();
        let mut frame_2 = FrameBuffer::new();
        let mut frame_3 = FrameBuffer::new();

        draw_prepared_screen(&mut committed, &PreparedScreen::Dashboard(from));
        draw_transition_frame(&mut frame_1, &step_1);
        draw_transition_frame(&mut frame_2, &step_2);
        draw_transition_frame(&mut frame_3, &step_3);

        for dirty in [
            diff_dirty_rows(&committed, &frame_1),
            diff_dirty_rows(&frame_1, &frame_2),
            diff_dirty_rows(&frame_2, &frame_3),
        ] {
            assert!(dirty.count() <= 120, "dirty rows={}", dirty.count());
            for row in dirty.iter() {
                assert!((42..191).contains(&row), "unexpected dirty row {row}");
            }
        }
    }

    #[test]
    fn collection_row_spinner_dirty_rows_stay_localized() {
        let rows = [
            ("SOURCE", "Previous item"),
            ("SOURCE", "Fetching item"),
            ("SOURCE", "Next item"),
        ];
        let mut committed = FrameBuffer::new();
        let mut working = FrameBuffer::new();

        draw_prepared_screen(
            &mut committed,
            &PreparedScreen::Collection(make_collection_shell_with_spinner(rows, Some(0))),
        );
        draw_prepared_screen(
            &mut working,
            &PreparedScreen::Collection(make_collection_shell_with_spinner(rows, Some(1))),
        );

        let dirty = diff_dirty_rows(&committed, &working);

        assert!(dirty.count() <= 16);
        for row in dirty.iter() {
            assert!((122..136).contains(&row), "unexpected dirty row {row}");
        }
    }

    #[test]
    fn list_step_transition_start_stays_within_visible_rows() {
        let from = make_collection_shell([("meta-a", "A"), ("meta-b", "B"), ("meta-c", "C")]);
        let to = make_collection_shell([("meta-b", "B"), ("meta-c", "C"), ("meta-d", "D")]);
        let step_1 = AnimationPlayback {
            from: PreparedScreen::Collection(from),
            to: PreparedScreen::Collection(to),
            screen: Screen::Saved,
            plan: TransitionPlan::new(
                AnimationDescriptor::ListStep(MotionDirection::Forward),
                3,
                55,
            ),
            step: 1,
        };
        let mut committed = FrameBuffer::new();
        let mut frame_1 = FrameBuffer::new();

        draw_prepared_screen(&mut committed, &PreparedScreen::Collection(from));
        draw_transition_frame(&mut frame_1, &step_1);

        let dirty = diff_dirty_rows(&committed, &frame_1);

        assert!(dirty.count() <= 120, "dirty rows={}", dirty.count());
        for row in dirty.iter() {
            assert!((42..221).contains(&row), "unexpected dirty row {row}");
        }
    }

    #[test]
    fn list_step_intermediate_frames_stay_within_visible_rows() {
        let from = make_collection_shell([("meta-a", "A"), ("meta-b", "B"), ("meta-c", "C")]);
        let to = make_collection_shell([("meta-b", "B"), ("meta-c", "C"), ("meta-d", "D")]);
        let step_1 = AnimationPlayback {
            from: PreparedScreen::Collection(from),
            to: PreparedScreen::Collection(to),
            screen: Screen::Saved,
            plan: TransitionPlan::new(
                AnimationDescriptor::ListStep(MotionDirection::Forward),
                3,
                55,
            ),
            step: 1,
        };
        let step_2 = step_1.advance();
        let mut frame_1 = FrameBuffer::new();
        let mut frame_2 = FrameBuffer::new();

        draw_transition_frame(&mut frame_1, &step_1);
        draw_transition_frame(&mut frame_2, &step_2);

        let dirty = diff_dirty_rows(&frame_1, &frame_2);

        assert!(dirty.count() <= 120, "dirty rows={}", dirty.count());
        for row in dirty.iter() {
            assert!((42..221).contains(&row), "unexpected dirty row {row}");
        }
    }

    #[test]
    fn list_step_final_commit_stays_within_visible_rows() {
        let from = make_collection_shell([("meta-a", "A"), ("meta-b", "B"), ("meta-c", "C")]);
        let to = make_collection_shell([("meta-b", "B"), ("meta-c", "C"), ("meta-d", "D")]);
        let step_1 = AnimationPlayback {
            from: PreparedScreen::Collection(from),
            to: PreparedScreen::Collection(to),
            screen: Screen::Saved,
            plan: TransitionPlan::new(
                AnimationDescriptor::ListStep(MotionDirection::Forward),
                3,
                55,
            ),
            step: 1,
        };
        let step_2 = step_1.advance();
        let step_3 = step_2.advance();
        let mut frame_2 = FrameBuffer::new();
        let mut frame_3 = FrameBuffer::new();

        draw_transition_frame(&mut frame_2, &step_2);
        draw_transition_frame(&mut frame_3, &step_3);

        let dirty = diff_dirty_rows(&frame_2, &frame_3);

        assert!(dirty.count() <= 120, "dirty rows={}", dirty.count());
        for row in dirty.iter() {
            assert!((42..221).contains(&row), "unexpected dirty row {row}");
        }
    }

    #[test]
    fn paragraph_transition_frames_stay_within_selector_rows() {
        let from = make_paragraph_shell(
            4,
            12,
            2,
            "A previous top preview",
            "P4",
            "Selected paragraph excerpt",
            "A previous bottom preview",
            "Final excerpt for paragraph",
        );
        let to = make_paragraph_shell(
            5,
            12,
            3,
            "Selected paragraph excerpt",
            "P5",
            "Next paragraph excerpt",
            "Final excerpt for paragraph",
            "Another final paragraph",
        );
        let step_1 = AnimationPlayback {
            from: PreparedScreen::ParagraphNavigation(from),
            to: PreparedScreen::ParagraphNavigation(to),
            screen: Screen::ParagraphNavigation,
            plan: TransitionPlan::new(
                AnimationDescriptor::ParagraphTickMove(MotionDirection::Forward),
                3,
                55,
            ),
            step: 1,
        };
        let step_2 = step_1.advance();
        let step_3 = step_2.advance();
        let mut committed = FrameBuffer::new();
        let mut frame_1 = FrameBuffer::new();
        let mut frame_2 = FrameBuffer::new();
        let mut frame_3 = FrameBuffer::new();

        draw_prepared_screen(&mut committed, &PreparedScreen::ParagraphNavigation(from));
        draw_transition_frame(&mut frame_1, &step_1);
        draw_transition_frame(&mut frame_2, &step_2);
        draw_transition_frame(&mut frame_3, &step_3);

        for dirty in [
            diff_dirty_rows(&committed, &frame_1),
            diff_dirty_rows(&frame_1, &frame_2),
            diff_dirty_rows(&frame_2, &frame_3),
        ] {
            assert!(dirty.count() <= 200, "dirty rows={}", dirty.count());
            for row in dirty.iter() {
                assert!((40..240).contains(&row), "unexpected dirty row {row}");
            }
        }
    }
}
