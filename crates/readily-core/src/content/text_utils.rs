pub(super) fn count_words(text: &str) -> usize {
    let mut count = 0usize;
    let mut cursor = 0usize;

    while let Some((_, next_cursor)) = next_word_at(text, cursor) {
        count += 1;
        cursor = next_cursor;
    }

    count
}

pub(super) fn next_word_at(text: &str, mut cursor: usize) -> Option<(&str, usize)> {
    let bytes = text.as_bytes();
    let len = bytes.len();

    while cursor < len && bytes[cursor].is_ascii_whitespace() {
        cursor += 1;
    }
    if cursor >= len {
        return None;
    }

    let start = cursor;
    while cursor < len && !bytes[cursor].is_ascii_whitespace() {
        cursor += 1;
    }

    Some((&text[start..cursor], cursor))
}

pub(super) fn first_words_excerpt(text: &str, max_words: usize) -> &str {
    if text.is_empty() || max_words == 0 {
        return "";
    }

    let mut words = 0usize;
    let mut end = 0usize;

    for (idx, ch) in text.char_indices() {
        if ch.is_whitespace() && end != idx {
            words += 1;
            if words >= max_words {
                return &text[..idx];
            }
        }
        end = idx + ch.len_utf8();
    }

    text[..end].trim_end()
}
