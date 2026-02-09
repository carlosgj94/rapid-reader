//! App-level view models and animation metadata.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FontFamily {
    Serif,
    Pixel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FontSize {
    Small,
    Medium,
    Large,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VisualStyle {
    pub font_family: FontFamily,
    pub font_size: FontSize,
    pub inverted: bool,
}

impl Default for VisualStyle {
    fn default() -> Self {
        Self {
            font_family: FontFamily::Serif,
            font_size: FontSize::Medium,
            inverted: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MenuItemKind {
    Text,
    Settings,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MenuItemView<'a> {
    pub label: &'a str,
    pub kind: MenuItemKind,
}

impl Default for MenuItemView<'_> {
    fn default() -> Self {
        Self {
            label: "",
            kind: MenuItemKind::Text,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingValue<'a> {
    Label(&'a str),
    Toggle(bool),
    Number(u16),
    Action(&'a str),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SettingRowView<'a> {
    pub key: &'a str,
    pub value: SettingValue<'a>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnimationKind {
    SlideLeft,
    SlideRight,
    Fade,
    Pulse,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AnimationFrame {
    pub kind: AnimationKind,
    /// 0..=100
    pub progress_pct: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AnimationSpec {
    pub kind: AnimationKind,
    pub start_ms: u64,
    pub duration_ms: u16,
}

impl AnimationSpec {
    pub const fn new(kind: AnimationKind, start_ms: u64, duration_ms: u16) -> Self {
        Self {
            kind,
            start_ms,
            duration_ms,
        }
    }

    pub fn frame(self, now_ms: u64) -> Option<AnimationFrame> {
        let duration = self.duration_ms.max(1) as u64;
        let elapsed = now_ms.saturating_sub(self.start_ms);
        if elapsed >= duration {
            return None;
        }

        let progress = ((elapsed * 100) / duration).min(100) as u8;
        Some(AnimationFrame {
            kind: self.kind,
            progress_pct: progress,
        })
    }
}

/// App-level view model consumed by board/HAL renderer.
pub enum Screen<'a> {
    Library {
        title: &'a str,
        subtitle: &'a str,
        items: &'a [MenuItemView<'a>],
        cursor: usize,
        style: VisualStyle,
        animation: Option<AnimationFrame>,
    },
    Settings {
        title: &'a str,
        subtitle: &'a str,
        rows: &'a [SettingRowView<'a>],
        cursor: usize,
        editing: bool,
        style: VisualStyle,
        animation: Option<AnimationFrame>,
    },
    Countdown {
        title: &'a str,
        wpm: u16,
        remaining: u8,
        style: VisualStyle,
        animation: Option<AnimationFrame>,
    },
    Reading {
        title: &'a str,
        wpm: u16,
        word: &'a str,
        paragraph_word_index: u16,
        paragraph_word_total: u16,
        paused: bool,
        paused_elapsed_ms: u32,
        pause_chapter_label: &'a str,
        style: VisualStyle,
        animation: Option<AnimationFrame>,
    },
    NavigateChapters {
        title: &'a str,
        wpm: u16,
        current_chapter: u16,
        target_chapter: u16,
        chapter_total: u16,
        current_label: &'a str,
        target_label: &'a str,
        current_secondary: &'a str,
        target_secondary: &'a str,
        style: VisualStyle,
        animation: Option<AnimationFrame>,
    },
    NavigateParagraphs {
        title: &'a str,
        wpm: u16,
        chapter_label: &'a str,
        current_preview: &'a str,
        target_preview: &'a str,
        current_secondary: &'a str,
        target_secondary: &'a str,
        target_index_in_chapter: u16,
        paragraph_total_in_chapter: u16,
        style: VisualStyle,
        animation: Option<AnimationFrame>,
    },
    Status {
        title: &'a str,
        wpm: u16,
        line1: &'a str,
        line2: &'a str,
        style: VisualStyle,
        animation: Option<AnimationFrame>,
    },
}
