#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum SyncStatus {
    #[default]
    Uninitialized,
    Idle,
}

pub trait BackendSyncService {
    fn status(&self) -> SyncStatus;
    fn request_refresh(&mut self) -> SyncStatus;
}

#[derive(Debug, Default)]
pub struct NoopBackendSyncService;

impl BackendSyncService for NoopBackendSyncService {
    fn status(&self) -> SyncStatus {
        SyncStatus::Uninitialized
    }

    fn request_refresh(&mut self) -> SyncStatus {
        SyncStatus::Idle
    }
}
