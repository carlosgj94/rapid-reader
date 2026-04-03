use core::cmp::Ordering;

use ::domain::{
    settings::{AppearanceMode, PersistedSettings, TopicPreferences},
    storage::{
        QueueKind, QueueSeq, RecordKey, RecordNamespace, StorageHealth, StorageRecoveryStatus,
        StorageStatus,
    },
};
use ::services::storage::{
    QueueCodec, RecordCodec, StorageCodecError, StorageError, StorageService,
};
use embedded_storage::nor_flash::{ErrorType, NorFlash, ReadNorFlash};
use esp_bootloader_esp_idf::partitions::{PARTITION_TABLE_MAX_LEN, read_partition_table};
use esp_hal::peripherals::FLASH as Flash;
use esp_storage::FlashStorage;
use log::info;

pub const STATE_PARTITION_LABEL: &str = "motif_state";
pub const OUTBOX_PARTITION_LABEL: &str = "motif_outbox";
pub const SETTINGS_RECORD_KEY: RecordKey = RecordKey::new(RecordNamespace::Settings, 1);
pub const BACKEND_CREDENTIAL_RECORD_KEY: RecordKey = RecordKey::new(RecordNamespace::Backend, 1);
pub const BACKEND_REFRESH_TOKEN_MAX_LEN: usize = 320;

const SLOT_SIZE: usize = 512;
const SLOT_PAYLOAD_MAX: usize = SLOT_SIZE - SLOT_HEADER_LEN - SLOT_COMMIT_LEN;
const SLOT_HEADER_LEN: usize = 32;
const SLOT_COMMIT_LEN: usize = 4;
const SLOT_COMMIT_OFFSET: usize = SLOT_SIZE - SLOT_COMMIT_LEN;
const SLOT_MAGIC: u32 = 0x4D54_5243;
const SLOT_COMMIT_MAGIC: u32 = 0xC0DE_CAFE;
const BANK_HEADER_LEN: usize = 64;
const BANK_HEADER_COMMIT_OFFSET: usize = BANK_HEADER_LEN - 4;
const BANK_MAGIC: u32 = 0x4D54_424B;
const BANK_COMMIT_MAGIC: u32 = 0xB16B_C0DE;
const FORMAT_VERSION: u16 = 1;
const MAX_STATE_KEYS: usize = 32;

#[derive(Debug)]
pub struct PlatformStorageService<'d> {
    inner: InternalStorage<FlashStorage<'d>>,
}

impl<'d> PlatformStorageService<'d> {
    pub fn mount(flash: Flash<'d>) -> Self {
        let mut flash = FlashStorage::new(flash).multicore_auto_park();
        let mut partition_table = [0u8; PARTITION_TABLE_MAX_LEN];
        let state =
            find_partition_geometry(&mut flash, &mut partition_table, STATE_PARTITION_LABEL);
        let outbox =
            find_partition_geometry(&mut flash, &mut partition_table, OUTBOX_PARTITION_LABEL);

        if let Err(err) = state {
            info!(
                "storage mount failed: partition label '{}' unavailable: {:?}",
                STATE_PARTITION_LABEL, err
            );
        }
        if let Err(err) = outbox {
            info!(
                "storage mount failed: partition label '{}' unavailable: {:?}",
                OUTBOX_PARTITION_LABEL, err
            );
        }

        let inner = match (state, outbox) {
            (Ok(state), Ok(outbox)) => InternalStorage::from_geometries(flash, state, outbox),
            (Err(err), _) | (_, Err(err)) => InternalStorage::unavailable_with_error(flash, err),
        };

        Self { inner }
    }

    pub fn health_snapshot(&self) -> StorageHealth {
        self.inner.health()
    }

    pub fn read_persisted_settings_sync(
        &mut self,
    ) -> Result<Option<PersistedSettings>, StorageError> {
        self.read_record_sync::<PersistedSettingsCodec>()
    }

    pub fn write_persisted_settings_sync(
        &mut self,
        settings: &PersistedSettings,
    ) -> Result<(), StorageError> {
        self.write_record_sync::<PersistedSettingsCodec>(settings)
    }

    pub fn read_backend_credential_sync(
        &mut self,
    ) -> Result<Option<BackendCredential>, StorageError> {
        self.read_record_sync::<BackendCredentialCodec>()
    }

    pub fn write_backend_credential_sync(
        &mut self,
        credential: &BackendCredential,
    ) -> Result<(), StorageError> {
        self.write_record_sync::<BackendCredentialCodec>(credential)
    }

    pub fn read_record_sync<C: RecordCodec>(&mut self) -> Result<Option<C::Value>, StorageError> {
        self.inner.read_record_sync::<C>()
    }

    pub fn write_record_sync<C: RecordCodec>(
        &mut self,
        value: &C::Value,
    ) -> Result<(), StorageError> {
        self.inner.write_record_sync::<C>(value)
    }

    pub fn delete_record_sync<C: RecordCodec>(&mut self) -> Result<(), StorageError> {
        self.inner.delete_record_sync::<C>()
    }

    pub fn enqueue_sync<Q: QueueCodec>(
        &mut self,
        value: &Q::Value,
    ) -> Result<QueueSeq, StorageError> {
        self.inner.enqueue_sync::<Q>(value)
    }

    pub fn peek_sync<Q: QueueCodec>(
        &mut self,
    ) -> Result<Option<(QueueSeq, Q::Value)>, StorageError> {
        self.inner.peek_sync::<Q>()
    }

    pub fn ack_sync<Q: QueueCodec>(&mut self, seq: QueueSeq) -> Result<(), StorageError> {
        self.inner.ack_sync::<Q>(seq)
    }
}

impl StorageService for PlatformStorageService<'_> {
    fn health(&self) -> StorageHealth {
        self.inner.health()
    }

    async fn read_record<C: RecordCodec>(&mut self) -> Result<Option<C::Value>, StorageError> {
        self.read_record_sync::<C>()
    }

    async fn write_record<C: RecordCodec>(&mut self, value: &C::Value) -> Result<(), StorageError> {
        self.write_record_sync::<C>(value)
    }

    async fn delete_record<C: RecordCodec>(&mut self) -> Result<(), StorageError> {
        self.delete_record_sync::<C>()
    }

    async fn enqueue<Q: QueueCodec>(&mut self, value: &Q::Value) -> Result<QueueSeq, StorageError> {
        self.enqueue_sync::<Q>(value)
    }

    async fn peek<Q: QueueCodec>(&mut self) -> Result<Option<(QueueSeq, Q::Value)>, StorageError> {
        self.peek_sync::<Q>()
    }

    async fn ack<Q: QueueCodec>(&mut self, seq: QueueSeq) -> Result<(), StorageError> {
        self.ack_sync::<Q>(seq)
    }
}

#[derive(Debug)]
struct InternalStorage<F> {
    flash: F,
    partitions: Option<MountedPartitions>,
    health: StorageHealth,
}

