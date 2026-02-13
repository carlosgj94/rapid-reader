pub(super) fn decode_html_entity(entity: &[u8]) -> Option<char> {
    if entity.eq_ignore_ascii_case(b"amp") {
        Some('&')
    } else if entity.eq_ignore_ascii_case(b"lt") {
        Some('<')
    } else if entity.eq_ignore_ascii_case(b"gt") {
        Some('>')
    } else if entity.eq_ignore_ascii_case(b"quot") {
        Some('"')
    } else if entity.eq_ignore_ascii_case(b"apos")
        || entity.eq_ignore_ascii_case(b"lsquo")
        || entity.eq_ignore_ascii_case(b"rsquo")
    {
        Some('\'')
    } else if entity.eq_ignore_ascii_case(b"ldquo")
        || entity.eq_ignore_ascii_case(b"rdquo")
        || entity.eq_ignore_ascii_case(b"laquo")
        || entity.eq_ignore_ascii_case(b"raquo")
    {
        Some('"')
    } else if entity.eq_ignore_ascii_case(b"nbsp") || entity == b"#160" {
        Some(' ')
    } else if entity == b"#39" {
        Some('\'')
    } else if entity.eq_ignore_ascii_case(b"ndash") || entity.eq_ignore_ascii_case(b"mdash") {
        Some('-')
    } else if entity.eq_ignore_ascii_case(b"hellip") {
        Some('.')
    } else if entity.eq_ignore_ascii_case(b"aacute") {
        Some('á')
    } else if entity.eq_ignore_ascii_case(b"eacute") {
        Some('é')
    } else if entity.eq_ignore_ascii_case(b"iacute") {
        Some('í')
    } else if entity.eq_ignore_ascii_case(b"oacute") {
        Some('ó')
    } else if entity.eq_ignore_ascii_case(b"uacute") {
        Some('ú')
    } else if entity.eq_ignore_ascii_case(b"ntilde") {
        Some('ñ')
    } else if entity.eq_ignore_ascii_case(b"uuml") {
        Some('ü')
    } else if entity.eq_ignore_ascii_case(b"agrave") {
        Some('à')
    } else if entity.eq_ignore_ascii_case(b"egrave") {
        Some('è')
    } else if entity.eq_ignore_ascii_case(b"igrave") {
        Some('ì')
    } else if entity.eq_ignore_ascii_case(b"ograve") {
        Some('ò')
    } else if entity.eq_ignore_ascii_case(b"ugrave") {
        Some('ù')
    } else if entity.eq_ignore_ascii_case(b"ccedil") {
        Some('ç')
    } else if entity.eq_ignore_ascii_case(b"iexcl") {
        Some('¡')
    } else if entity.eq_ignore_ascii_case(b"iquest") {
        Some('¿')
    } else {
        decode_numeric_entity(entity)
    }
}

pub(super) fn decode_numeric_entity(entity: &[u8]) -> Option<char> {
    let first = entity.first().copied()?;
    if first != b'#' {
        return None;
    }

    let (digits, radix) = match entity.get(1).copied() {
        Some(b'x' | b'X') => (&entity[2..], 16),
        _ => (&entity[1..], 10),
    };
    if digits.is_empty() {
        return None;
    }

    let mut value = 0u32;
    for &digit in digits {
        let step = match digit {
            b'0'..=b'9' => (digit - b'0') as u32,
            b'a'..=b'f' if radix == 16 => (digit - b'a' + 10) as u32,
            b'A'..=b'F' if radix == 16 => (digit - b'A' + 10) as u32,
            _ => return None,
        };
        value = value.saturating_mul(radix).saturating_add(step);
    }

    core::char::from_u32(value)
}
