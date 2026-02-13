use super::{
    html_entities::decode_html_entity,
    parsing_utils::{apply_html_tag_state, tag_inserts_paragraph_break},
    *,
};

pub(super) fn sanitize_epub_chunk(
    chunk: &[u8],
    html_state: &mut HtmlParseState,
    treat_as_plain_text: bool,
) -> (String<SD_CATALOG_TEXT_BYTES>, bool, Option<usize>) {
    let mut out = String::<SD_CATALOG_TEXT_BYTES>::new();
    let mut truncated = false;
    let mut tail_start = None;
    let mut last_was_space = true;
    let mut cursor = 0usize;

    while cursor < chunk.len() {
        let byte = chunk[cursor];
        if byte == b'<' {
            let Some(tag_end_rel) = chunk[cursor + 1..].iter().position(|b| *b == b'>') else {
                tail_start = Some(cursor);
                break;
            };
            let tag_end = cursor + 1 + tag_end_rel;
            let raw_tag = &chunk[cursor + 1..tag_end];
            let paragraph_break = tag_inserts_paragraph_break(raw_tag);
            apply_html_tag_state(raw_tag, html_state);
            cursor = tag_end + 1;
            if html_state.should_emit_text(treat_as_plain_text) {
                if paragraph_break {
                    push_paragraph_break(&mut out, &mut truncated, &mut last_was_space);
                } else {
                    push_normalized_char(&mut out, ' ', &mut truncated, &mut last_was_space);
                }
            }
            if truncated {
                break;
            }
            continue;
        }

        if byte == b'&' {
            let mut entity = [0u8; HTML_ENTITY_BYTES];
            let mut entity_len = 0usize;
            let mut entity_cursor = cursor + 1;
            let mut decoded = None;
            let mut incomplete = true;

            while entity_cursor < chunk.len() {
                let entity_byte = chunk[entity_cursor];
                if entity_byte == b';' {
                    decoded = Some(decode_html_entity(&entity[..entity_len]).unwrap_or(' '));
                    entity_cursor += 1;
                    incomplete = false;
                    break;
                }

                if entity_byte.is_ascii_alphanumeric() || matches!(entity_byte, b'#' | b'x' | b'X')
                {
                    if entity_len < entity.len() {
                        entity[entity_len] = entity_byte;
                        entity_len += 1;
                        entity_cursor += 1;
                        continue;
                    }
                    incomplete = false;
                    break;
                }

                incomplete = false;
                break;
            }

            if incomplete {
                tail_start = Some(cursor);
                break;
            }

            if html_state.should_emit_text(treat_as_plain_text) {
                push_normalized_char(
                    &mut out,
                    decoded.unwrap_or(' '),
                    &mut truncated,
                    &mut last_was_space,
                );
            }
            cursor = if decoded.is_some() {
                entity_cursor
            } else {
                cursor + 1
            };
            if truncated {
                break;
            }
            continue;
        }

        if !html_state.should_emit_text(treat_as_plain_text) {
            cursor += 1;
            continue;
        }

        match byte {
            b'\r' | b'\n' | b'\t' | b' ' => {
                push_normalized_char(&mut out, ' ', &mut truncated, &mut last_was_space);
                cursor += 1;
            }
            _ if byte.is_ascii_control() => {
                cursor += 1;
            }
            _ => match decode_utf8_char(chunk, cursor) {
                Utf8ChunkDecode::Char(ch, advance) => {
                    push_normalized_char(&mut out, ch, &mut truncated, &mut last_was_space);
                    cursor += advance;
                }
                Utf8ChunkDecode::Incomplete => {
                    tail_start = Some(cursor);
                    break;
                }
                Utf8ChunkDecode::Invalid => {
                    let fallback = decode_single_byte_fallback(byte);
                    push_normalized_char(&mut out, fallback, &mut truncated, &mut last_was_space);
                    cursor += 1;
                }
            },
        }

        if truncated {
            break;
        }
    }

    while out.ends_with(' ') || out.ends_with('\n') {
        let _ = out.pop();
    }

    (out, truncated, tail_start)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Utf8ChunkDecode {
    Char(char, usize),
    Incomplete,
    Invalid,
}

fn decode_utf8_char(chunk: &[u8], cursor: usize) -> Utf8ChunkDecode {
    let first = chunk[cursor];
    if first < 0x80 {
        return Utf8ChunkDecode::Char(first as char, 1);
    }

    let remaining = chunk.len().saturating_sub(cursor);
    if (0xC2..=0xDF).contains(&first) {
        if remaining < 2 {
            return Utf8ChunkDecode::Incomplete;
        }
        let b1 = chunk[cursor + 1];
        if !is_utf8_continuation(b1) {
            return Utf8ChunkDecode::Invalid;
        }
        let codepoint = (((first & 0x1f) as u32) << 6) | ((b1 & 0x3f) as u32);
        return core::char::from_u32(codepoint)
            .map(|ch| Utf8ChunkDecode::Char(ch, 2))
            .unwrap_or(Utf8ChunkDecode::Invalid);
    }

    if (0xE0..=0xEF).contains(&first) {
        if remaining < 3 {
            return Utf8ChunkDecode::Incomplete;
        }
        let b1 = chunk[cursor + 1];
        let b2 = chunk[cursor + 2];
        if !is_utf8_continuation(b1) || !is_utf8_continuation(b2) {
            return Utf8ChunkDecode::Invalid;
        }
        if (first == 0xE0 && b1 < 0xA0) || (first == 0xED && b1 >= 0xA0) {
            return Utf8ChunkDecode::Invalid;
        }
        let codepoint =
            (((first & 0x0f) as u32) << 12) | (((b1 & 0x3f) as u32) << 6) | ((b2 & 0x3f) as u32);
        return core::char::from_u32(codepoint)
            .map(|ch| Utf8ChunkDecode::Char(ch, 3))
            .unwrap_or(Utf8ChunkDecode::Invalid);
    }

    if (0xF0..=0xF4).contains(&first) {
        if remaining < 4 {
            return Utf8ChunkDecode::Incomplete;
        }
        let b1 = chunk[cursor + 1];
        let b2 = chunk[cursor + 2];
        let b3 = chunk[cursor + 3];
        if !is_utf8_continuation(b1) || !is_utf8_continuation(b2) || !is_utf8_continuation(b3) {
            return Utf8ChunkDecode::Invalid;
        }
        if (first == 0xF0 && b1 < 0x90) || (first == 0xF4 && b1 > 0x8F) {
            return Utf8ChunkDecode::Invalid;
        }
        let codepoint = (((first & 0x07) as u32) << 18)
            | (((b1 & 0x3f) as u32) << 12)
            | (((b2 & 0x3f) as u32) << 6)
            | ((b3 & 0x3f) as u32);
        return core::char::from_u32(codepoint)
            .map(|ch| Utf8ChunkDecode::Char(ch, 4))
            .unwrap_or(Utf8ChunkDecode::Invalid);
    }

    Utf8ChunkDecode::Invalid
}

fn is_utf8_continuation(byte: u8) -> bool {
    (byte & 0b1100_0000) == 0b1000_0000
}

fn decode_single_byte_fallback(byte: u8) -> char {
    match byte {
        0x91 | 0x92 => '\'',
        0x93 | 0x94 => '"',
        0x96 | 0x97 => '-',
        0x85 => '.',
        0xA0 => ' ',
        0xA1 => '¡',
        0xBF => '¿',
        0xC0 => 'À',
        0xC1 => 'Á',
        0xC8 => 'È',
        0xC9 => 'É',
        0xCC => 'Ì',
        0xCD => 'Í',
        0xD1 => 'Ñ',
        0xD2 => 'Ò',
        0xD3 => 'Ó',
        0xD9 => 'Ù',
        0xDA => 'Ú',
        0xDC => 'Ü',
        0xE0 => 'à',
        0xE1 => 'á',
        0xE7 => 'ç',
        0xE8 => 'è',
        0xE9 => 'é',
        0xEC => 'ì',
        0xED => 'í',
        0xF1 => 'ñ',
        0xF2 => 'ò',
        0xF3 => 'ó',
        0xF9 => 'ù',
        0xFA => 'ú',
        0xFC => 'ü',
        _ if byte.is_ascii() => byte as char,
        _ => '?',
    }
}

fn push_paragraph_break<const N: usize>(
    out: &mut String<N>,
    truncated: &mut bool,
    last_was_space: &mut bool,
) {
    while out.ends_with(' ') {
        let _ = out.pop();
    }

    if out.is_empty() || out.ends_with('\n') {
        *last_was_space = true;
        return;
    }

    if out.push('\n').is_err() {
        *truncated = true;
        return;
    }
    *last_was_space = true;
}

fn push_normalized_char<const N: usize>(
    out: &mut String<N>,
    ch: char,
    truncated: &mut bool,
    last_was_space: &mut bool,
) {
    if ch.is_ascii_whitespace() {
        if out.is_empty() || *last_was_space {
            return;
        }
        if out.push(' ').is_err() {
            *truncated = true;
            return;
        }
        *last_was_space = true;
        return;
    }

    if out.push(ch).is_err() {
        *truncated = true;
        return;
    }
    *last_was_space = false;
}
