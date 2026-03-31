use domain::storage::{
    QueueKind, QueueSeq, RecordKey, StorageHealth, StorageRecoveryStatus, StorageStatus,
};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum StorageCodecError {
    #[default]
    InvalidData,
    BufferTooSmall,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum StorageError {
    #[default]
    Unavailable,
    PartitionMissing,
    InvalidPartition,
    CorruptData,
    PayloadTooLarge,
    PartitionFull,
    UnsupportedLayout,
    TooManyKeys,
    FlashFailure,
    CodecFailure,
}

impl embedded_storage::nor_flash::NorFlashError for StorageError {
    fn kind(&self) -> embedded_storage::nor_flash::NorFlashErrorKind {
        match self {
            StorageError::PartitionFull => {
                embedded_storage::nor_flash::NorFlashErrorKind::OutOfBounds
            }
            _ => embedded_storage::nor_flash::NorFlashErrorKind::Other,
        }
    }
}

pub trait RecordCodec {
    type Value;

    const KEY: RecordKey;
    const SCHEMA_VERSION: u16;
    const MAX_ENCODED_LEN: usize;

    fn encode(value: &Self::Value, out: &mut [u8]) -> Result<usize, StorageCodecError>;
    fn decode(bytes: &[u8]) -> Result<Self::Value, StorageCodecError>;
}

pub trait QueueCodec {
    type Value;

    const KIND: QueueKind;
    const SCHEMA_VERSION: u16;
    const MAX_ENCODED_LEN: usize;

    fn encode(value: &Self::Value, out: &mut [u8]) -> Result<usize, StorageCodecError>;
    fn decode(bytes: &[u8]) -> Result<Self::Value, StorageCodecError>;
}

pub trait StorageService {
    fn health(&self) -> StorageHealth;

    fn status(&self) -> StorageStatus {
        self.health().status
    }

    async fn read_record<C: RecordCodec>(&mut self) -> Result<Option<C::Value>, StorageError>;

    async fn write_record<C: RecordCodec>(&mut self, value: &C::Value) -> Result<(), StorageError>;

    async fn delete_record<C: RecordCodec>(&mut self) -> Result<(), StorageError>;

    async fn enqueue<Q: QueueCodec>(&mut self, value: &Q::Value) -> Result<QueueSeq, StorageError>;

    async fn peek<Q: QueueCodec>(&mut self) -> Result<Option<(QueueSeq, Q::Value)>, StorageError>;

    async fn ack<Q: QueueCodec>(&mut self, seq: QueueSeq) -> Result<(), StorageError>;
}

#[derive(Debug, Default)]
pub struct NoopStorageService;

impl NoopStorageService {
    pub const fn new() -> Self {
        Self
    }
}

impl StorageService for NoopStorageService {
    fn health(&self) -> StorageHealth {
        StorageHealth {
            status: StorageStatus::Unavailable,
            last_recovery: StorageRecoveryStatus::Failed,
            state_partition_ready: false,
            outbox_partition_ready: false,
            state_free_bytes: 0,
            outbox_free_bytes: 0,
        }
    }

    async fn read_record<C: RecordCodec>(&mut self) -> Result<Option<C::Value>, StorageError> {
        Err(StorageError::Unavailable)
    }

    async fn write_record<C: RecordCodec>(
        &mut self,
        _value: &C::Value,
    ) -> Result<(), StorageError> {
        Err(StorageError::Unavailable)
    }

    async fn delete_record<C: RecordCodec>(&mut self) -> Result<(), StorageError> {
        Err(StorageError::Unavailable)
    }

    async fn enqueue<Q: QueueCodec>(
        &mut self,
        _value: &Q::Value,
    ) -> Result<QueueSeq, StorageError> {
        Err(StorageError::Unavailable)
    }

    async fn peek<Q: QueueCodec>(&mut self) -> Result<Option<(QueueSeq, Q::Value)>, StorageError> {
        Err(StorageError::Unavailable)
    }

    async fn ack<Q: QueueCodec>(&mut self, _seq: QueueSeq) -> Result<(), StorageError> {
        Err(StorageError::Unavailable)
    }
}
