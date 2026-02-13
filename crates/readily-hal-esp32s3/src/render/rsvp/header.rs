use super::*;

const WIFI_ICON_SIZE: usize = 14;
const WIFI_ICON_RIGHT_PAD: usize = 12;
const WIFI_ICON_TEXT_GAP: usize = 8;

pub(super) fn wifi_icon_xy() -> (usize, usize) {
    (
        WIDTH.saturating_sub(WIFI_ICON_RIGHT_PAD + WIFI_ICON_SIZE),
        11,
    )
}

pub(super) fn header_right_text_x(right_width: usize) -> usize {
    let (icon_x, _) = wifi_icon_xy();
    let anchor = icon_x.saturating_sub(WIFI_ICON_TEXT_GAP);
    anchor.saturating_sub(right_width)
}

pub(super) fn draw_header_wifi_icon(
    frame: &mut FrameBuffer,
    connectivity: ConnectivitySnapshot,
    on: bool,
) {
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

pub(super) fn render_header_wpm(
    frame: &mut FrameBuffer,
    title: &str,
    wpm: u16,
    connectivity: ConnectivitySnapshot,
    on: bool,
) {
    render_header_wpm_custom(frame, title, wpm, 2, 2, connectivity, on);
}

pub(super) fn render_header_wpm_custom(
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

pub(super) fn render_header_text(
    frame: &mut FrameBuffer,
    title: &str,
    right: &str,
    connectivity: ConnectivitySnapshot,
    on: bool,
) {
    render_header_text_scaled(frame, title, right, 2, connectivity, on);
}

pub(super) fn render_header_text_scaled(
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

pub(super) fn render_library_header(
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

pub(super) fn draw_wifi_icon(
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
