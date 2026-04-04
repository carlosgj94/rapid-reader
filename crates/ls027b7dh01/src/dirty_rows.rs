//! Dirty row tracking for LS027B7DH01 framebuffer updates.

use crate::protocol::HEIGHT;

const WORD_BITS: usize = u32::BITS as usize;
const WORD_COUNT: usize = HEIGHT.div_ceil(WORD_BITS);
const LAST_WORD_MASK: u32 = match HEIGHT % WORD_BITS {
    0 => u32::MAX,
    remainder => (1u32 << remainder) - 1,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DirtyRows {
    words: [u32; WORD_COUNT],
}

impl DirtyRows {
    pub const fn new() -> Self {
        Self {
            words: [0; WORD_COUNT],
        }
    }

    pub fn clear(&mut self) {
        self.words.fill(0);
    }

    pub fn mark_row(&mut self, row: usize) -> bool {
        if row >= HEIGHT {
            return false;
        }

        let word = row / WORD_BITS;
        let bit = row % WORD_BITS;
        self.words[word] |= 1u32 << bit;
        true
    }

    pub fn mark_line(&mut self, line: u16) -> bool {
        if !(1..=HEIGHT as u16).contains(&line) {
            return false;
        }

        self.mark_row(line as usize - 1)
    }

    pub fn mark_all(&mut self) {
        self.words.fill(u32::MAX);
        self.words[WORD_COUNT - 1] = LAST_WORD_MASK;
    }

    pub fn is_dirty_row(&self, row: usize) -> bool {
        if row >= HEIGHT {
            return false;
        }

        let word = row / WORD_BITS;
        let bit = row % WORD_BITS;
        (self.words[word] & (1u32 << bit)) != 0
    }

    pub fn is_empty(&self) -> bool {
        self.words.iter().all(|word| *word == 0)
    }

    pub fn count(&self) -> u16 {
        self.words.iter().map(|word| word.count_ones() as u16).sum()
    }

    pub fn is_full_height(&self) -> bool {
        self.count() as usize == HEIGHT
    }

    pub fn iter(&self) -> DirtyRowIter {
        DirtyRowIter {
            dirty_rows: *self,
            next_row: 0,
        }
    }

    pub fn iter_spans(&self) -> DirtyRowSpanIter {
        DirtyRowSpanIter {
            dirty_rows: *self,
            next_row: 0,
        }
    }
}

impl Default for DirtyRows {
    fn default() -> Self {
        Self::new()
    }
}

pub struct DirtyRowIter {
    dirty_rows: DirtyRows,
    next_row: usize,
}

impl Iterator for DirtyRowIter {
    type Item = usize;

    fn next(&mut self) -> Option<Self::Item> {
        while self.next_row < HEIGHT {
            let row = self.next_row;
            self.next_row += 1;
            if self.dirty_rows.is_dirty_row(row) {
                return Some(row);
            }
        }

        None
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DirtyRowSpan {
    pub start_row: usize,
    pub end_row: usize,
}

pub struct DirtyRowSpanIter {
    dirty_rows: DirtyRows,
    next_row: usize,
}

impl Iterator for DirtyRowSpanIter {
    type Item = DirtyRowSpan;

    fn next(&mut self) -> Option<Self::Item> {
        while self.next_row < HEIGHT && !self.dirty_rows.is_dirty_row(self.next_row) {
            self.next_row += 1;
        }

        if self.next_row >= HEIGHT {
            return None;
        }

        let start_row = self.next_row;
        while self.next_row < HEIGHT && self.dirty_rows.is_dirty_row(self.next_row) {
            self.next_row += 1;
        }

        Some(DirtyRowSpan {
            start_row,
            end_row: self.next_row - 1,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_and_count_rows() {
        let mut dirty = DirtyRows::new();

        assert!(dirty.mark_row(0));
        assert!(dirty.mark_row(5));
        assert!(dirty.mark_line(240));
        assert!(!dirty.mark_line(241));

        assert_eq!(dirty.count(), 3);
        assert!(dirty.is_dirty_row(0));
        assert!(dirty.is_dirty_row(5));
        assert!(dirty.is_dirty_row(239));
        assert!(!dirty.is_dirty_row(1));
    }

    #[test]
    fn spans_group_contiguous_rows() {
        let mut dirty = DirtyRows::new();
        let _ = dirty.mark_row(4);
        let _ = dirty.mark_row(5);
        let _ = dirty.mark_row(9);
        let _ = dirty.mark_row(10);
        let _ = dirty.mark_row(11);

        let spans: std::vec::Vec<DirtyRowSpan> = dirty.iter_spans().collect();
        assert_eq!(
            spans.as_slice(),
            &[
                DirtyRowSpan {
                    start_row: 4,
                    end_row: 5
                },
                DirtyRowSpan {
                    start_row: 9,
                    end_row: 11
                },
            ]
        );
    }

    #[test]
    fn mark_all_covers_entire_panel_height() {
        let mut dirty = DirtyRows::new();
        dirty.mark_all();

        assert!(dirty.is_full_height());
        assert_eq!(dirty.count() as usize, HEIGHT);
        assert!(dirty.is_dirty_row(HEIGHT - 1));
    }
}
