use super::*;

pub(super) fn draw_rect(frame: &mut FrameBuffer, x: usize, y: usize, w: usize, h: usize, on: bool) {
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

pub(super) fn draw_filled_rect(
    frame: &mut FrameBuffer,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    on: bool,
) {
    for py in y..(y + h) {
        for px in x..(x + w) {
            set_pixel(frame, px, py, on);
        }
    }
}

pub(super) fn draw_filled_rect_signed(
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

pub(super) fn set_pixel(frame: &mut FrameBuffer, x: usize, y: usize, on: bool) {
    let _ = frame.set_pixel(x, y, on);
}

pub(super) fn set_pixel_signed(frame: &mut FrameBuffer, x: isize, y: isize, on: bool) {
    if x < 0 || y < 0 {
        return;
    }

    set_pixel(frame, x as usize, y as usize, on);
}
