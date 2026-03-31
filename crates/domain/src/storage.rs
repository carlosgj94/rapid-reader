#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum StorageStatus {
    #[default]
    Uninitialized,
    Available,
    Degraded,
    Unavailable,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum StorageRecoveryStatus {
    #[default]
    Clean,
    Recovered,
    Failed,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct StorageHealth {
    pub status: StorageStatus,
    pub last_recovery: StorageRecoveryStatus,
    pub state_partition_ready: bool,
    pub outbox_partition_ready: bool,
    pub state_free_bytes: u32,
    pub outbox_free_bytes: u32,
}

impl StorageHealth {
    pub const fn new() -> Self {
        Self {
            status: StorageStatus::Uninitialized,
            last_recovery: StorageRecoveryStatus::Clean,
            state_partition_ready: false,
            outbox_partition_ready: false,
            state_free_bytes: 0,
            outbox_free_bytes: 0,
        }
    }

    pub const fn unavailable() -> Self {
        Self {
            status: StorageStatus::Unavailable,
            last_recovery: StorageRecoveryStatus::Failed,
            state_partition_ready: false,
            outbox_partition_ready: false,
            state_free_bytes: 0,
            outbox_free_bytes: 0,
        }
    }

    pub const fn available(
        state_free_bytes: u32,
        outbox_free_bytes: u32,
        last_recovery: StorageRecoveryStatus,
    ) -> Self {
        Self {
            status: StorageStatus::Available,
            last_recovery,
            state_partition_ready: true,
            outbox_partition_ready: true,
            state_free_bytes,
            outbox_free_bytes,
        }
    }
}

impl Default for StorageHealth {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
#[repr(u8)]
pub enum RecordNamespace {
    Settings = 1,
    Network = 2,
    Backend = 3,
    Device = 4,
    Storage = 5,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct RecordKey {
    pub namespace: RecordNamespace,
    pub object_id: u16,
}

impl RecordKey {
    pub const fn new(namespace: RecordNamespace, object_id: u16) -> Self {
        Self {
            namespace,
            object_id,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct QueueKind(pub u16);

impl QueueKind {
    pub const fn new(kind: u16) -> Self {
        Self(kind)
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Default)]
pub struct QueueSeq(pub u64);

impl QueueSeq {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
}
