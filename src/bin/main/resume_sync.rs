use readily_core::{
    app::ReaderApp, content::sd_catalog::SdCatalogSource, input::InputProvider,
    settings::ResumeState,
};

const RESUME_SAVE_DEBOUNCE_MS: u64 = 4_000;
const RESUME_SAVE_MIN_SPACING_MS: u64 = 500;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ResumeFlushReason {
    Debounce,
    PauseOrNavigation,
    Sleep,
}

impl ResumeFlushReason {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Debounce => "debounce",
            Self::PauseOrNavigation => "pause_or_navigation",
            Self::Sleep => "sleep",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ResumeSyncState {
    last_saved: Option<ResumeState>,
    last_seen: Option<ResumeState>,
    dirty_since_ms: Option<u64>,
    last_flush_ms: Option<u64>,
    prev_sleep_eligible: bool,
}

impl ResumeSyncState {
    pub(super) fn new(initial_resume: Option<ResumeState>, initial_sleep_eligible: bool) -> Self {
        Self {
            last_saved: initial_resume,
            last_seen: initial_resume,
            dirty_since_ms: None,
            last_flush_ms: None,
            prev_sleep_eligible: initial_sleep_eligible,
        }
    }

    pub(super) fn observe(
        &mut self,
        current: Option<ResumeState>,
        sleep_eligible_now: bool,
        now_ms: u64,
    ) -> Option<(ResumeState, ResumeFlushReason)> {
        let mut force = None;

        if !self.prev_sleep_eligible
            && sleep_eligible_now
            && let Some(resume) = current
        {
            force = Some((resume, ResumeFlushReason::PauseOrNavigation));
        }
        self.prev_sleep_eligible = sleep_eligible_now;

        match (self.last_seen, current) {
            (Some(previous), Some(now)) => {
                if sleep_eligible_now
                    && previous.selected_book == now.selected_book
                    && !same_resume_paragraph(previous, now)
                    && force.is_none()
                {
                    force = Some((now, ResumeFlushReason::PauseOrNavigation));
                }
                if !sleep_eligible_now && !same_resume_paragraph(previous, now) {
                    self.dirty_since_ms = Some(now_ms);
                }
                self.last_seen = Some(now);
            }
            (None, Some(now)) => {
                if !sleep_eligible_now
                    && self
                        .last_saved
                        .is_none_or(|saved| !same_resume_paragraph(saved, now))
                {
                    self.dirty_since_ms = Some(now_ms);
                }
                self.last_seen = Some(now);
            }
            (_, None) => {
                self.last_seen = None;
                self.dirty_since_ms = None;
            }
        }

        force
    }

    pub(super) fn debounced_due(&self, now_ms: u64) -> Option<ResumeState> {
        let dirty_since = self.dirty_since_ms?;
        let resume = self.last_seen?;
        if now_ms.saturating_sub(dirty_since) < RESUME_SAVE_DEBOUNCE_MS {
            return None;
        }
        if self
            .last_saved
            .is_some_and(|saved| same_resume_paragraph(saved, resume))
        {
            return None;
        }
        Some(resume)
    }

    pub(super) fn can_flush_now(&self, now_ms: u64) -> bool {
        self.last_flush_ms
            .is_none_or(|last| now_ms.saturating_sub(last) >= RESUME_SAVE_MIN_SPACING_MS)
    }

    pub(super) fn mark_saved(&mut self, saved: ResumeState, now_ms: u64) {
        self.last_saved = Some(saved);
        self.last_flush_ms = Some(now_ms);
        if self
            .last_seen
            .is_some_and(|seen| same_resume_paragraph(seen, saved))
        {
            self.dirty_since_ms = None;
        }
    }
}

fn same_resume_paragraph(a: ResumeState, b: ResumeState) -> bool {
    a.selected_book == b.selected_book
        && a.chapter_index == b.chapter_index
        && a.paragraph_in_chapter == b.paragraph_in_chapter
}

pub(super) fn apply_resume_chapter_hint<IN>(
    app: &mut ReaderApp<SdCatalogSource, IN>,
    resume: ResumeState,
) where
    IN: InputProvider,
{
    let hinted_total = resume.chapter_index.saturating_add(1).max(1);
    let _ = app.with_content_mut(|content| {
        content.set_catalog_stream_chapter_hint(
            resume.selected_book,
            resume.chapter_index,
            hinted_total,
        )
    });
}
