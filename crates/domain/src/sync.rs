#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum SyncStatus {
    Disabled,
    WaitingForNetwork,
    RefreshingSession,
    VerifyingIdentity,
    SyncingContent,
    Ready,
    TransportFailed,
    AuthFailed,
    #[default]
    Uninitialized,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct StartupSyncProgress {
    pub completed_queries: u8,
    pub total_queries: u8,
}

impl StartupSyncProgress {
    pub const fn new(completed_queries: u8, total_queries: u8) -> Self {
        Self {
            completed_queries,
            total_queries,
        }
    }

    pub const fn clamped_completed(self) -> u8 {
        if self.completed_queries > self.total_queries {
            self.total_queries
        } else {
            self.completed_queries
        }
    }
}

impl SyncStatus {
    pub const fn is_active(self) -> bool {
        matches!(
            self,
            Self::WaitingForNetwork
                | Self::RefreshingSession
                | Self::VerifyingIdentity
                | Self::SyncingContent
        )
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct SyncState {
    pub status: SyncStatus,
    pub spinner_phase: u8,
}

impl SyncState {
    pub const fn new() -> Self {
        Self {
            status: SyncStatus::Uninitialized,
            spinner_phase: 0,
        }
    }

    pub fn set_status(&mut self, status: SyncStatus) {
        self.status = status;
        if !status.is_active() {
            self.spinner_phase = 0;
        }
    }

    pub fn advance_spinner(&mut self) {
        if self.status.is_active() {
            self.spinner_phase = (self.spinner_phase + 1) % 4;
        } else {
            self.spinner_phase = 0;
        }
    }

    pub const fn shows_dashboard_indicator(self) -> bool {
        self.status.is_active()
    }
}

impl Default for SyncState {
    fn default() -> Self {
        Self::new()
    }
}
