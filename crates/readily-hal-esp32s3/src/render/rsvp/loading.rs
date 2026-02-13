use super::*;

const ORBIT_POINTS: [(isize, isize); 12] = [
    (0, -22),
    (11, -18),
    (18, -11),
    (22, 0),
    (18, 11),
    (11, 18),
    (0, 22),
    (-11, 18),
    (-18, 11),
    (-22, 0),
    (-18, -11),
    (-11, -18),
];

#[derive(Clone, Copy, Debug)]
pub struct LoadingView<'a> {
    pub title: &'a str,
    pub subtitle: &'a str,
    pub phase: &'a str,
    pub detail: &'a str,
    pub progress_current: u16,
    pub progress_total: u16,
    pub elapsed_ms: u64,
}

pub(super) fn draw_loading_stage(
    frame: &mut FrameBuffer,
    view: LoadingView<'_>,
    connectivity: ConnectivitySnapshot,
    on: bool,
) {
    draw_rect(frame, 0, 0, WIDTH, HEIGHT, on);
    render_header_text(frame, view.title, view.subtitle, connectivity, on);

    draw_loading_background(frame, view.elapsed_ms, on);

    let card_x = 14usize;
    let card_y = 52usize;
    let card_w = WIDTH.saturating_sub(28);
    let card_h = HEIGHT.saturating_sub(80);
    draw_rect(frame, card_x, card_y, card_w, card_h, on);
    if card_w > 4 && card_h > 4 {
        draw_rect(frame, card_x + 2, card_y + 2, card_w - 4, card_h - 4, on);
    }

    let mut phase_buf = [0u8; SELECTOR_TEXT_BUF];
    let phase_fit = fit_text_in_width(view.phase, card_w.saturating_sub(18), 2, &mut phase_buf);
    draw_text_centered(frame, card_y + 12, phase_fit, 2, on);

    draw_orbit_spinner(
        frame,
        (WIDTH / 2) as isize,
        (card_y + card_h / 2) as isize + 2,
        view.elapsed_ms,
        on,
    );

    let mut detail_buf = [0u8; SELECTOR_TEXT_BUF];
    let detail_fit = fit_text_in_width(view.detail, card_w.saturating_sub(18), 1, &mut detail_buf);
    draw_text_centered(frame, card_y + card_h.saturating_sub(34), detail_fit, 1, on);

    draw_loading_progress(
        frame,
        card_x + 10,
        HEIGHT.saturating_sub(24),
        WIDTH.saturating_sub(48),
        10,
        view.progress_current as usize,
        view.progress_total as usize,
        view.elapsed_ms,
        on,
    );
}

fn draw_loading_background(frame: &mut FrameBuffer, elapsed_ms: u64, on: bool) {
    let drift = ((elapsed_ms / 45) as usize) % 24;
    let base_y = 44usize;
    for row in 0..6usize {
        let y = base_y + row * 30;
        let x = 8usize + ((drift + row * 9) % 24);
        draw_filled_rect(frame, x, y, 18, 1, on);
        draw_filled_rect(frame, WIDTH.saturating_sub(26 + x), y + 10, 14, 1, on);
    }
}

fn draw_orbit_spinner(frame: &mut FrameBuffer, cx: isize, cy: isize, elapsed_ms: u64, on: bool) {
    let head = ((elapsed_ms / 85) % ORBIT_POINTS.len() as u64) as usize;
    let pulse = triangle_wave((elapsed_ms / 40) as usize, 8).saturating_add(4) as isize;
    draw_filled_rect_signed(frame, cx - pulse / 2, cy - pulse / 2, pulse, pulse, on);

    for (idx, (dx, dy)) in ORBIT_POINTS.iter().enumerate() {
        let age = (idx + ORBIT_POINTS.len() - head) % ORBIT_POINTS.len();
        if age > 3 {
            continue;
        }
        let size = match age {
            0 => 5,
            1 => 4,
            _ => 3,
        } as isize;
        draw_filled_rect_signed(
            frame,
            cx + dx - size / 2,
            cy + dy - size / 2,
            size,
            size,
            on,
        );
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "loading progress keeps params explicit"
)]
fn draw_loading_progress(
    frame: &mut FrameBuffer,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    current: usize,
    total: usize,
    elapsed_ms: u64,
    on: bool,
) {
    if w < 4 || h < 4 {
        return;
    }

    draw_rect(frame, x, y, w, h, on);
    let inner_w = w.saturating_sub(2);
    let inner_h = h.saturating_sub(2);

    if total > 0 {
        let clamped = current.min(total);
        let fill_w = (inner_w * clamped) / total.max(1);
        if fill_w > 0 {
            draw_filled_rect(frame, x + 1, y + 1, fill_w, inner_h, on);
        }
        return;
    }

    let travel = inner_w.saturating_sub(12).max(1);
    let period = travel.saturating_mul(2);
    let step = ((elapsed_ms / 35) as usize) % period.max(1);
    let offset = if step <= travel { step } else { period - step };
    draw_filled_rect(frame, x + 1 + offset, y + 1, 12.min(inner_w), inner_h, on);
}

fn triangle_wave(step: usize, span: usize) -> usize {
    let period = span.saturating_mul(2).max(1);
    let pos = step % period;
    if pos <= span { pos } else { period - pos }
}
