struct WordBuffer<const N: usize> {
    bytes: [u8; N],
    len: usize,
}

impl<const N: usize> WordBuffer<N> {
    const fn new() -> Self {
        Self {
            bytes: [0u8; N],
            len: 0,
        }
    }

    fn clear(&mut self) {
        self.len = 0;
    }

    fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn set(&mut self, word: &str) {
        self.len = 0;

        for ch in word.chars() {
            let mut utf8 = [0u8; 4];
            let encoded = ch.encode_utf8(&mut utf8).as_bytes();
            if self.len + encoded.len() > N {
                break;
            }

            self.bytes[self.len..self.len + encoded.len()].copy_from_slice(encoded);
            self.len += encoded.len();
        }

        if self.len == 0 {
            self.bytes[0] = b'?';
            self.len = 1;
        }
    }

    fn as_str(&self) -> &str {
        if self.len == 0 {
            return "";
        }

        str::from_utf8(&self.bytes[..self.len]).unwrap_or("?")
    }
}

fn rotate_cw(current: u16, total: u16) -> u16 {
    if total == 0 { 0 } else { (current + 1) % total }
}

fn rotate_ccw(current: u16, total: u16) -> u16 {
    if total == 0 {
        0
    } else if current == 0 {
        total - 1
    } else {
        current - 1
    }
}

fn font_family_label(font: FontFamily) -> &'static str {
    match font {
        FontFamily::Serif => "Serif",
        FontFamily::Pixel => "Pixel",
    }
}

fn font_size_label(size: FontSize) -> &'static str {
    match size {
        FontSize::Small => "Small",
        FontSize::Medium => "Medium",
        FontSize::Large => "Large",
    }
}
