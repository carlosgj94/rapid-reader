use super::*;

pub(super) fn draw_settings_rows(
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
pub(super) enum ValueLabel<'a> {
    Plain(&'a str),
    Wrapped {
        prefix: &'a str,
        core: &'a str,
        suffix: &'a str,
    },
}

pub(super) fn highlighted_value<'a>(core: &'a str) -> ValueLabel<'a> {
    ValueLabel::Wrapped {
        prefix: "[",
        core,
        suffix: "]",
    }
}

pub(super) fn draw_right_label(frame: &mut FrameBuffer, y: usize, label: ValueLabel<'_>, on: bool) {
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

pub(super) fn draw_footer_hint(frame: &mut FrameBuffer, text: &str, on: bool) {
    draw_text(frame, 12, HEIGHT - 30, text, 1, on);
}
