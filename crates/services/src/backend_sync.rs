pub use domain::sync::SyncStatus;

pub trait BackendSyncService {
    fn status(&self) -> SyncStatus;
    fn request_refresh(&mut self) -> SyncStatus;
}

#[derive(Debug, Default)]
pub struct NoopBackendSyncService;

impl BackendSyncService for NoopBackendSyncService {
    fn status(&self) -> SyncStatus {
        SyncStatus::Disabled
    }

    fn request_refresh(&mut self) -> SyncStatus {
        SyncStatus::Disabled
    }
}
