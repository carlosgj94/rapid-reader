use app_runtime::{
    AnimationDescriptor, MotionDirection, PreparedScreen, Screen, ScreenUpdate, TransitionPlan,
    components::{
        ContentListShell, DashboardShell, ParagraphNavigationShell, PauseModal, ReaderShell,
        SettingsShell, TopicPreferenceGrid,
    },
};
use domain::formatter::StageFont;
use domain::settings::AppearanceMode;
use domain::ui::TopicRegion;
use embedded_graphics::{
    mono_font::{
        MonoFont, MonoTextStyleBuilder,
        ascii::{FONT_6X10, FONT_8X13, FONT_10X20},
    },
    pixelcolor::BinaryColor,
    prelude::*,
    primitives::{PrimitiveStyle, PrimitiveStyleBuilder, Rectangle},
    text::{Alignment, Baseline, Text, TextStyleBuilder},
};
use ls027b7dh01::FrameBuffer;

pub const UI_TICK_MS: u64 = 160;
pub const MAINTENANCE_REFRESH_TICKS: u8 = 6;
const RSVP_STAGE_CENTER_X: i32 = 170;
const RSVP_STAGE_LEFT_ANCHOR_X: i32 = 169;
const RSVP_STAGE_RIGHT_ANCHOR_X: i32 = 173;

fn ui_font_small() -> &'static MonoFont<'static> {
    &FONT_6X10
}

fn ui_font_body() -> &'static MonoFont<'static> {
    &FONT_8X13
}

fn ui_font_title() -> &'static MonoFont<'static> {
    &FONT_10X20
}

