use core::fmt::Write;

use heapless::String as HeaplessString;
use readily_hal_esp32s3::render::rsvp::LoadingView;

use super::SD_SPI_HZ_CANDIDATES;

const WAKE_VISIBILITY_DELAY_MS: u64 = 250;
const FRAME_INTERVAL_MS: u64 = 90;
const DETAIL_BYTES: usize = 80;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LoadingMode {
    ColdBoot,
    WakeFromDeepSleep,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LoadingEvent {
    Begin,
    ProbeAttempt {
        speed_index: usize,
        attempt: u8,
        max_attempts: u8,
    },
    ProbeSuccess {
        speed_index: usize,
    },
    ProbeRetryTick {
        speed_index: usize,
        attempt: u8,
        remaining_ms: u64,
    },
    ScanResult {
        books_dir_found: bool,
        books_total: u16,
    },
    BookText {
        index: u16,
        total: u16,
    },
    BookCover {
        index: u16,
        total: u16,
    },
    FallbackNoCatalog,
    Finished,
}

impl LoadingEvent {
    fn immediate_redraw(self) -> bool {
        !matches!(self, Self::ProbeRetryTick { .. })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LoadingPhase {
    Init,
    ProbeSd,
    ScanCatalog,
    LoadText,
    LoadCover,
    Fallback,
    Finalize,
}

impl LoadingPhase {
    fn label(self) -> &'static str {
        match self {
            Self::Init => "Preparing",
            Self::ProbeSd => "Probing SD Card",
            Self::ScanCatalog => "Scanning Library",
            Self::LoadText => "Loading Chapters",
            Self::LoadCover => "Decoding Covers",
            Self::Fallback => "Using Fallback",
            Self::Finalize => "Finalizing",
        }
    }
}

pub(super) struct LoadingCoordinator {
    mode: LoadingMode,
    phase: LoadingPhase,
    progress_current: u16,
    progress_total: u16,
    detail: HeaplessString<DETAIL_BYTES>,
    visible: bool,
    last_render_ms: Option<u64>,
}

impl LoadingCoordinator {
    pub(super) fn new(mode: LoadingMode) -> Self {
        let mut detail = HeaplessString::<DETAIL_BYTES>::new();
        let _ = detail.push_str("Initializing...");

        Self {
            mode,
            phase: LoadingPhase::Init,
            progress_current: 0,
            progress_total: 0,
            detail,
            visible: matches!(mode, LoadingMode::ColdBoot),
            last_render_ms: None,
        }
    }

    pub(super) fn on_event(&mut self, now_ms: u64, event: LoadingEvent) -> bool {
        self.apply_event(event);

        let became_visible = self.update_visibility(now_ms);
        if !self.visible {
            return false;
        }

        let frame_due = match self.last_render_ms {
            Some(last_ms) => now_ms.saturating_sub(last_ms) >= FRAME_INTERVAL_MS,
            None => true,
        };

        if became_visible || event.immediate_redraw() || frame_due {
            self.last_render_ms = Some(now_ms);
            true
        } else {
            false
        }
    }

    pub(super) fn view(&self, now_ms: u64) -> LoadingView<'_> {
        LoadingView {
            title: "READILY",
            subtitle: match self.mode {
                LoadingMode::ColdBoot => "Cold Boot",
                LoadingMode::WakeFromDeepSleep => "Wake Resume",
            },
            phase: self.phase.label(),
            detail: if self.detail.is_empty() {
                "Please wait..."
            } else {
                self.detail.as_str()
            },
            progress_current: self.progress_current,
            progress_total: self.progress_total,
            elapsed_ms: now_ms,
        }
    }

    fn update_visibility(&mut self, now_ms: u64) -> bool {
        let should_be_visible = match self.mode {
            LoadingMode::ColdBoot => true,
            LoadingMode::WakeFromDeepSleep => now_ms >= WAKE_VISIBILITY_DELAY_MS,
        };

        let became_visible = !self.visible && should_be_visible;
        self.visible = should_be_visible;
        became_visible
    }

    fn apply_event(&mut self, event: LoadingEvent) {
        match event {
            LoadingEvent::Begin => {
                self.phase = LoadingPhase::Init;
                self.progress_current = 0;
                self.progress_total = 0;
                self.set_detail("Initializing SD pipeline");
            }
            LoadingEvent::ProbeAttempt {
                speed_index,
                attempt,
                max_attempts,
            } => {
                self.phase = LoadingPhase::ProbeSd;
                self.set_detail_fmt(|buf| {
                    let _ = write!(
                        buf,
                        "{} kHz  attempt {}/{}",
                        hz_to_khz(speed_for_index(speed_index)),
                        attempt,
                        max_attempts
                    );
                });
            }
            LoadingEvent::ProbeSuccess { speed_index } => {
                self.phase = LoadingPhase::ScanCatalog;
                self.set_detail_fmt(|buf| {
                    let _ = write!(
                        buf,
                        "SD ready @ {} kHz",
                        hz_to_khz(speed_for_index(speed_index))
                    );
                });
            }
            LoadingEvent::ProbeRetryTick {
                speed_index,
                attempt,
                remaining_ms,
            } => {
                self.phase = LoadingPhase::ProbeSd;
                self.set_detail_fmt(|buf| {
                    let _ = write!(
                        buf,
                        "Retry {}/{}  {}ms  @ {} kHz",
                        attempt,
                        super::SD_PROBE_ATTEMPTS,
                        remaining_ms,
                        hz_to_khz(speed_for_index(speed_index))
                    );
                });
            }
            LoadingEvent::ScanResult {
                books_dir_found,
                books_total,
            } => {
                self.phase = LoadingPhase::ScanCatalog;
                self.progress_current = 0;
                self.progress_total = books_total;
                if books_dir_found {
                    self.set_detail_fmt(|buf| {
                        let _ = write!(buf, "{} books detected", books_total);
                    });
                } else {
                    self.phase = LoadingPhase::Fallback;
                    self.progress_total = 0;
                    self.set_detail("BOOKS folder missing");
                }
            }
            LoadingEvent::BookText { index, total } => {
                self.phase = LoadingPhase::LoadText;
                self.progress_current = index.min(total);
                self.progress_total = total;
                self.set_detail_fmt(|buf| {
                    let _ = write!(buf, "Book {}/{}  text", index, total.max(1));
                });
            }
            LoadingEvent::BookCover { index, total } => {
                self.phase = LoadingPhase::LoadCover;
                self.progress_current = index.min(total);
                self.progress_total = total;
                self.set_detail_fmt(|buf| {
                    let _ = write!(buf, "Book {}/{}  cover", index, total.max(1));
                });
            }
            LoadingEvent::FallbackNoCatalog => {
                self.phase = LoadingPhase::Fallback;
                self.progress_current = 0;
                self.progress_total = 0;
                self.set_detail("No SD catalog, using defaults");
            }
            LoadingEvent::Finished => {
                self.phase = LoadingPhase::Finalize;
                self.set_detail("Opening reader");
            }
        }
    }

    fn set_detail(&mut self, text: &str) {
        self.detail.clear();
        let _ = self.detail.push_str(text);
    }

    fn set_detail_fmt<F>(&mut self, build: F)
    where
        F: FnOnce(&mut HeaplessString<DETAIL_BYTES>),
    {
        self.detail.clear();
        build(&mut self.detail);
    }
}

fn speed_for_index(speed_index: usize) -> u32 {
    SD_SPI_HZ_CANDIDATES
        .get(speed_index)
        .copied()
        .unwrap_or(SD_SPI_HZ_CANDIDATES[0])
}

fn hz_to_khz(hz: u32) -> u32 {
    hz / 1_000
}
