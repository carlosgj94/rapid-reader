use core::str;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct InlineText<const N: usize> {
    bytes: [u8; N],
    len: u8,
}

impl<const N: usize> InlineText<N> {
    pub const fn new() -> Self {
        Self {
            bytes: [0; N],
            len: 0,
        }
    }

    pub fn from_slice(value: &str) -> Self {
        let mut text = Self::new();
        let _ = text.try_push_str(value);
        text
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    pub fn try_push_str(&mut self, value: &str) -> bool {
        let bytes = value.as_bytes();
        if self.len as usize + bytes.len() > N {
            return false;
        }

        let start = self.len as usize;
        let end = start + bytes.len();
        self.bytes[start..end].copy_from_slice(bytes);
        self.len = end as u8;
        true
    }

    pub fn try_push_char(&mut self, ch: char) -> bool {
        let mut utf8 = [0u8; 4];
        self.try_push_str(ch.encode_utf8(&mut utf8))
    }

    pub fn set_truncated(&mut self, value: &str) {
        self.clear();

        for ch in value.chars() {
            if !self.try_push_char(ch) {
                break;
            }
        }
    }

    pub fn as_str(&self) -> &str {
        str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or("")
    }

    pub const fn len(&self) -> usize {
        self.len as usize
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn char_count(&self) -> usize {
        self.as_str().chars().count()
    }
}

impl<const N: usize> Default for InlineText<N> {
    fn default() -> Self {
        Self::new()
    }
}
