//! Shared text shaping and truncation policies for compact UI surfaces.

use core::str;

pub const COMPACT_MAX_WORDS: usize = 7;
pub const COMPACT_MAX_CHARS: usize = 34;

pub fn preview_compact<'a>(source: &str, out: &'a mut [u8]) -> &'a str {
    preview_limited(source, out, COMPACT_MAX_WORDS, COMPACT_MAX_CHARS)
}

pub fn preview_limited<'a>(
    source: &str,
    out: &'a mut [u8],
    max_words: usize,
    max_chars: usize,
) -> &'a str {
    if out.is_empty() {
        return "";
    }

    let mut len = 0usize;
    let mut char_count = 0usize;
    let mut truncated = false;

    for (word_count, word) in source.split_whitespace().enumerate() {
        if word_count >= max_words {
            truncated = true;
            break;
        }

        if word_count > 0 {
            if len + 1 > out.len() || char_count >= max_chars {
                truncated = true;
                break;
            }
            out[len] = b' ';
            len += 1;
            char_count += 1;
        }

        for ch in word.chars() {
            let mut utf8 = [0u8; 4];
            let encoded = ch.encode_utf8(&mut utf8).as_bytes();
            if char_count >= max_chars || len + encoded.len() > out.len() {
                truncated = true;
                break;
            }

            out[len..len + encoded.len()].copy_from_slice(encoded);
            len += encoded.len();
            char_count += 1;
        }

        if truncated {
            break;
        }
    }

    if len == 0 {
        return "";
    }

    if truncated && len + 3 <= out.len() {
        out[len..len + 3].copy_from_slice(b"...");
        len += 3;
    }

    str::from_utf8(&out[..len]).unwrap_or("?")
}

pub fn chapter_number_label(chapter_number: u16, out: &mut [u8; 10]) -> &str {
    let len = write_u16_ascii(chapter_number, out);
    str::from_utf8(&out[..len]).unwrap_or("1")
}

pub fn section_secondary_label<'a>(
    index: u16,
    total: u16,
    suffix: &str,
    out: &'a mut [u8],
) -> &'a str {
    let mut len = 0usize;
    len += write_u16_ascii(index, &mut out[len..]);
    if len + 1 >= out.len() {
        return str::from_utf8(&out[..len]).unwrap_or("");
    }
    out[len] = b'/';
    len += 1;
    len += write_u16_ascii(total, &mut out[len..]);

    if !suffix.is_empty() && len + 1 < out.len() {
        out[len] = b' ';
        len += 1;
        for ch in suffix.chars() {
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

pub fn write_u16_ascii(mut value: u16, out: &mut [u8]) -> usize {
    if out.is_empty() {
        return 0;
    }

    if value == 0 {
        out[0] = b'0';
        return 1;
    }

    let mut tmp = [0u8; 5];
    let mut n = 0usize;
    while value > 0 && n < tmp.len() {
        tmp[n] = b'0' + (value % 10) as u8;
        value /= 10;
        n += 1;
    }

    let len = n.min(out.len());
    for i in 0..len {
        out[i] = tmp[n - 1 - i];
    }
    len
}