impl<F> InternalStorage<F>
where
    F: NorFlash + ReadNorFlash,
{
    fn from_geometries(
        flash: F,
        state_geometry: PartitionGeometry,
        outbox_geometry: PartitionGeometry,
    ) -> Self {
        let mut storage = Self {
            flash,
            partitions: Some(MountedPartitions {
                state: MountedPartition::new(state_geometry, PartitionKind::State),
                outbox: MountedPartition::new(outbox_geometry, PartitionKind::Outbox),
            }),
            health: StorageHealth::new(),
        };

        let mut recovery = StorageRecoveryStatus::Clean;
        if storage
            .mount_partition(PartitionSelector::State, &mut recovery)
            .and_then(|_| storage.mount_partition(PartitionSelector::Outbox, &mut recovery))
            .and_then(|_| storage.refresh_health_with_recovery(recovery))
            .is_err()
        {
            storage.partitions = None;
            storage.health = StorageHealth::unavailable();
        }

        storage
    }

    fn unavailable_with_error(flash: F, error: StorageError) -> Self {
        let mut health = StorageHealth::unavailable();
        if matches!(error, StorageError::PartitionMissing) {
            health.last_recovery = StorageRecoveryStatus::Failed;
        }

        Self {
            flash,
            partitions: None,
            health,
        }
    }

    fn health(&self) -> StorageHealth {
        self.health
    }

    fn read_record_sync<C: RecordCodec>(&mut self) -> Result<Option<C::Value>, StorageError> {
        let latest = self.with_partition(PartitionSelector::State, |flash, mounted| {
            read_latest_state_record::<_, C>(flash, mounted)
        })?;

        match latest {
            Some(payload) => C::decode(payload.as_slice()).map(Some).map_err(codec_error),
            None => Ok(None),
        }
    }

    fn write_record_sync<C: RecordCodec>(&mut self, value: &C::Value) -> Result<(), StorageError> {
        let mut encoded = [0xFF; SLOT_PAYLOAD_MAX];
        let encoded_len = C::encode(value, &mut encoded).map_err(codec_error)?;
        if encoded_len > SLOT_PAYLOAD_MAX || C::MAX_ENCODED_LEN > SLOT_PAYLOAD_MAX {
            return Err(StorageError::PayloadTooLarge);
        }

        self.with_partition(PartitionSelector::State, |flash, mounted| {
            append_state_record(
                flash,
                mounted,
                C::KEY,
                C::SCHEMA_VERSION,
                EntryKind::StatePut,
                &encoded[..encoded_len],
            )
        })?;
        self.refresh_health()?;
        Ok(())
    }

    fn delete_record_sync<C: RecordCodec>(&mut self) -> Result<(), StorageError> {
        self.with_partition(PartitionSelector::State, |flash, mounted| {
            append_state_record(
                flash,
                mounted,
                C::KEY,
                C::SCHEMA_VERSION,
                EntryKind::StateDelete,
                &[],
            )
        })?;
        self.refresh_health()?;
        Ok(())
    }

    fn enqueue_sync<Q: QueueCodec>(&mut self, value: &Q::Value) -> Result<QueueSeq, StorageError> {
        let mut encoded = [0xFF; SLOT_PAYLOAD_MAX];
        let encoded_len = Q::encode(value, &mut encoded).map_err(codec_error)?;
        if encoded_len > SLOT_PAYLOAD_MAX || Q::MAX_ENCODED_LEN > SLOT_PAYLOAD_MAX {
            return Err(StorageError::PayloadTooLarge);
        }

        let seq = self.with_partition(PartitionSelector::Outbox, |flash, mounted| {
            append_queue_record(
                flash,
                mounted,
                Q::KIND,
                Q::SCHEMA_VERSION,
                EntryKind::QueueEnqueue,
                None,
                &encoded[..encoded_len],
            )
        })?;
        self.refresh_health()?;
        Ok(seq)
    }

    fn peek_sync<Q: QueueCodec>(&mut self) -> Result<Option<(QueueSeq, Q::Value)>, StorageError> {
        let queued = self.with_partition(PartitionSelector::Outbox, |flash, mounted| {
            peek_queue_record::<_, Q>(flash, mounted)
        })?;

        match queued {
            Some((seq, payload)) => Q::decode(payload.as_slice())
                .map(|value| Some((seq, value)))
                .map_err(codec_error),
            None => Ok(None),
        }
    }

    fn ack_sync<Q: QueueCodec>(&mut self, seq: QueueSeq) -> Result<(), StorageError> {
        self.with_partition(PartitionSelector::Outbox, |flash, mounted| {
            if is_queue_seq_acked(flash, mounted, Q::KIND, seq.0)? {
                return Ok(());
            }

            append_queue_record(
                flash,
                mounted,
                Q::KIND,
                Q::SCHEMA_VERSION,
                EntryKind::QueueAck,
                Some(seq),
                &[],
            )
            .map(|_| ())
        })?;
        self.refresh_health()?;
        Ok(())
    }

    fn mount_partition(
        &mut self,
        selector: PartitionSelector,
        recovery: &mut StorageRecoveryStatus,
    ) -> Result<(), StorageError> {
        self.with_partition(selector, |flash, mounted| {
            let bank_state = mount_bank_pair(flash, mounted)?;
            mounted.active_bank = bank_state.active_bank;
            mounted.active_generation = bank_state.active_generation;
            if bank_state.recovered {
                *recovery = StorageRecoveryStatus::Recovered;
            }
            Ok(())
        })
    }

    fn refresh_health(&mut self) -> Result<(), StorageError> {
        self.refresh_health_with_recovery(self.health.last_recovery)
    }

    fn refresh_health_with_recovery(
        &mut self,
        last_recovery: StorageRecoveryStatus,
    ) -> Result<(), StorageError> {
        let state_free_bytes = self
            .with_partition(PartitionSelector::State, |flash, mounted| {
                free_bytes_in_active_bank(flash, mounted)
            })?;
        let outbox_free_bytes = self
            .with_partition(PartitionSelector::Outbox, |flash, mounted| {
                free_bytes_in_active_bank(flash, mounted)
            })?;

        self.health = StorageHealth::available(state_free_bytes, outbox_free_bytes, last_recovery);

        if state_free_bytes == 0 || outbox_free_bytes == 0 {
            self.health.status = StorageStatus::Degraded;
        }

        Ok(())
    }

    fn with_partition<R>(
        &mut self,
        selector: PartitionSelector,
        f: impl FnOnce(&mut FlashSlice<'_, F>, &mut MountedPartition) -> Result<R, StorageError>,
    ) -> Result<R, StorageError> {
        let partitions = self.partitions.as_mut().ok_or(StorageError::Unavailable)?;
        let flash = &mut self.flash;

        match selector {
            PartitionSelector::State => {
                let mut slice = FlashSlice::new(flash, partitions.state.geometry);
                f(&mut slice, &mut partitions.state)
            }
            PartitionSelector::Outbox => {
                let mut slice = FlashSlice::new(flash, partitions.outbox.geometry);
                f(&mut slice, &mut partitions.outbox)
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct MountedPartitions {
    state: MountedPartition,
    outbox: MountedPartition,
}

#[derive(Debug, Clone, Copy)]
struct MountedPartition {
    geometry: PartitionGeometry,
    kind: PartitionKind,
    active_bank: u8,
    active_generation: u64,
}

impl MountedPartition {
    const fn new(geometry: PartitionGeometry, kind: PartitionKind) -> Self {
        Self {
            geometry,
            kind,
            active_bank: 0,
            active_generation: 0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct PartitionGeometry {
    offset: u32,
    length: u32,
}

impl PartitionGeometry {
    const fn new(offset: u32, length: u32) -> Self {
        Self { offset, length }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum PartitionKind {
    State = 1,
    Outbox = 2,
}

#[derive(Debug, Clone, Copy)]
enum PartitionSelector {
    State,
    Outbox,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum EntryKind {
    StatePut = 1,
    StateDelete = 2,
    QueueEnqueue = 3,
    QueueAck = 4,
}

impl EntryKind {
    fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::StatePut),
            2 => Some(Self::StateDelete),
            3 => Some(Self::QueueEnqueue),
            4 => Some(Self::QueueAck),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct BankMountState {
    active_bank: u8,
    active_generation: u64,
    recovered: bool,
}

#[derive(Debug, Clone, Copy)]
struct BankGeometry {
    offset: u32,
    length: u32,
}

#[derive(Debug, Clone, Copy)]
struct BankHeader {
    generation: u64,
    kind: PartitionKind,
    committed: bool,
}

#[derive(Debug, Clone, Copy)]
struct ParsedEntry {
    kind: EntryKind,
    namespace: u8,
    object_id: u16,
    schema_version: u16,
    payload_len: usize,
    sequence: u64,
}

#[derive(Debug, Clone, Copy)]
struct PayloadBuffer {
    bytes: [u8; SLOT_PAYLOAD_MAX],
    len: usize,
}

impl PayloadBuffer {
    fn from_slice(bytes: &[u8]) -> Result<Self, StorageError> {
        if bytes.len() > SLOT_PAYLOAD_MAX {
            return Err(StorageError::PayloadTooLarge);
        }

        let mut buffer = [0u8; SLOT_PAYLOAD_MAX];
        buffer[..bytes.len()].copy_from_slice(bytes);
        Ok(Self {
            bytes: buffer,
            len: bytes.len(),
        })
    }

    fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

#[derive(Debug, Clone, Copy)]
struct LatestStateEntry {
    key: RecordKey,
    sequence: u64,
    deleted: bool,
    slot_index: u16,
}

#[derive(Debug)]
struct FlashSlice<'a, F> {
    flash: &'a mut F,
    geometry: PartitionGeometry,
}

impl<'a, F> FlashSlice<'a, F> {
    fn new(flash: &'a mut F, geometry: PartitionGeometry) -> Self {
        Self { flash, geometry }
    }

    fn absolute_range(&self) -> core::ops::Range<u32> {
        self.geometry.offset..self.geometry.offset + self.geometry.length
    }

    fn in_bounds(&self, offset: u32, len: usize) -> bool {
        let start = self.geometry.offset.saturating_add(offset);
        let end = start.saturating_add(len as u32);
        start >= self.geometry.offset && end <= self.geometry.offset + self.geometry.length
    }
}

impl<F> ErrorType for FlashSlice<'_, F> {
    type Error = StorageError;
}

impl<F> ReadNorFlash for FlashSlice<'_, F>
where
    F: ReadNorFlash,
{
    const READ_SIZE: usize = F::READ_SIZE;

    fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
        if !self.in_bounds(offset, bytes.len()) {
            return Err(StorageError::InvalidPartition);
        }

        self.flash
            .read(self.geometry.offset + offset, bytes)
            .map_err(|_| StorageError::FlashFailure)
    }

    fn capacity(&self) -> usize {
        self.geometry.length as usize
    }
}

impl<F> NorFlash for FlashSlice<'_, F>
where
    F: NorFlash,
{
    const WRITE_SIZE: usize = F::WRITE_SIZE;
    const ERASE_SIZE: usize = F::ERASE_SIZE;

    fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
        if from > to || !self.in_bounds(from, (to - from) as usize) {
            return Err(StorageError::InvalidPartition);
        }

        self.flash
            .erase(self.geometry.offset + from, self.geometry.offset + to)
            .map_err(|_| StorageError::FlashFailure)
    }

    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
        if !self.in_bounds(offset, bytes.len()) {
            return Err(StorageError::InvalidPartition);
        }

        self.flash
            .write(self.geometry.offset + offset, bytes)
            .map_err(|_| StorageError::FlashFailure)
    }
}

impl<F> FlashSlice<'_, F>
where
    F: ReadNorFlash + NorFlash,
{
    fn read_into(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), StorageError> {
        ReadNorFlash::read(self, offset, bytes)
    }
}

fn find_partition_geometry(
    flash: &mut FlashStorage<'_>,
    buffer: &mut [u8; PARTITION_TABLE_MAX_LEN],
    label: &str,
) -> Result<PartitionGeometry, StorageError> {
    let partition_table =
        read_partition_table(flash, buffer).map_err(|_| StorageError::PartitionMissing)?;
    let entry = partition_table
        .iter()
        .find(|entry| entry.label_as_str() == label)
        .ok_or(StorageError::PartitionMissing)?;

    Ok(PartitionGeometry::new(entry.offset(), entry.len()))
}

fn mount_bank_pair<F>(
    flash: &mut FlashSlice<'_, F>,
    mounted: &mut MountedPartition,
) -> Result<BankMountState, StorageError>
where
    F: NorFlash + ReadNorFlash,
{
    let (bank0, bank1) = bank_geometries(mounted.geometry)?;
    let header0 = read_bank_header(flash, bank0)?;
    let header1 = read_bank_header(flash, bank1)?;

    let selected = match (header0, header1) {
        (Some(a), Some(b))
            if a.committed && b.committed && a.kind == mounted.kind && b.kind == mounted.kind =>
        {
            match a.generation.cmp(&b.generation) {
                Ordering::Greater => Some((0u8, a.generation)),
                Ordering::Less => Some((1u8, b.generation)),
                Ordering::Equal => Some((0u8, a.generation)),
            }
        }
        (Some(a), _) if a.committed && a.kind == mounted.kind => Some((0u8, a.generation)),
        (_, Some(b)) if b.committed && b.kind == mounted.kind => Some((1u8, b.generation)),
        _ => None,
    };

    match selected {
        Some((active_bank, generation)) => Ok(BankMountState {
            active_bank,
            active_generation: generation,
            recovered: false,
        }),
        None => {
            erase_bank(flash, bank0)?;
            write_bank_header_staged(flash, bank0, mounted.kind, 1)?;
            commit_bank_header(flash, bank0)?;
            Ok(BankMountState {
                active_bank: 0,
                active_generation: 1,
                recovered: true,
            })
        }
    }
}

fn bank_geometries(
    geometry: PartitionGeometry,
) -> Result<(BankGeometry, BankGeometry), StorageError> {
    let erase_size = FlashSlice::<DummyFlash>::ERASE_SIZE as u32;
    let half = geometry.length / 2;
    let bank_len = (half / erase_size) * erase_size;

    if bank_len < erase_size * 2 {
        return Err(StorageError::InvalidPartition);
    }

    Ok((
        BankGeometry {
            offset: 0,
            length: bank_len,
        },
        BankGeometry {
            offset: bank_len,
            length: bank_len,
        },
    ))
}

fn read_bank_header<F>(
    flash: &mut FlashSlice<'_, F>,
    bank: BankGeometry,
) -> Result<Option<BankHeader>, StorageError>
where
    F: NorFlash + ReadNorFlash,
{
    let mut header = [0u8; BANK_HEADER_LEN];
    flash.read_into(bank.offset, &mut header)?;

    if is_blank(&header) {
        return Ok(None);
    }

    if read_u32(&header, 0) != BANK_MAGIC {
        return Ok(None);
    }

    if read_u16(&header, 4) != FORMAT_VERSION {
        return Ok(None);
    }

    let kind = match header[6] {
        1 => PartitionKind::State,
        2 => PartitionKind::Outbox,
        _ => return Ok(None),
    };

    let committed = read_u32(&header, BANK_HEADER_COMMIT_OFFSET) == BANK_COMMIT_MAGIC;

    Ok(Some(BankHeader {
        generation: read_u64(&header, 8),
        kind,
        committed,
    }))
}

fn write_bank_header_staged<F>(
    flash: &mut FlashSlice<'_, F>,
    bank: BankGeometry,
    kind: PartitionKind,
    generation: u64,
) -> Result<(), StorageError>
where
    F: NorFlash + ReadNorFlash,
{
    let mut header = [0xFF; BANK_HEADER_LEN];
    write_u32(&mut header, 0, BANK_MAGIC);
    write_u16(&mut header, 4, FORMAT_VERSION);
    header[6] = kind as u8;
    write_u64(&mut header, 8, generation);
    flash.write(bank.offset, &header[..BANK_HEADER_COMMIT_OFFSET])?;
    Ok(())
}

fn commit_bank_header<F>(
    flash: &mut FlashSlice<'_, F>,
    bank: BankGeometry,
) -> Result<(), StorageError>
where
    F: NorFlash + ReadNorFlash,
{
    let commit = BANK_COMMIT_MAGIC.to_le_bytes();
    flash.write(bank.offset + BANK_HEADER_COMMIT_OFFSET as u32, &commit)?;
    Ok(())
}

fn erase_bank<F>(flash: &mut FlashSlice<'_, F>, bank: BankGeometry) -> Result<(), StorageError>
where
    F: NorFlash + ReadNorFlash,
{
    flash.erase(bank.offset, bank.offset + bank.length)
}

fn free_bytes_in_active_bank<F>(
    flash: &mut FlashSlice<'_, F>,
    mounted: &MountedPartition,
) -> Result<u32, StorageError>
where
    F: NorFlash + ReadNorFlash,
{
    let bank = active_bank_geometry(mounted)?;
    let blank_slots = count_blank_slots(flash, bank)?;
    Ok(blank_slots as u32 * SLOT_SIZE as u32)
}

fn append_state_record<F>(
    flash: &mut FlashSlice<'_, F>,
    mounted: &mut MountedPartition,
    key: RecordKey,
    schema_version: u16,
    kind: EntryKind,
    payload: &[u8],
) -> Result<(), StorageError>
where
    F: NorFlash + ReadNorFlash,
{
    if payload.len() > SLOT_PAYLOAD_MAX {
        return Err(StorageError::PayloadTooLarge);
    }

    let next_seq = next_state_generation(flash, mounted)?.saturating_add(1);
    let slot = build_slot(
        kind,
        key.namespace as u8,
        key.object_id,
        schema_version,
        next_seq,
        payload,
    )?;

    if !append_slot(flash, mounted, &slot)? {
        compact_state_partition(flash, mounted)?;
        if !append_slot(flash, mounted, &slot)? {
            return Err(StorageError::PartitionFull);
        }
    }

    Ok(())
}

fn append_queue_record<F>(
    flash: &mut FlashSlice<'_, F>,
    mounted: &mut MountedPartition,
    kind: QueueKind,
    schema_version: u16,
    entry_kind: EntryKind,
    seq: Option<QueueSeq>,
    payload: &[u8],
) -> Result<QueueSeq, StorageError>
where
    F: NorFlash + ReadNorFlash,
{
    if payload.len() > SLOT_PAYLOAD_MAX {
        return Err(StorageError::PayloadTooLarge);
    }

    let next_seq = seq.unwrap_or(QueueSeq(
        next_queue_sequence(flash, mounted)?.saturating_add(1),
    ));
    let slot = build_slot(entry_kind, 0, kind.0, schema_version, next_seq.0, payload)?;

    if !append_slot(flash, mounted, &slot)? {
        compact_outbox_partition(flash, mounted)?;
        if !append_slot(flash, mounted, &slot)? {
            return Err(StorageError::PartitionFull);
        }
    }

    Ok(next_seq)
}

fn append_slot<F>(
    flash: &mut FlashSlice<'_, F>,
    mounted: &mut MountedPartition,
    slot: &[u8; SLOT_SIZE],
) -> Result<bool, StorageError>
where
    F: NorFlash + ReadNorFlash,
{
    let bank = active_bank_geometry(mounted)?;
    let Some(index) = first_blank_slot(flash, bank)? else {
        return Ok(false);
    };

    write_slot(flash, bank, index, slot)?;
    Ok(true)
}

fn compact_state_partition<F>(
    flash: &mut FlashSlice<'_, F>,
    mounted: &mut MountedPartition,
) -> Result<(), StorageError>
where
    F: NorFlash + ReadNorFlash,
{
    let active = active_bank_geometry(mounted)?;
    let target = inactive_bank_geometry(mounted)?;
    erase_bank(flash, target)?;
    write_bank_header_staged(flash, target, mounted.kind, mounted.active_generation + 1)?;

    let mut latest = [None; MAX_STATE_KEYS];
    collect_latest_state_entries(flash, active, &mut latest)?;
    sort_state_entries_by_sequence(&mut latest);

    let mut write_index = 0u16;
    let mut slot_buffer = [0u8; SLOT_SIZE];
    for entry in latest.into_iter().flatten() {
        if entry.deleted {
            continue;
        }

        read_slot(flash, active, entry.slot_index, &mut slot_buffer)?;
        write_slot(flash, target, write_index, &slot_buffer)?;
        write_index = write_index.saturating_add(1);
    }

    commit_bank_header(flash, target)?;
    mounted.active_bank ^= 1;
    mounted.active_generation = mounted.active_generation.saturating_add(1);
    Ok(())
}

fn compact_outbox_partition<F>(
    flash: &mut FlashSlice<'_, F>,
    mounted: &mut MountedPartition,
) -> Result<(), StorageError>
where
    F: NorFlash + ReadNorFlash,
{
    let active = active_bank_geometry(mounted)?;
    let target = inactive_bank_geometry(mounted)?;
    erase_bank(flash, target)?;
    write_bank_header_staged(flash, target, mounted.kind, mounted.active_generation + 1)?;

    let mut write_index = 0u16;
    let mut slot_buffer = [0u8; SLOT_SIZE];
    let slots = slots_per_bank(active)?;
    for slot_index in 0..slots {
        read_slot(flash, active, slot_index, &mut slot_buffer)?;
        let ParsedSlot::Valid(entry) = parse_slot(&slot_buffer)? else {
            continue;
        };
        if entry.kind != EntryKind::QueueEnqueue {
            continue;
        }

        if !is_queue_seq_acked_by_kind(flash, active, QueueKind(entry.object_id), entry.sequence)? {
            write_slot(flash, target, write_index, &slot_buffer)?;
            write_index = write_index.saturating_add(1);
        }
    }

    commit_bank_header(flash, target)?;
    mounted.active_bank ^= 1;
    mounted.active_generation = mounted.active_generation.saturating_add(1);
    Ok(())
}

fn read_latest_state_record<F, C: RecordCodec>(
    flash: &mut FlashSlice<'_, F>,
    mounted: &MountedPartition,
) -> Result<Option<PayloadBuffer>, StorageError>
where
    F: NorFlash + ReadNorFlash,
{
    let bank = active_bank_geometry(mounted)?;
    let slots = slots_per_bank(bank)?;
    let mut best_seq = None;
    let mut best_payload: Option<PayloadBuffer> = None;
    let mut slot_buffer = [0u8; SLOT_SIZE];

    for slot_index in 0..slots {
        read_slot(flash, bank, slot_index, &mut slot_buffer)?;
        let ParsedSlot::Valid(entry) = parse_slot(&slot_buffer)? else {
            continue;
        };
        if entry.kind != EntryKind::StatePut && entry.kind != EntryKind::StateDelete {
            continue;
        }

        if entry.namespace != C::KEY.namespace as u8 || entry.object_id != C::KEY.object_id {
            continue;
        }

        if entry.schema_version != C::SCHEMA_VERSION {
            return Err(StorageError::CorruptData);
        }

        if best_seq.is_none_or(|seq| entry.sequence > seq) {
            best_seq = Some(entry.sequence);
            best_payload = match entry.kind {
                EntryKind::StatePut => Some(PayloadBuffer::from_slice(
                    &slot_buffer[SLOT_HEADER_LEN..SLOT_HEADER_LEN + entry.payload_len],
                )?),
                EntryKind::StateDelete => None,
                _ => None,
            };
        }
    }

    Ok(best_payload)
}

fn peek_queue_record<F, Q: QueueCodec>(
    flash: &mut FlashSlice<'_, F>,
    mounted: &MountedPartition,
) -> Result<Option<(QueueSeq, PayloadBuffer)>, StorageError>
where
    F: NorFlash + ReadNorFlash,
{
    let bank = active_bank_geometry(mounted)?;
    let slots = slots_per_bank(bank)?;
    let mut best_seq = None;
    let mut best_payload: Option<PayloadBuffer> = None;
    let mut slot_buffer = [0u8; SLOT_SIZE];

    for slot_index in 0..slots {
        read_slot(flash, bank, slot_index, &mut slot_buffer)?;
        let ParsedSlot::Valid(entry) = parse_slot(&slot_buffer)? else {
            continue;
        };
        if entry.kind != EntryKind::QueueEnqueue || entry.object_id != Q::KIND.0 {
            continue;
        }
        if entry.schema_version != Q::SCHEMA_VERSION {
            return Err(StorageError::CorruptData);
        }
        if is_queue_seq_acked(flash, mounted, Q::KIND, entry.sequence)? {
            continue;
        }
        if best_seq.is_none_or(|seq: QueueSeq| entry.sequence < seq.0) {
            best_seq = Some(QueueSeq(entry.sequence));
            best_payload = Some(PayloadBuffer::from_slice(
                &slot_buffer[SLOT_HEADER_LEN..SLOT_HEADER_LEN + entry.payload_len],
            )?);
        }
    }

    match (best_seq, best_payload) {
        (Some(seq), Some(payload)) => Ok(Some((seq, payload))),
        _ => Ok(None),
    }
}

fn next_state_generation<F>(
    flash: &mut FlashSlice<'_, F>,
    mounted: &MountedPartition,
) -> Result<u64, StorageError>
where
    F: NorFlash + ReadNorFlash,
{
    let bank = active_bank_geometry(mounted)?;
    let mut max_seq = 0;
    let slots = slots_per_bank(bank)?;
    let mut slot_buffer = [0u8; SLOT_SIZE];

    for slot_index in 0..slots {
        read_slot(flash, bank, slot_index, &mut slot_buffer)?;
        let ParsedSlot::Valid(entry) = parse_slot(&slot_buffer)? else {
            continue;
        };
        if matches!(entry.kind, EntryKind::StatePut | EntryKind::StateDelete) {
            max_seq = max_seq.max(entry.sequence);
        }
    }

    Ok(max_seq)
}

fn next_queue_sequence<F>(
    flash: &mut FlashSlice<'_, F>,
    mounted: &MountedPartition,
) -> Result<u64, StorageError>
where
    F: NorFlash + ReadNorFlash,
{
    let bank = active_bank_geometry(mounted)?;
    let mut max_seq = 0;
    let slots = slots_per_bank(bank)?;
    let mut slot_buffer = [0u8; SLOT_SIZE];

    for slot_index in 0..slots {
        read_slot(flash, bank, slot_index, &mut slot_buffer)?;
        let ParsedSlot::Valid(entry) = parse_slot(&slot_buffer)? else {
            continue;
        };
        if matches!(entry.kind, EntryKind::QueueEnqueue | EntryKind::QueueAck) {
            max_seq = max_seq.max(entry.sequence);
        }
    }

    Ok(max_seq)
}

fn collect_latest_state_entries<F>(
    flash: &mut FlashSlice<'_, F>,
    bank: BankGeometry,
    latest: &mut [Option<LatestStateEntry>; MAX_STATE_KEYS],
) -> Result<(), StorageError>
where
    F: NorFlash + ReadNorFlash,
{
    let mut slot_buffer = [0u8; SLOT_SIZE];
    let slots = slots_per_bank(bank)?;

    for slot_index in 0..slots {
        read_slot(flash, bank, slot_index, &mut slot_buffer)?;
        let ParsedSlot::Valid(entry) = parse_slot(&slot_buffer)? else {
            continue;
        };
        if !matches!(entry.kind, EntryKind::StatePut | EntryKind::StateDelete) {
            continue;
        }

        let key = RecordKey::new(namespace_from_raw(entry.namespace)?, entry.object_id);
        let candidate = LatestStateEntry {
            key,
            sequence: entry.sequence,
            deleted: entry.kind == EntryKind::StateDelete,
            slot_index,
        };

        let mut placed = false;
        for slot in latest.iter_mut() {
            match slot {
                Some(existing) if existing.key == key => {
                    if candidate.sequence > existing.sequence {
                        *existing = candidate;
                    }
                    placed = true;
                    break;
                }
                None if !placed => {
                    *slot = Some(candidate);
                    placed = true;
                    break;
                }
                _ => {}
            }
        }

        if !placed {
            return Err(StorageError::TooManyKeys);
        }
    }

    Ok(())
}

fn sort_state_entries_by_sequence(entries: &mut [Option<LatestStateEntry>; MAX_STATE_KEYS]) {
    let len = entries.iter().flatten().count();
    if len < 2 {
        return;
    }

    let slice = &mut entries[..len];
    let mut i = 1;
    while i < slice.len() {
        let mut j = i;
        while j > 0 {
            let left = slice[j - 1].unwrap();
            let right = slice[j].unwrap();
            if left.sequence <= right.sequence {
                break;
            }
            slice.swap(j - 1, j);
            j -= 1;
        }
        i += 1;
    }
}

fn is_queue_seq_acked<F>(
    flash: &mut FlashSlice<'_, F>,
    mounted: &MountedPartition,
    kind: QueueKind,
    sequence: u64,
) -> Result<bool, StorageError>
where
    F: NorFlash + ReadNorFlash,
{
    is_queue_seq_acked_by_kind(flash, active_bank_geometry(mounted)?, kind, sequence)
}

fn is_queue_seq_acked_by_kind<F>(
    flash: &mut FlashSlice<'_, F>,
    bank: BankGeometry,
    kind: QueueKind,
    sequence: u64,
) -> Result<bool, StorageError>
where
    F: NorFlash + ReadNorFlash,
{
    let slots = slots_per_bank(bank)?;
    let mut slot_buffer = [0u8; SLOT_SIZE];
    for slot_index in 0..slots {
        read_slot(flash, bank, slot_index, &mut slot_buffer)?;
        let ParsedSlot::Valid(entry) = parse_slot(&slot_buffer)? else {
            continue;
        };
        if entry.kind == EntryKind::QueueAck
            && entry.object_id == kind.0
            && entry.sequence == sequence
        {
            return Ok(true);
        }
    }

    Ok(false)
}

fn active_bank_geometry(mounted: &MountedPartition) -> Result<BankGeometry, StorageError> {
    let (bank0, bank1) = bank_geometries(mounted.geometry)?;
    Ok(if mounted.active_bank == 0 {
        bank0
    } else {
        bank1
    })
}

fn inactive_bank_geometry(mounted: &MountedPartition) -> Result<BankGeometry, StorageError> {
    let (bank0, bank1) = bank_geometries(mounted.geometry)?;
    Ok(if mounted.active_bank == 0 {
        bank1
    } else {
        bank0
    })
}

fn slots_per_bank(bank: BankGeometry) -> Result<u16, StorageError> {
    if bank.length < FlashSlice::<DummyFlash>::ERASE_SIZE as u32 * 2 {
        return Err(StorageError::InvalidPartition);
    }
    Ok(((bank.length - FlashSlice::<DummyFlash>::ERASE_SIZE as u32) / SLOT_SIZE as u32) as u16)
}

fn count_blank_slots<F>(
    flash: &mut FlashSlice<'_, F>,
    bank: BankGeometry,
) -> Result<u16, StorageError>
where
    F: NorFlash + ReadNorFlash,
{
    let slots = slots_per_bank(bank)?;
    let mut blank = 0u16;
    let mut slot_buffer = [0u8; SLOT_SIZE];

    for slot_index in 0..slots {
        read_slot(flash, bank, slot_index, &mut slot_buffer)?;
        if is_blank(&slot_buffer) {
            blank = blank.saturating_add(1);
        }
    }

    Ok(blank)
}

fn first_blank_slot<F>(
    flash: &mut FlashSlice<'_, F>,
    bank: BankGeometry,
) -> Result<Option<u16>, StorageError>
where
    F: NorFlash + ReadNorFlash,
{
    let slots = slots_per_bank(bank)?;
    let mut slot_buffer = [0u8; SLOT_SIZE];

    for slot_index in 0..slots {
        read_slot(flash, bank, slot_index, &mut slot_buffer)?;
        if is_blank(&slot_buffer) {
            return Ok(Some(slot_index));
        }
    }

    Ok(None)
}

fn read_slot<F>(
    flash: &mut FlashSlice<'_, F>,
    bank: BankGeometry,
    slot_index: u16,
    out: &mut [u8; SLOT_SIZE],
) -> Result<(), StorageError>
where
    F: NorFlash + ReadNorFlash,
{
    let offset = bank.offset
        + FlashSlice::<DummyFlash>::ERASE_SIZE as u32
        + slot_index as u32 * SLOT_SIZE as u32;
    flash.read_into(offset, out)
}

fn write_slot<F>(
    flash: &mut FlashSlice<'_, F>,
    bank: BankGeometry,
    slot_index: u16,
    slot: &[u8; SLOT_SIZE],
) -> Result<(), StorageError>
where
    F: NorFlash + ReadNorFlash,
{
    let offset = bank.offset
        + FlashSlice::<DummyFlash>::ERASE_SIZE as u32
        + slot_index as u32 * SLOT_SIZE as u32;
    flash.write(offset, &slot[..SLOT_COMMIT_OFFSET])?;
    flash.write(
        offset + SLOT_COMMIT_OFFSET as u32,
        &slot[SLOT_COMMIT_OFFSET..],
    )?;
    Ok(())
}

enum ParsedSlot {
    Blank,
    Invalid,
    Valid(ParsedEntry),
}

fn parse_slot(slot: &[u8; SLOT_SIZE]) -> Result<ParsedSlot, StorageError> {
    if is_blank(slot) {
        return Ok(ParsedSlot::Blank);
    }

    if read_u32(slot, 0) != SLOT_MAGIC {
        return Ok(ParsedSlot::Invalid);
    }

    let Some(kind) = EntryKind::from_u8(slot[4]) else {
        return Ok(ParsedSlot::Invalid);
    };
    let payload_len = read_u16(slot, 10) as usize;
    if payload_len > SLOT_PAYLOAD_MAX {
        return Ok(ParsedSlot::Invalid);
    }

    if read_u32(slot, SLOT_COMMIT_OFFSET) != SLOT_COMMIT_MAGIC {
        return Ok(ParsedSlot::Invalid);
    }

    let payload = &slot[SLOT_HEADER_LEN..SLOT_HEADER_LEN + payload_len];
    let crc = read_u32(slot, 20);
    if crc32(payload) != crc {
        return Ok(ParsedSlot::Invalid);
    }

    Ok(ParsedSlot::Valid(ParsedEntry {
        kind,
        namespace: slot[5],
        object_id: read_u16(slot, 6),
        schema_version: read_u16(slot, 8),
        payload_len,
        sequence: read_u64(slot, 12),
    }))
}

fn build_slot(
    kind: EntryKind,
    namespace: u8,
    object_id: u16,
    schema_version: u16,
    sequence: u64,
    payload: &[u8],
) -> Result<[u8; SLOT_SIZE], StorageError> {
    if payload.len() > SLOT_PAYLOAD_MAX {
        return Err(StorageError::PayloadTooLarge);
    }

    let mut slot = [0xFF; SLOT_SIZE];
    write_u32(&mut slot, 0, SLOT_MAGIC);
    slot[4] = kind as u8;
    slot[5] = namespace;
    write_u16(&mut slot, 6, object_id);
    write_u16(&mut slot, 8, schema_version);
    write_u16(&mut slot, 10, payload.len() as u16);
    write_u64(&mut slot, 12, sequence);
    write_u32(&mut slot, 20, crc32(payload));
    slot[SLOT_HEADER_LEN..SLOT_HEADER_LEN + payload.len()].copy_from_slice(payload);
    write_u32(&mut slot, SLOT_COMMIT_OFFSET, SLOT_COMMIT_MAGIC);
    Ok(slot)
}

fn namespace_from_raw(raw: u8) -> Result<::domain::storage::RecordNamespace, StorageError> {
    match raw {
        1 => Ok(::domain::storage::RecordNamespace::Settings),
        2 => Ok(::domain::storage::RecordNamespace::Network),
        3 => Ok(::domain::storage::RecordNamespace::Backend),
        4 => Ok(::domain::storage::RecordNamespace::Device),
        5 => Ok(::domain::storage::RecordNamespace::Storage),
        _ => Err(StorageError::CorruptData),
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}

fn write_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn is_blank(bytes: &[u8]) -> bool {
    bytes.iter().all(|byte| *byte == 0xFF)
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in bytes {
        crc ^= *byte as u32;
        let mut bit = 0;
        while bit < 8 {
            let mask = (crc & 1).wrapping_neg() & 0xEDB8_8320;
            crc = (crc >> 1) ^ mask;
            bit += 1;
        }
    }
    !crc
}

fn codec_error(_: StorageCodecError) -> StorageError {
    StorageError::CodecFailure
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct BackendCredential {
    pub refresh_token: [u8; BACKEND_REFRESH_TOKEN_MAX_LEN],
    pub refresh_token_len: u16,
}

impl BackendCredential {
    pub const fn empty() -> Self {
        Self {
            refresh_token: [0; BACKEND_REFRESH_TOKEN_MAX_LEN],
            refresh_token_len: 0,
        }
    }

    pub fn from_refresh_token(token: &str) -> Result<Self, StorageCodecError> {
        let token = token.trim();
        if token.is_empty() || token.len() > BACKEND_REFRESH_TOKEN_MAX_LEN {
            return Err(StorageCodecError::InvalidData);
        }

        let mut refresh_token = [0u8; BACKEND_REFRESH_TOKEN_MAX_LEN];
        refresh_token[..token.len()].copy_from_slice(token.as_bytes());

        Ok(Self {
            refresh_token,
            refresh_token_len: token.len() as u16,
        })
    }

    pub fn refresh_token(&self) -> Result<&str, StorageCodecError> {
        let len = self.refresh_token_len as usize;
        if len == 0 || len > BACKEND_REFRESH_TOKEN_MAX_LEN {
            return Err(StorageCodecError::InvalidData);
        }

        core::str::from_utf8(&self.refresh_token[..len]).map_err(|_| StorageCodecError::InvalidData)
    }
}

impl Default for BackendCredential {
    fn default() -> Self {
        Self::empty()
    }
}

pub struct PersistedSettingsCodec;

impl RecordCodec for PersistedSettingsCodec {
    type Value = PersistedSettings;

    const KEY: RecordKey = SETTINGS_RECORD_KEY;
    const SCHEMA_VERSION: u16 = 1;
    const MAX_ENCODED_LEN: usize = 16;

    fn encode(value: &Self::Value, out: &mut [u8]) -> Result<usize, StorageCodecError> {
        if out.len() < Self::MAX_ENCODED_LEN {
            return Err(StorageCodecError::BufferTooSmall);
        }

        let topic_bits = value.topics.to_bits();
        out[..8].copy_from_slice(&value.inactivity_timeout_ms.to_le_bytes());
        out[8..10].copy_from_slice(&value.reading_speed_wpm.to_le_bytes());
        out[10] = value.appearance.to_byte();
        out[11] = 0;
        out[12..16].copy_from_slice(&topic_bits.to_le_bytes());
        Ok(16)
    }

    fn decode(bytes: &[u8]) -> Result<Self::Value, StorageCodecError> {
        if bytes.len() == 8 {
            let mut raw = [0u8; 8];
            raw.copy_from_slice(bytes);
            return Ok(PersistedSettings::new(u64::from_le_bytes(raw)));
        }

        if bytes.len() != 16 {
            return Err(StorageCodecError::InvalidData);
        }

        let mut timeout_raw = [0u8; 8];
        timeout_raw.copy_from_slice(&bytes[..8]);

        let mut speed_raw = [0u8; 2];
        speed_raw.copy_from_slice(&bytes[8..10]);

        let mut topic_bits_raw = [0u8; 4];
        topic_bits_raw.copy_from_slice(&bytes[12..16]);

        Ok(PersistedSettings::with_preferences(
            u64::from_le_bytes(timeout_raw),
            u16::from_le_bytes(speed_raw),
            AppearanceMode::from_byte(bytes[10]),
            TopicPreferences::from_bits(u32::from_le_bytes(topic_bits_raw)),
        ))
    }
}

pub struct BackendCredentialCodec;

impl RecordCodec for BackendCredentialCodec {
    type Value = BackendCredential;

    const KEY: RecordKey = BACKEND_CREDENTIAL_RECORD_KEY;
    const SCHEMA_VERSION: u16 = 1;
    const MAX_ENCODED_LEN: usize = 2 + BACKEND_REFRESH_TOKEN_MAX_LEN;

    fn encode(value: &Self::Value, out: &mut [u8]) -> Result<usize, StorageCodecError> {
        let len = value.refresh_token_len as usize;
        if len == 0 || len > BACKEND_REFRESH_TOKEN_MAX_LEN {
            return Err(StorageCodecError::InvalidData);
        }

        if out.len() < 2 + len {
            return Err(StorageCodecError::BufferTooSmall);
        }

        out[..2].copy_from_slice(&value.refresh_token_len.to_le_bytes());
        out[2..2 + len].copy_from_slice(&value.refresh_token[..len]);
        Ok(2 + len)
    }

    fn decode(bytes: &[u8]) -> Result<Self::Value, StorageCodecError> {
        if bytes.len() < 3 {
            return Err(StorageCodecError::InvalidData);
        }

        let mut len_raw = [0u8; 2];
        len_raw.copy_from_slice(&bytes[..2]);
        let len = u16::from_le_bytes(len_raw) as usize;

        if len == 0 || len > BACKEND_REFRESH_TOKEN_MAX_LEN || bytes.len() != 2 + len {
            return Err(StorageCodecError::InvalidData);
        }

        let mut refresh_token = [0u8; BACKEND_REFRESH_TOKEN_MAX_LEN];
        refresh_token[..len].copy_from_slice(&bytes[2..]);
        core::str::from_utf8(&refresh_token[..len]).map_err(|_| StorageCodecError::InvalidData)?;

        Ok(BackendCredential {
            refresh_token,
            refresh_token_len: len as u16,
        })
    }
}

#[derive(Debug)]
struct DummyFlash;

impl ErrorType for DummyFlash {
    type Error = StorageError;
}

impl ReadNorFlash for DummyFlash {
    const READ_SIZE: usize = 4;

    fn read(&mut self, _offset: u32, _bytes: &mut [u8]) -> Result<(), Self::Error> {
        Err(StorageError::Unavailable)
    }

    fn capacity(&self) -> usize {
        0
    }
}

impl NorFlash for DummyFlash {
    const WRITE_SIZE: usize = 4;
    const ERASE_SIZE: usize = 4096;

    fn erase(&mut self, _from: u32, _to: u32) -> Result<(), Self::Error> {
        Err(StorageError::Unavailable)
    }

    fn write(&mut self, _offset: u32, _bytes: &[u8]) -> Result<(), Self::Error> {
        Err(StorageError::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::{
        settings::PersistedSettings,
        storage::{RecordKey, RecordNamespace},
    };

    const FLASH_SIZE: usize = 256 * 1024;
    const PARTITION_SIZE: u32 = 64 * 1024;

    #[derive(Debug)]
    struct EmuFlash<const SIZE: usize> {
        bytes: [u8; SIZE],
    }

    impl<const SIZE: usize> EmuFlash<SIZE> {
        fn new() -> Self {
            Self {
                bytes: [0xFF; SIZE],
            }
        }

        fn corrupt(&mut self, offset: usize, bytes: &[u8]) {
            self.bytes[offset..offset + bytes.len()].copy_from_slice(bytes);
        }
    }

    impl<const SIZE: usize> ErrorType for EmuFlash<SIZE> {
        type Error = StorageError;
    }

    impl<const SIZE: usize> ReadNorFlash for EmuFlash<SIZE> {
        const READ_SIZE: usize = 4;

        fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
            let offset = offset as usize;
            let end = offset + bytes.len();
            if end > SIZE {
                return Err(StorageError::InvalidPartition);
            }
            bytes.copy_from_slice(&self.bytes[offset..end]);
            Ok(())
        }

        fn capacity(&self) -> usize {
            SIZE
        }
    }

    impl<const SIZE: usize> NorFlash for EmuFlash<SIZE> {
        const WRITE_SIZE: usize = 4;
        const ERASE_SIZE: usize = 4096;

        fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
            let from = from as usize;
            let to = to as usize;
            if from > to || to > SIZE || from % Self::ERASE_SIZE != 0 || to % Self::ERASE_SIZE != 0
            {
                return Err(StorageError::InvalidPartition);
            }
            self.bytes[from..to].fill(0xFF);
            Ok(())
        }

        fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
            let offset = offset as usize;
            let end = offset + bytes.len();
            if end > SIZE || offset % Self::WRITE_SIZE != 0 || bytes.len() % Self::WRITE_SIZE != 0 {
                return Err(StorageError::InvalidPartition);
            }
            for (index, byte) in bytes.iter().enumerate() {
                let current = self.bytes[offset + index];
                if (current & *byte) != *byte {
                    return Err(StorageError::FlashFailure);
                }
                self.bytes[offset + index] = *byte;
            }
            Ok(())
        }
    }

    struct TokenCodec;
    impl RecordCodec for TokenCodec {
        type Value = [u8; 13];

        const KEY: RecordKey = RecordKey::new(RecordNamespace::Backend, 1);
        const SCHEMA_VERSION: u16 = 1;
        const MAX_ENCODED_LEN: usize = 13;

        fn encode(value: &Self::Value, out: &mut [u8]) -> Result<usize, StorageCodecError> {
            if out.len() < value.len() {
                return Err(StorageCodecError::BufferTooSmall);
            }
            out[..value.len()].copy_from_slice(value);
            Ok(value.len())
        }

        fn decode(bytes: &[u8]) -> Result<Self::Value, StorageCodecError> {
            if bytes.len() != 13 {
                return Err(StorageCodecError::InvalidData);
            }
            let mut out = [0u8; 13];
            out.copy_from_slice(bytes);
            Ok(out)
        }
    }

    struct WifiCodec;
    impl RecordCodec for WifiCodec {
        type Value = [u8; 16];

        const KEY: RecordKey = RecordKey::new(RecordNamespace::Network, 7);
        const SCHEMA_VERSION: u16 = 1;
        const MAX_ENCODED_LEN: usize = 16;

        fn encode(value: &Self::Value, out: &mut [u8]) -> Result<usize, StorageCodecError> {
            if out.len() < value.len() {
                return Err(StorageCodecError::BufferTooSmall);
            }
            out[..value.len()].copy_from_slice(value);
            Ok(value.len())
        }

        fn decode(bytes: &[u8]) -> Result<Self::Value, StorageCodecError> {
            if bytes.len() != 16 {
                return Err(StorageCodecError::InvalidData);
            }
            let mut out = [0u8; 16];
            out.copy_from_slice(bytes);
            Ok(out)
        }
    }

    struct OutboxCodec;
    impl QueueCodec for OutboxCodec {
        type Value = [u8; 9];

        const KIND: QueueKind = QueueKind::new(3);
        const SCHEMA_VERSION: u16 = 1;
        const MAX_ENCODED_LEN: usize = 9;

        fn encode(value: &Self::Value, out: &mut [u8]) -> Result<usize, StorageCodecError> {
            if out.len() < value.len() {
                return Err(StorageCodecError::BufferTooSmall);
            }
            out[..value.len()].copy_from_slice(value);
            Ok(value.len())
        }

        fn decode(bytes: &[u8]) -> Result<Self::Value, StorageCodecError> {
            if bytes.len() != 9 {
                return Err(StorageCodecError::InvalidData);
            }
            let mut out = [0u8; 9];
            out.copy_from_slice(bytes);
            Ok(out)
        }
    }

    fn new_storage() -> InternalStorage<EmuFlash<FLASH_SIZE>> {
        InternalStorage::from_geometries(
            EmuFlash::new(),
            PartitionGeometry::new(0, PARTITION_SIZE),
            PartitionGeometry::new(PARTITION_SIZE, PARTITION_SIZE),
        )
    }

    #[test]
    fn latest_committed_state_wins() {
        let mut storage = new_storage();
        storage
            .write_record_sync::<TokenCodec>(b"token-value-1")
            .unwrap();
        storage
            .write_record_sync::<TokenCodec>(b"token-value-2")
            .unwrap();

        let loaded = storage.read_record_sync::<TokenCodec>().unwrap().unwrap();
        assert_eq!(&loaded, b"token-value-2");
    }

    #[test]
    fn tombstone_delete_removes_latest_value() {
        let mut storage = new_storage();
        storage
            .write_record_sync::<TokenCodec>(b"token-value-1")
            .unwrap();
        storage.delete_record_sync::<TokenCodec>().unwrap();

        assert_eq!(storage.read_record_sync::<TokenCodec>().unwrap(), None);
    }

    #[test]
    fn queue_enqueue_peek_ack_round_trip() {
        let mut storage = new_storage();
        let first = storage.enqueue_sync::<OutboxCodec>(b"queued-01").unwrap();
        let second = storage.enqueue_sync::<OutboxCodec>(b"queued-02").unwrap();

        let peeked = storage.peek_sync::<OutboxCodec>().unwrap().unwrap();
        assert_eq!(peeked.0, first);
        assert_eq!(&peeked.1, b"queued-01");

        storage.ack_sync::<OutboxCodec>(first).unwrap();

        let peeked = storage.peek_sync::<OutboxCodec>().unwrap().unwrap();
        assert_eq!(peeked.0, second);
        assert_eq!(&peeked.1, b"queued-02");
    }

    #[test]
    fn torn_record_without_commit_is_ignored() {
        let mut storage = new_storage();
        storage
            .write_record_sync::<TokenCodec>(b"token-value-1")
            .unwrap();

        let bank = active_bank_geometry(&storage.partitions.as_ref().unwrap().state).unwrap();
        let offset = bank.offset + DummyFlash::ERASE_SIZE as u32 + SLOT_SIZE as u32;
        let torn = build_slot(
            EntryKind::StatePut,
            RecordNamespace::Backend as u8,
            1,
            1,
            2,
            b"token-value-2",
        )
        .unwrap();
        storage
            .flash
            .write(offset, &torn[..SLOT_COMMIT_OFFSET])
            .unwrap();

        let loaded = storage.read_record_sync::<TokenCodec>().unwrap().unwrap();
        assert_eq!(&loaded, b"token-value-1");
    }

    #[test]
    fn corrupt_crc_entry_is_ignored() {
        let mut storage = new_storage();
        storage
            .write_record_sync::<TokenCodec>(b"token-value-1")
            .unwrap();

        let bank = active_bank_geometry(&storage.partitions.as_ref().unwrap().state).unwrap();
        let offset = bank.offset + DummyFlash::ERASE_SIZE as u32;
        storage
            .flash
            .corrupt(offset as usize + 20, &[0xAA, 0xBB, 0xCC, 0xDD]);

        assert_eq!(storage.read_record_sync::<TokenCodec>().unwrap(), None);
    }

    #[test]
    fn compaction_preserves_latest_live_records() {
        let mut storage = new_storage();
        let state = storage.partitions.as_ref().unwrap().state;
        let bank = active_bank_geometry(&state).unwrap();
        let slots = slots_per_bank(bank).unwrap();

        for index in 0..slots + 2 {
            let mut token = *b"token-value-1";
            token[11] = b'0' + (index % 10) as u8;
            storage.write_record_sync::<TokenCodec>(&token).unwrap();

            let mut wifi = *b"wifi-credential1";
            wifi[15] = b'0' + (index % 10) as u8;
            storage.write_record_sync::<WifiCodec>(&wifi).unwrap();
        }

        let token = storage.read_record_sync::<TokenCodec>().unwrap().unwrap();
        let wifi = storage.read_record_sync::<WifiCodec>().unwrap().unwrap();
        assert_eq!(token[11], b'0' + ((slots + 1) % 10) as u8);
        assert_eq!(wifi[15], b'0' + ((slots + 1) % 10) as u8);
    }

    #[test]
    fn health_reports_mounted_partitions() {
        let storage = new_storage();
        let health = storage.health();
        assert_eq!(health.status, StorageStatus::Available);
        assert!(health.state_partition_ready);
        assert!(health.outbox_partition_ready);
    }

    #[test]
    fn persisted_settings_codec_round_trips() {
        let mut topics = TopicPreferences::new();
        topics.toggle_chip(0, 1);
        topics.toggle_chip(3, 6);
        let settings =
            PersistedSettings::with_preferences(45_000, 320, AppearanceMode::Dark, topics);
        let mut encoded = [0u8; PersistedSettingsCodec::MAX_ENCODED_LEN];

        let len = PersistedSettingsCodec::encode(&settings, &mut encoded).unwrap();
        let decoded = PersistedSettingsCodec::decode(&encoded[..len]).unwrap();

        assert_eq!(decoded, settings);
    }

    #[test]
    fn persisted_settings_codec_reads_legacy_timeout_only_payload() {
        let decoded = PersistedSettingsCodec::decode(&45_000u64.to_le_bytes()).unwrap();

        assert_eq!(decoded.inactivity_timeout_ms, 45_000);
        assert_eq!(
            decoded.reading_speed_wpm,
            domain::settings::DEFAULT_READING_SPEED_WPM
        );
        assert_eq!(decoded.appearance, AppearanceMode::Light);
        assert_eq!(decoded.topics, TopicPreferences::new());
    }
}