fn stage_font_spec(font: StageFont) -> (&'static MonoFont<'static>, i32) {
    match font {
        StageFont::Large | StageFont::Medium => (ui_font_title(), 88),
        StageFont::Small => (ui_font_body(), 96),
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
        AnimationDescriptor::BandReveal(_) => match playback.to {
            PreparedScreen::Dashboard(shell) => {
                draw_dashboard(frame, &shell, playback.step, playback.plan.steps)
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
            if let PreparedScreen::Collection(shell) = playback.to {
                let slide = slide_offset(direction, playback.step, playback.plan.steps, 18);
                draw_collection(frame, &shell, playback.step, playback.plan.steps, slide);
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
        AnimationDescriptor::ModalReveal | AnimationDescriptor::ModalHide => {
            if let PreparedScreen::Reader(shell) = playback.to {
                draw_reader(frame, &shell, playback.step, playback.plan.steps);
            } else {
                draw_prepared_screen(frame, &playback.to);
            }
        }
        AnimationDescriptor::ParagraphTickMove(direction) => {
            if let PreparedScreen::ParagraphNavigation(shell) = playback.to {
                draw_paragraph_navigation(frame, &shell, playback.step, playback.plan.steps);
                draw_tick_motion_accent(
                    frame,
                    &shell.rail,
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
    draw_status_cluster(
        frame,
        shell.status.battery_percent,
        shell.status.wifi_online,
    );
    draw_vertical_rail(frame, shell.rail.text, 398, 26);
    draw_text(
        frame,
        shell.items[0].label,
        Point::new(20, 54),
        ui_font_title(),
        BinaryColor::On,
        Alignment::Left,
    );
    draw_live_dot(frame, 100, 61, shell.items[0].live_dot);
    draw_selection_band(
        frame,
        16,
        shell.band.y,
        320,
        shell.band.height as i32,
        step,
        total_steps,
    );
    draw_text(
        frame,
        shell.items[1].label,
        Point::new(30, 92),
        ui_font_title(),
        BinaryColor::Off,
        Alignment::Left,
    );

    if step >= total_steps {
        fill_rect(frame, 303, 105, 28, 5, BinaryColor::Off);
        fill_rect(frame, 307, 116, 22, 5, BinaryColor::Off);
    }

    draw_text(
        frame,
        shell.items[2].label,
        Point::new(20, 161),
        ui_font_title(),
        BinaryColor::On,
        Alignment::Left,
    );
    draw_live_dot(frame, 100, 168, shell.items[2].live_dot);

    if let Some(sync_indicator) = shell.sync_indicator {
        draw_dashboard_sync_indicator(frame, sync_indicator.label, sync_indicator.spinner_phase);
    }
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

fn sync_spinner_frame(spinner_phase: u8) -> &'static str {
    match spinner_phase % 4 {
        0 => "|",
        1 => "/",
        2 => "-",
        _ => "\\",
    }
}

fn draw_collection(
    frame: &mut FrameBuffer,
    shell: &ContentListShell,
    step: u8,
    total_steps: u8,
    slide_offset: i32,
) {
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
    draw_vertical_rail(frame, shell.rail.text, 398, 26);

    draw_text(
        frame,
        shell.rows[0].meta.as_str(),
        Point::new(20, 40 + slide_offset),
        ui_font_body(),
        BinaryColor::On,
        Alignment::Left,
    );
    draw_text(
        frame,
        shell.rows[0].title.as_str(),
        Point::new(20, 58 + slide_offset),
        ui_font_body(),
        BinaryColor::On,
        Alignment::Left,
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
    draw_text(
        frame,
        shell.rows[1].meta.as_str(),
        Point::new(28, 110 + slide_offset),
        ui_font_body(),
        BinaryColor::Off,
        Alignment::Left,
    );
    draw_text(
        frame,
        shell.rows[1].title.as_str(),
        Point::new(28, 127 + slide_offset),
        ui_font_body(),
        BinaryColor::Off,
        Alignment::Left,
    );

    if step >= total_steps {
        fill_rect(frame, 303, 121, 28, 5, BinaryColor::Off);
        fill_rect(frame, 307, 132, 22, 5, BinaryColor::Off);
    }

    draw_text(
        frame,
        shell.rows[2].meta.as_str(),
        Point::new(20, 177 + slide_offset),
        ui_font_body(),
        BinaryColor::On,
        Alignment::Left,
    );
    draw_text(
        frame,
        shell.rows[2].title.as_str(),
        Point::new(20, 195 + slide_offset),
        ui_font_body(),
        BinaryColor::On,
        Alignment::Left,
    );
}

fn draw_reader(frame: &mut FrameBuffer, shell: &ReaderShell, step: u8, total_steps: u8) {
    draw_text(
        frame,
        shell.stage.title.as_str(),
        Point::new(20, 18),
        ui_font_title(),
        BinaryColor::On,
        Alignment::Left,
    );
    draw_text_right(
        frame,
        wpm_label(shell.stage.wpm),
        Point::new(380, 18),
        ui_font_body(),
        BinaryColor::On,
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

    draw_text(
        frame,
        shell.stage.preview.as_str(),
        Point::new(20, 184),
        ui_font_body(),
        BinaryColor::On,
        Alignment::Left,
    );
    fill_rect(
        frame,
        0,
        232,
        shell.stage.progress_width.into(),
        8,
        BinaryColor::On,
    );

    if let Some(modal) = shell.pause_modal {
        draw_pause_modal(frame, &modal, step, total_steps);
    }
}

fn draw_pause_modal(frame: &mut FrameBuffer, modal: &PauseModal, step: u8, total_steps: u8) {
    let width = lerp_u32(112, 286, step, total_steps);
    let height = lerp_u32(52, 166, step, total_steps);
    let x = 200 - (width as i32 / 2);
    let y = 118 - (height as i32 / 2);

    fill_rect(frame, x, y, width as i32, height as i32, BinaryColor::On);

    if step >= 2 {
        draw_text(
            frame,
            modal.title,
            Point::new(200, y + 14),
            ui_font_title(),
            BinaryColor::Off,
            Alignment::Center,
        );
        fill_rect(
            frame,
            x + 16,
            y + 46,
            width as i32 - 32,
            1,
            BinaryColor::Off,
        );
    }

    if step >= total_steps {
        draw_text(
            frame,
            modal.rows[0].label,
            Point::new(x + 18, y + 60),
            ui_font_small(),
            BinaryColor::Off,
            Alignment::Left,
        );
        draw_text(
            frame,
            modal.rows[0].action,
            Point::new(x + 166, y + 60),
            ui_font_small(),
            BinaryColor::Off,
            Alignment::Left,
        );
        draw_text(
            frame,
            modal.rows[1].label,
            Point::new(x + 18, y + 88),
            ui_font_small(),
            BinaryColor::Off,
            Alignment::Left,
        );
        draw_text(
            frame,
            modal.rows[1].action,
            Point::new(x + 166, y + 88),
            ui_font_small(),
            BinaryColor::Off,
            Alignment::Left,
        );
        draw_text(
            frame,
            modal.rows[2].label,
            Point::new(x + 18, y + 116),
            ui_font_small(),
            BinaryColor::Off,
            Alignment::Left,
        );
        draw_text(
            frame,
            modal.rows[2].action,
            Point::new(x + 166, y + 116),
            ui_font_small(),
            BinaryColor::Off,
            Alignment::Left,
        );
    }
}

fn draw_stage_token(frame: &mut FrameBuffer, left: &str, right: &str, font: StageFont) {
    let (font, y) = stage_font_spec(font);
    draw_text_right(
        frame,
        left,
        Point::new(RSVP_STAGE_LEFT_ANCHOR_X, y),
        font,
        BinaryColor::On,
    );
    draw_text(
        frame,
        right,
        Point::new(RSVP_STAGE_RIGHT_ANCHOR_X, y),
        font,
        BinaryColor::On,
        Alignment::Left,
    );
}

fn draw_paragraph_navigation(
    frame: &mut FrameBuffer,
    shell: &ParagraphNavigationShell,
    step: u8,
    total_steps: u8,
) {
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
        shell.previous_top.as_str(),
        Point::new(20, 58),
        ui_font_body(),
        BinaryColor::On,
        Alignment::Left,
    );

    let card_color = if step == 2 {
        BinaryColor::Off
    } else {
        BinaryColor::On
    };
    let card_text = if matches!(card_color, BinaryColor::Off) {
        BinaryColor::On
    } else {
        BinaryColor::Off
    };
    fill_rect(frame, 20, 88, 304, 82, card_color);
    draw_text(
        frame,
        shell.selected_label.as_str(),
        Point::new(34, 106),
        ui_font_body(),
        card_text,
        Alignment::Left,
    );
    draw_text(
        frame,
        shell.selected_excerpt.as_str(),
        Point::new(34, 126),
        ui_font_body(),
        card_text,
        Alignment::Left,
    );

    draw_text(
        frame,
        shell.previous_bottom.as_str(),
        Point::new(20, 188),
        ui_font_body(),
        BinaryColor::On,
        Alignment::Left,
    );
    draw_text(
        frame,
        shell.final_excerpt.as_str(),
        Point::new(20, 210),
        ui_font_body(),
        BinaryColor::On,
        Alignment::Left,
    );

    draw_paragraph_map_rail(
        frame,
        shell.rail.selected_index,
        shell.rail.total_ticks,
        step,
        total_steps,
    );
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

fn draw_vertical_rail(frame: &mut FrameBuffer, text: &str, right_edge: i32, y: i32) {
    draw_text_right(
        frame,
        text,
        Point::new(right_edge, y),
        ui_font_title(),
        BinaryColor::On,
    );
}

fn draw_paragraph_map_rail(
    frame: &mut FrameBuffer,
    selected_index: u8,
    total_ticks: u8,
    step: u8,
    total_steps: u8,
) {
    let x = 352;
    let y = 40;

    let mut index = 0;
    while index < total_ticks as i32 {
        let tick_y = y + match index {
            0 => 0,
            1 => 24,
            2 => 48,
            3 => 76,
            4 => 116,
            5 => 140,
            _ => 162,
        };

        if index == selected_index as i32 {
            let accent_width = lerp_u32(4, 12, step, total_steps) as i32;
            stroke_rect(
                frame,
                x + (12 - accent_width),
                tick_y,
                accent_width,
                28,
                BinaryColor::On,
            );
            fill_rect(
                frame,
                x + (12 - accent_width),
                tick_y,
                accent_width,
                28,
                BinaryColor::On,
            );
        } else {
            fill_rect(frame, x + 8, tick_y, 4, 14, BinaryColor::On);
        }

        index += 1;
    }
}

fn draw_tick_motion_accent(
    frame: &mut FrameBuffer,
    rail: &app_runtime::components::ParagraphMapRail,
    direction: MotionDirection,
    step: u8,
    total_steps: u8,
) {
    let accent_x = match direction {
        MotionDirection::Forward => 348 + (step as i32 * 2),
        MotionDirection::Backward => 352 - (step as i32 * 2),
    };
    let accent_y = match rail.selected_index {
        0 => 40,
        1 => 64,
        2 => 88,
        3 => 116,
        4 => 156,
        5 => 180,
        _ => 202,
    };

    stroke_rect(
        frame,
        accent_x,
        accent_y,
        lerp_u32(4, 12, step, total_steps) as i32,
        28,
        BinaryColor::On,
    );
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
    for byte in frame.bytes_mut() {
        *byte = !*byte;
    }
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
        let mut column = x;
        while column < visible_right {
            set_pixel_color(frame, column, y + row, BinaryColor::On);
            column += 1;
        }
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
    if width <= 0 || height <= 0 {
        return;
    }

    Rectangle::new(Point::new(x, y), Size::new(width as u32, height as u32))
        .into_styled(PrimitiveStyle::with_fill(color))
        .draw(frame)
        .ok();
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

    Rectangle::new(Point::new(x, y), Size::new(width as u32, height as u32))
        .into_styled(
            PrimitiveStyleBuilder::new()
                .stroke_color(color)
                .stroke_width(1)
                .build(),
        )
        .draw(frame)
        .ok();
}

fn draw_text(
    frame: &mut FrameBuffer,
    text: &str,
    position: Point,
    font: &embedded_graphics::mono_font::MonoFont<'static>,
    color: BinaryColor,
    alignment: Alignment,
) {
    let style = MonoTextStyleBuilder::new()
        .font(font)
        .text_color(color)
        .build();
    let text_style = TextStyleBuilder::new()
        .alignment(alignment)
        .baseline(Baseline::Top)
        .build();

    Text::with_text_style(text, position, style, text_style)
        .draw(frame)
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
