extern crate alloc;

use alloc::{boxed::Box, string::String, vec::Vec};
use core::{cmp::Ordering, mem::size_of, ptr::addr_of_mut};

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering as AtomicOrdering};
use domain::{
    content::{
        CONTENT_ID_MAX_BYTES, CONTENT_META_MAX_BYTES, CONTENT_TITLE_MAX_BYTES, CollectionKind,
        CollectionManifestItem, CollectionManifestState, ContentState, DetailLocator,
        MANIFEST_ITEM_CAPACITY, PackageState, RECOMMENDATION_SERVE_ID_MAX_BYTES,
        REMOTE_ITEM_ID_MAX_BYTES, RemoteContentStatus,
    },
    formatter::{
        MAX_PARAGRAPH_PREVIEW_BYTES, MAX_READING_PARAGRAPHS, MAX_READING_TOKEN_BYTES,
        ReadingDocument, StageFont, UnitFlags,
    },
    reader::{READER_WINDOW_MAX_UNITS, ReaderParagraphInfo, ReaderWindow},
    storage::StorageRecoveryStatus,
    text::InlineText,
};
use embassy_executor::Spawner;
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel, signal::Signal,
};
use embassy_time::Instant;
use embedded_hal_bus::spi::{ExclusiveDevice, NoDelay};
use embedded_sdmmc::{
    Block, BlockDevice, BlockIdx, Directory, Error as SdError, File, Mode, RawFile, RawVolume,
    SdCard, ShortFileName, TimeSource, Timestamp, VolumeIdx, VolumeManager,
};
use esp_hal::{Blocking, delay::Delay, gpio::Output, spi::master::Spi, time::Rate};
use log::info;
use services::storage::StorageError;

use crate::telemetry::{TraceContext, bool_flag, collection_label};

const MAX_DIRS: usize = 8;
const MAX_FILES: usize = 4;
const MAX_VOLUMES: usize = 1;
// Chunk payloads now live in boxed buffers, so increasing the queue only adds a
// small amount of fixed resident state while allowing the backend to stay a few
// writes ahead of SD flush latency.
const STORAGE_CMD_QUEUE_CAPACITY: usize = 8;
const MANIFEST_MAGIC: u32 = 0x4D43_4F4C;
const CACHE_INDEX_MAGIC: u32 = 0x4D43_4944;
const PACKAGE_META_MAGIC: u32 = 0x4D43_504D;
const READER_PACKAGE_MAGIC: u32 = u32::from_le_bytes(*b"MTRP");
const READER_PACKAGE_FORMAT_VERSION: u16 = 1;
const FORMAT_VERSION: u16 = 1;
const MAX_MANIFEST_SNAPSHOT_LEN: usize = 4096;
const MAX_CACHE_INDEX_LEN: usize = 4096;
const MAX_PACKAGE_META_LEN: usize = 128;
const READER_PACKAGE_HEADER_LEN: usize = 32;
const READER_PACKAGE_PARAGRAPH_ENTRY_LEN: usize = 72;
const READER_PACKAGE_UNIT_ENTRY_LEN: usize = 40;
const PACKAGE_COPY_BUFFER_LEN: usize = 8 * 1024;
// Keep this aligned with the backend download chunk. The payload itself is now
// boxed so we can raise the transfer size without exploding the storage
// command queue's fixed internal residency.
const STAGE_WRITE_CHUNK_LEN: usize = 8 * 1024;
const STAGE_FLUSH_INTERVAL_BYTES: u32 = 16 * 1024;
const STAGE_PROGRESS_LOG_INTERVAL_BYTES: u32 = 16 * 1024;
const CACHE_ENTRY_CAPACITY: usize = 48;
const CACHE_SIZE_BUDGET_BYTES: u64 = 32 * 1024 * 1024;
const PACKAGE_READ_BUFFER_LEN: usize = 512;
const MAX_JSON_KEY_BYTES: usize = 16;
const MAX_PARSED_TITLE_BYTES: usize = CONTENT_TITLE_MAX_BYTES * 4;
// Keep per-block scratch bounded independently from the whole-document capacity.
// We want much larger articles overall without allowing a single paragraph parse
// to balloon peak heap usage in lockstep with MAX_READING_UNITS.
const MAX_PARSED_BLOCK_TEXT_BYTES: usize = 8 * 1024;
const MAX_PARSED_LIST_ITEMS: usize = MAX_READING_PARAGRAPHS;
const MAX_PARSED_LIST_TOTAL_BYTES: usize = 16 * 1024;

// Dev-time content storage reset. Use a fresh top-level root while storage evolves.
const ROOT_DIR_NAME: &str = "MTDV0003";
const VERSION_DIR_NAME: &str = "V1";
const MANIFEST_DIR_NAME: &str = "MANIF";
const PACKAGE_DIR_NAME: &str = "PKG";
const STAGING_DIR_NAME: &str = "STAGE";
const CACHE_DIR_NAME: &str = "CACHE";
const ACTIVE_STAGE_FILE_NAME: &str = "ACTIVE.PRT";
const SAVED_MANIFEST_FILE_NAME: &str = "SAVED.BIN";
const INBOX_MANIFEST_FILE_NAME: &str = "INBOX.BIN";
const RECOMMENDATION_MANIFEST_FILE_NAME: &str = "RECS.BIN";
const CACHE_INDEX_FILE_NAME: &str = "PKGIDX.BIN";

type SdBus<'d> = Spi<'d, Blocking>;
type SdSpiDevice<'d> = ExclusiveDevice<SdBus<'d>, Output<'d>, NoDelay>;
type SdBlockDevice<'d> = SdCard<SdSpiDevice<'d>, Delay>;
type SdVolumeManager<'d> =
    VolumeManager<SdBlockDevice<'d>, FixedTimeSource, MAX_DIRS, MAX_FILES, MAX_VOLUMES>;
type SdDirectory<'a, 'd> =
    Directory<'a, SdBlockDevice<'d>, FixedTimeSource, MAX_DIRS, MAX_FILES, MAX_VOLUMES>;
type SdFile<'a, 'd> =
    File<'a, SdBlockDevice<'d>, FixedTimeSource, MAX_DIRS, MAX_FILES, MAX_VOLUMES>;

static STORAGE_CMD_CH: Channel<
    CriticalSectionRawMutex,
    StorageCommand,
    STORAGE_CMD_QUEUE_CAPACITY,
> = Channel::new();
static STORAGE_RESP_SIG: Signal<CriticalSectionRawMutex, StorageResponse> = Signal::new();
static STORAGE_AVAILABLE: AtomicBool = AtomicBool::new(false);
static STORAGE_CMD_DEPTH: AtomicUsize = AtomicUsize::new(0);
static STORAGE_CMD_DEPTH_PEAK: AtomicUsize = AtomicUsize::new(0);
static STORAGE_CMD_PAYLOAD_BYTES: AtomicUsize = AtomicUsize::new(0);
static STORAGE_CMD_PAYLOAD_BYTES_PEAK: AtomicUsize = AtomicUsize::new(0);

pub struct ContentStorageMount<'d> {
    pub storage: Option<Box<SdContentStorage<'d>>>,
    pub sd_card_ready: bool,
    pub sd_total_bytes: u64,
    pub sd_free_bytes: u64,
    pub last_recovery: StorageRecoveryStatus,
}

pub struct SdContentStorage<'d> {
    volume_mgr: SdVolumeManager<'d>,
    total_bytes: u64,
    snapshots: [Option<Box<CollectionManifestState>>; 3],
    cache_index: CacheIndex,
    pending_stage: Option<PendingStage>,
    pending_stage_error: Option<StorageError>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct OpenedReaderContent {
    pub title: InlineText<CONTENT_TITLE_MAX_BYTES>,
    pub document: Box<ReadingDocument>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct OpenedReaderPackage {
    pub title: InlineText<CONTENT_TITLE_MAX_BYTES>,
    pub total_units: u32,
    pub paragraphs: Box<[ReaderParagraphInfo]>,
    pub window: Box<ReaderWindow>,
}

#[derive(Debug)]
pub struct CommitAndOpenPackageResult {
    pub snapshot: Box<CollectionManifestState>,
    pub opened: Result<Box<OpenedReaderPackage>, StorageError>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct ReaderPackageHeader {
    title_len: u16,
    paragraph_count: u16,
    unit_count: u32,
    paragraph_table_offset: u32,
    unit_table_offset: u32,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct PendingStage {
    trace: TraceContext,
    content_id: InlineText<CONTENT_ID_MAX_BYTES>,
    remote_revision: u64,
    slot_id: u8,
    target_kind: PendingStageTargetKind,
    stage_volume: RawVolume,
    stage_file: RawFile,
    bytes_written: u32,
    flushed_bytes: u32,
    crc32: u32,
    started_at_ms: u64,
    overwritten_entry: Option<CacheEntry>,
    superseded_entry: Option<CacheEntry>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum PendingStageTargetKind {
    StagingFile,
    PackageSlot,
}

impl PendingStageTargetKind {
    const fn label(self) -> &'static str {
        match self {
            Self::StagingFile => "staging_file",
            Self::PackageSlot => "package_slot",
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct PackageMeta {
    slot_id: u8,
    remote_revision: u64,
    size_bytes: u32,
    crc32: u32,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct CacheEntry {
    slot_id: u8,
    content_id: InlineText<CONTENT_ID_MAX_BYTES>,
    remote_revision: u64,
    size_bytes: u32,
    crc32: u32,
    last_touch_seq: u32,
    collection_flags: u8,
}

impl CacheEntry {
    const fn empty() -> Self {
        Self {
            slot_id: 0,
            content_id: InlineText::new(),
            remote_revision: 0,
            size_bytes: 0,
            crc32: 0,
            last_touch_seq: 0,
            collection_flags: 0,
        }
    }

    const fn is_empty(&self) -> bool {
        self.slot_id == 0 || self.content_id.is_empty()
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct CacheIndex {
    entries: [CacheEntry; CACHE_ENTRY_CAPACITY],
    len: u8,
    next_touch_seq: u32,
}

impl CacheIndex {
    const fn empty() -> Self {
        Self {
            entries: [CacheEntry::empty(); CACHE_ENTRY_CAPACITY],
            len: 0,
            next_touch_seq: 1,
        }
    }

    fn len(&self) -> usize {
        self.len as usize
    }

    fn total_bytes(&self) -> u64 {
        let mut total = 0u64;
        let mut index = 0;
        while index < self.len() {
            total = total.saturating_add(self.entries[index].size_bytes as u64);
            index += 1;
        }
        total
    }

    fn find_by_content_id(
        &self,
        content_id: &InlineText<CONTENT_ID_MAX_BYTES>,
    ) -> Option<CacheEntry> {
        let mut index = 0;
        while index < self.len() {
            let entry = self.entries[index];
            if entry.content_id == *content_id {
                return Some(entry);
            }
            index += 1;
        }
        None
    }

    fn find_index_by_content_id(
        &self,
        content_id: &InlineText<CONTENT_ID_MAX_BYTES>,
    ) -> Option<usize> {
        let mut index = 0;
        while index < self.len() {
            if self.entries[index].content_id == *content_id {
                return Some(index);
            }
            index += 1;
        }
        None
    }

    fn find_index_by_slot(&self, slot_id: u8) -> Option<usize> {
        let mut index = 0;
        while index < self.len() {
            if self.entries[index].slot_id == slot_id {
                return Some(index);
            }
            index += 1;
        }
        None
    }

    fn contains_slot(&self, slot_id: u8) -> bool {
        self.find_index_by_slot(slot_id).is_some()
    }

    fn upsert(&mut self, mut entry: CacheEntry) {
        if let Some(index) = self.find_index_by_content_id(&entry.content_id) {
            entry.last_touch_seq = self.bump_touch_seq();
            self.entries[index] = entry;
            return;
        }

        if self.len() < CACHE_ENTRY_CAPACITY {
            entry.last_touch_seq = self.bump_touch_seq();
            self.entries[self.len()] = entry;
            self.len = self.len.saturating_add(1);
        }
    }

    fn remove_slot(&mut self, slot_id: u8) -> Option<CacheEntry> {
        let index = self.find_index_by_slot(slot_id)?;

        let removed = self.entries[index];
        let last_index = self.len().saturating_sub(1);
        self.entries[index] = self.entries[last_index];
        self.entries[last_index] = CacheEntry::empty();
        self.len = self.len.saturating_sub(1);
        Some(removed)
    }

    fn next_available_slot_id(&self) -> Option<u8> {
        let mut slot = 1u8;
        while (slot as usize) <= CACHE_ENTRY_CAPACITY {
            if self.find_index_by_slot(slot).is_none() {
                return Some(slot);
            }
            slot = slot.saturating_add(1);
        }
        None
    }

    fn bump_touch_seq(&mut self) -> u32 {
        let next = self.next_touch_seq;
        self.next_touch_seq = self.next_touch_seq.saturating_add(1).max(1);
        next
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct FixedTimeSource;

impl TimeSource for FixedTimeSource {
    fn get_timestamp(&self) -> Timestamp {
        Timestamp {
            year_since_1970: 56,
            zero_indexed_month: 0,
            zero_indexed_day: 0,
            hours: 0,
            minutes: 0,
            seconds: 0,
        }
    }
}

#[derive(Debug)]
enum StageChunkBytes<const N: usize> {
    External(crate::memory_policy::ExternalBox<[u8; N]>),
    Internal(crate::memory_policy::InternalBox<[u8; N]>),
}

impl<const N: usize> StageChunkBytes<N> {
    fn allocate_zeroed() -> Result<Self, StorageError> {
        match crate::memory_policy::try_external_zeroed_array_box::<N>() {
            Ok(buffer) => Ok(Self::External(buffer)),
            Err(_) => match crate::memory_policy::try_internal_zeroed_array_box::<N>() {
                Ok(buffer) => Ok(Self::Internal(buffer)),
                Err(_) => Err(StorageError::Unavailable),
            },
        }
    }

    fn as_mut_slice(&mut self) -> &mut [u8] {
        match self {
            Self::External(buffer) => &mut buffer[..],
            Self::Internal(buffer) => &mut buffer[..],
        }
    }

    fn as_slice(&self, len: usize) -> &[u8] {
        match self {
            Self::External(buffer) => &buffer[..len],
            Self::Internal(buffer) => &buffer[..len],
        }
    }
}

#[derive(Debug)]
enum StorageCommand {
    PersistSnapshot {
        trace: TraceContext,
        kind: CollectionKind,
        snapshot: Box<CollectionManifestState>,
    },
    BeginPackageStage {
        trace: TraceContext,
        content_id: InlineText<CONTENT_ID_MAX_BYTES>,
        remote_revision: u64,
    },
    WritePackageChunk {
        trace: TraceContext,
        len: usize,
        bytes: StageChunkBytes<STAGE_WRITE_CHUNK_LEN>,
    },
    CommitPackageStage {
        trace: TraceContext,
        collection: CollectionKind,
        remote_item_id: InlineText<REMOTE_ITEM_ID_MAX_BYTES>,
    },
    CommitAndOpenPackageStage {
        trace: TraceContext,
        collection: CollectionKind,
        remote_item_id: InlineText<REMOTE_ITEM_ID_MAX_BYTES>,
        content_id: InlineText<CONTENT_ID_MAX_BYTES>,
    },
    AbortPackageStage {
        trace: TraceContext,
    },
    UpdatePackageState {
        trace: TraceContext,
        collection: CollectionKind,
        remote_item_id: InlineText<REMOTE_ITEM_ID_MAX_BYTES>,
        package_state: PackageState,
    },
    OpenCachedReaderPackage {
        trace: TraceContext,
        content_id: InlineText<CONTENT_ID_MAX_BYTES>,
    },
    LoadReaderWindow {
        trace: TraceContext,
        content_id: InlineText<CONTENT_ID_MAX_BYTES>,
        window_start_unit_index: u32,
    },
    OpenCachedReaderContent {
        trace: TraceContext,
        content_id: InlineText<CONTENT_ID_MAX_BYTES>,
    },
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
enum StorageResponse {
    Snapshot(Result<Box<CollectionManifestState>, StorageError>),
    CommitAndOpenPackage(Result<Box<CommitAndOpenPackageResult>, StorageError>),
    OpenedPackage(Result<Box<OpenedReaderPackage>, StorageError>),
    Opened(Result<Box<OpenedReaderContent>, StorageError>),
    LoadedWindow(Result<Box<ReaderWindow>, StorageError>),
    Unit(Result<(), StorageError>),
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct DirUsage {
    files: u32,
    bytes: u64,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct StorageSpaceMetrics {
    sd_total_bytes: u64,
    sd_free_bytes: u64,
    sd_free_known: bool,
    sd_cluster_size_bytes: u32,
    motif_total_bytes: u64,
    manifest_bytes: u64,
    cache_bytes: u64,
    stage_bytes: u64,
    package_bytes: u64,
    package_files: u32,
    cache_entry_count: usize,
    cache_budget_remaining: u64,
}

pub(crate) fn log_static_inventory() {
    crate::memtrace!(
        "static_inventory",
        "component" = "storage",
        "at_ms" = storage_now_ms(),
        "storage_command_bytes" = size_of::<StorageCommand>(),
        "storage_response_bytes" = size_of::<StorageResponse>(),
        "pending_stage_bytes" = size_of::<PendingStage>(),
        "cache_entry_bytes" = size_of::<CacheEntry>(),
        "cache_index_bytes" = size_of::<CacheIndex>(),
        "opened_reader_package_bytes" = size_of::<OpenedReaderPackage>(),
        "opened_reader_content_bytes" = size_of::<OpenedReaderContent>(),
        "reader_window_bytes" = size_of::<ReaderWindow>(),
        "reading_document_bytes" = size_of::<ReadingDocument>(),
        "collection_manifest_state_bytes" = size_of::<CollectionManifestState>(),
        "storage_queue_capacity" = STORAGE_CMD_QUEUE_CAPACITY,
        "storage_queue_resident_bytes" = STORAGE_CMD_QUEUE_CAPACITY * size_of::<StorageCommand>(),
        "stage_write_chunk_len" = STAGE_WRITE_CHUNK_LEN,
        "stage_flush_interval_bytes" = STAGE_FLUSH_INTERVAL_BYTES,
        "package_copy_buffer_len" = PACKAGE_COPY_BUFFER_LEN,
        "package_read_buffer_len" = PACKAGE_READ_BUFFER_LEN,
        "cache_entry_capacity" = CACHE_ENTRY_CAPACITY,
        "cache_size_budget_bytes" = CACHE_SIZE_BUDGET_BYTES,
        "max_parsed_block_text_bytes" = MAX_PARSED_BLOCK_TEXT_BYTES,
        "max_parsed_list_total_bytes" = MAX_PARSED_LIST_TOTAL_BYTES,
        "max_parsed_list_items" = MAX_PARSED_LIST_ITEMS,
    );
}

fn storage_command_payload_len(command: &StorageCommand) -> usize {
    match command {
        StorageCommand::WritePackageChunk { len, .. } => *len,
        _ => 0,
    }
}

fn storage_command_label(command: &StorageCommand) -> &'static str {
    match command {
        StorageCommand::PersistSnapshot { .. } => "persist_snapshot",
        StorageCommand::BeginPackageStage { .. } => "begin_stage",
        StorageCommand::WritePackageChunk { .. } => "write_chunk",
        StorageCommand::CommitPackageStage { .. } => "commit_stage",
        StorageCommand::CommitAndOpenPackageStage { .. } => "commit_and_open_stage",
        StorageCommand::AbortPackageStage { .. } => "abort_stage",
        StorageCommand::UpdatePackageState { .. } => "update_package_state",
        StorageCommand::OpenCachedReaderPackage { .. } => "open_cached_reader_package",
        StorageCommand::LoadReaderWindow { .. } => "load_reader_window",
        StorageCommand::OpenCachedReaderContent { .. } => "open_cached_reader_content",
    }
}

fn storage_command_trace(command: &StorageCommand) -> TraceContext {
    match command {
        StorageCommand::PersistSnapshot { trace, .. }
        | StorageCommand::BeginPackageStage { trace, .. }
        | StorageCommand::WritePackageChunk { trace, .. }
        | StorageCommand::CommitPackageStage { trace, .. }
        | StorageCommand::CommitAndOpenPackageStage { trace, .. }
        | StorageCommand::AbortPackageStage { trace }
        | StorageCommand::UpdatePackageState { trace, .. }
        | StorageCommand::OpenCachedReaderPackage { trace, .. }
        | StorageCommand::LoadReaderWindow { trace, .. }
        | StorageCommand::OpenCachedReaderContent { trace, .. } => *trace,
    }
}

fn storage_queue_on_enqueue(trace: TraceContext, operation: &str, payload_bytes: usize) {
    let depth = STORAGE_CMD_DEPTH.fetch_add(1, AtomicOrdering::Relaxed) + 1;
    let payload_inflight =
        STORAGE_CMD_PAYLOAD_BYTES.fetch_add(payload_bytes, AtomicOrdering::Relaxed) + payload_bytes;
    let depth_peak = fetch_max(&STORAGE_CMD_DEPTH_PEAK, depth);
    let payload_peak = fetch_max(&STORAGE_CMD_PAYLOAD_BYTES_PEAK, payload_inflight);

    if depth == depth_peak
        || payload_inflight == payload_peak
        || depth == STORAGE_CMD_QUEUE_CAPACITY
    {
        emit_storage_queue_event(
            "enqueue",
            trace,
            operation,
            payload_bytes,
            depth,
            depth_peak,
            payload_inflight,
            payload_peak,
        );
    }
}

fn storage_queue_on_dequeue(command: &StorageCommand) {
    let payload_bytes = storage_command_payload_len(command);
    let depth = STORAGE_CMD_DEPTH
        .fetch_sub(1, AtomicOrdering::Relaxed)
        .saturating_sub(1);
    let payload_inflight = STORAGE_CMD_PAYLOAD_BYTES
        .fetch_sub(payload_bytes, AtomicOrdering::Relaxed)
        .saturating_sub(payload_bytes);

    if depth == 0 {
        emit_storage_queue_event(
            "drain",
            storage_command_trace(command),
            storage_command_label(command),
            payload_bytes,
            depth,
            STORAGE_CMD_DEPTH_PEAK.load(AtomicOrdering::Relaxed),
            payload_inflight,
            STORAGE_CMD_PAYLOAD_BYTES_PEAK.load(AtomicOrdering::Relaxed),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_storage_queue_event(
    action: &str,
    trace: TraceContext,
    operation: &str,
    payload_bytes: usize,
    depth: usize,
    depth_peak: usize,
    inflight_payload_bytes: usize,
    inflight_payload_peak: usize,
) {
    crate::memtrace!(
        "storage_queue",
        "component" = "storage",
        "at_ms" = storage_now_ms(),
        "action" = action,
        "op" = operation,
        "sync_id" = trace.sync_id,
        "req_id" = trace.req_id,
        "payload_bytes" = payload_bytes,
        "queue_depth" = depth,
        "queue_depth_peak" = depth_peak,
        "queue_payload_bytes" = inflight_payload_bytes,
        "queue_payload_peak" = inflight_payload_peak,
        "queue_capacity" = STORAGE_CMD_QUEUE_CAPACITY,
        "queue_resident_bytes" = STORAGE_CMD_QUEUE_CAPACITY * size_of::<StorageCommand>(),
    );
}

fn fetch_max(cell: &AtomicUsize, candidate: usize) -> usize {
    let mut current = cell.load(AtomicOrdering::Relaxed);
    while candidate > current {
        match cell.compare_exchange(
            current,
            candidate,
            AtomicOrdering::Relaxed,
            AtomicOrdering::Relaxed,
        ) {
            Ok(_) => return candidate,
            Err(observed) => current = observed,
        }
    }
    current
}

pub fn mount<'d>(spi: SdBus<'d>, cs: Output<'d>, run_spi_hz: u32) -> ContentStorageMount<'d> {
    let device = match ExclusiveDevice::new_no_delay(spi, cs) {
        Ok(device) => device,
        Err(_) => {
            info!("content storage mount failed: sd chip-select init");
            return ContentStorageMount {
                storage: None,
                sd_card_ready: false,
                sd_total_bytes: 0,
                sd_free_bytes: 0,
                last_recovery: StorageRecoveryStatus::Failed,
            };
        }
    };

    let delay = Delay::new();
    let card = SdCard::new(device, delay);
    let total_bytes = match card.num_bytes() {
        Ok(bytes) => bytes,
        Err(err) => {
            info!("content storage mount failed: {:?}", err);
            return ContentStorageMount {
                storage: None,
                sd_card_ready: false,
                sd_total_bytes: 0,
                sd_free_bytes: 0,
                last_recovery: StorageRecoveryStatus::Failed,
            };
        }
    };
    let run_spi_config = esp_hal::spi::master::Config::default()
        .with_frequency(Rate::from_hz(run_spi_hz))
        .with_mode(esp_hal::spi::Mode::_0);
    match card.spi(|device| device.bus_mut().apply_config(&run_spi_config)) {
        Ok(()) => info!("content storage sd spi run hz={}", run_spi_hz),
        Err(err) => info!(
            "content storage sd spi speed switch failed hz={} err={:?}",
            run_spi_hz, err
        ),
    }

    let volume_mgr = VolumeManager::<_, _, MAX_DIRS, MAX_FILES, MAX_VOLUMES>::new_with_limits(
        card,
        FixedTimeSource,
        0,
    );
    let mut storage = Box::<SdContentStorage<'d>>::new_uninit();
    let storage_ptr = storage.as_mut_ptr();
    unsafe {
        addr_of_mut!((*storage_ptr).volume_mgr).write(volume_mgr);
        addr_of_mut!((*storage_ptr).total_bytes).write(total_bytes);
        addr_of_mut!((*storage_ptr).snapshots).write([None, None, None]);
        addr_of_mut!((*storage_ptr).cache_index).write(CacheIndex::empty());
        addr_of_mut!((*storage_ptr).pending_stage).write(None);
        addr_of_mut!((*storage_ptr).pending_stage_error).write(None);
    }
    let mut storage = unsafe { storage.assume_init() };
    let mut last_recovery = StorageRecoveryStatus::Clean;

    if let Err(err) = storage.initialize_layout() {
        info!("content storage layout init failed: {:?}", err);
        if matches!(err, StorageError::CorruptData) {
            info!(
                "content storage layout corrupt: wiping motif sd data root={} version={}",
                ROOT_DIR_NAME, VERSION_DIR_NAME
            );
            match storage.reset_dev_data() {
                Ok(()) => {
                    last_recovery = StorageRecoveryStatus::Recovered;
                    info!("content storage layout recovered by wiping motif sd data");
                }
                Err(recovery_err) => {
                    info!("content storage layout recovery failed: {:?}", recovery_err);
                    return ContentStorageMount {
                        storage: None,
                        sd_card_ready: false,
                        sd_total_bytes: total_bytes,
                        sd_free_bytes: 0,
                        last_recovery: StorageRecoveryStatus::Failed,
                    };
                }
            }
        } else {
            return ContentStorageMount {
                storage: None,
                sd_card_ready: false,
                sd_total_bytes: total_bytes,
                sd_free_bytes: 0,
                last_recovery: StorageRecoveryStatus::Failed,
            };
        }
    } else if let Err(err) = storage.load_state() {
        info!("content storage state load failed: {:?}", err);
        if matches!(err, StorageError::CorruptData) {
            info!(
                "content storage state corrupt: wiping motif sd data root={} version={}",
                ROOT_DIR_NAME, VERSION_DIR_NAME
            );
            match storage.reset_dev_data() {
                Ok(()) => {
                    last_recovery = StorageRecoveryStatus::Recovered;
                    info!("content storage state recovered by wiping motif sd data");
                }
                Err(recovery_err) => {
                    info!("content storage state recovery failed: {:?}", recovery_err);
                    return ContentStorageMount {
                        storage: None,
                        sd_card_ready: false,
                        sd_total_bytes: total_bytes,
                        sd_free_bytes: 0,
                        last_recovery: StorageRecoveryStatus::Failed,
                    };
                }
            }
        } else {
            storage.snapshots = [None, None, None];
            storage.cache_index = CacheIndex::empty();
            let _ = storage.cleanup_active_stage_file();
        };
    }

    let sd_free_bytes = storage
        .storage_space_metrics()
        .map(|metrics| metrics.sd_free_bytes)
        .unwrap_or(0);

    ContentStorageMount {
        storage: Some(storage),
        sd_card_ready: true,
        sd_total_bytes: total_bytes,
        sd_free_bytes,
        last_recovery,
    }
}

pub fn install(spawner: Spawner, storage: Option<Box<SdContentStorage<'static>>>) {
    STORAGE_AVAILABLE.store(storage.is_some(), AtomicOrdering::Relaxed);
    let Some(storage) = storage else {
        info!("content storage disabled: sd unavailable");
        return;
    };

    if spawner.spawn(content_storage_task(storage)).is_err() {
        info!("content storage failed to spawn task");
    }
}

pub(crate) fn bootstrap_content_state(
    storage: Option<&SdContentStorage<'_>>,
) -> Option<Box<ContentState>> {
    let saved = storage
        .and_then(|storage| storage.snapshots[collection_index(CollectionKind::Saved)].as_deref())
        .copied()
        .filter(|snapshot| !snapshot.is_empty())?;

    info!(
        "content storage bootstrap saved snapshot item_count={}",
        saved.len()
    );

    Some(Box::new(ContentState {
        saved: Some(Box::new(saved)),
        inbox: None,
        recommendations: None,
    }))
}

pub async fn persist_snapshot(
    kind: CollectionKind,
    snapshot: CollectionManifestState,
) -> Result<CollectionManifestState, StorageError> {
    persist_snapshot_traced(TraceContext::none(), kind, snapshot).await
}

pub async fn persist_snapshot_traced(
    trace: TraceContext,
    kind: CollectionKind,
    snapshot: CollectionManifestState,
) -> Result<CollectionManifestState, StorageError> {
    if !STORAGE_AVAILABLE.load(AtomicOrdering::Relaxed) {
        return Err(StorageError::Unavailable);
    }
    let command = StorageCommand::PersistSnapshot {
        trace,
        kind,
        snapshot: Box::new(snapshot),
    };
    STORAGE_CMD_CH.send(command).await;
    storage_queue_on_enqueue(trace, "persist_snapshot", 0);

    match STORAGE_RESP_SIG.wait().await {
        StorageResponse::Snapshot(result) => result.map(|snapshot| *snapshot),
        StorageResponse::CommitAndOpenPackage(_)
        | StorageResponse::Opened(_)
        | StorageResponse::OpenedPackage(_)
        | StorageResponse::LoadedWindow(_)
        | StorageResponse::Unit(_) => Err(StorageError::Unavailable),
    }
}

pub async fn commit_package_stage_and_open_cached_reader_package_traced(
    trace: TraceContext,
    collection: CollectionKind,
    remote_item_id: InlineText<REMOTE_ITEM_ID_MAX_BYTES>,
    content_id: InlineText<CONTENT_ID_MAX_BYTES>,
) -> Result<CommitAndOpenPackageResult, StorageError> {
    if !STORAGE_AVAILABLE.load(AtomicOrdering::Relaxed) {
        return Err(StorageError::Unavailable);
    }
    let started_at = Instant::now();
    let command = StorageCommand::CommitAndOpenPackageStage {
        trace,
        collection,
        remote_item_id,
        content_id,
    };
    STORAGE_CMD_CH.send(command).await;
    storage_queue_on_enqueue(trace, "commit_and_open_stage", 0);

    let result = match STORAGE_RESP_SIG.wait().await {
        StorageResponse::CommitAndOpenPackage(result) => result.map(|result| *result),
        StorageResponse::Snapshot(_)
        | StorageResponse::Opened(_)
        | StorageResponse::OpenedPackage(_)
        | StorageResponse::LoadedWindow(_)
        | StorageResponse::Unit(_) => Err(StorageError::Unavailable),
    }?;

    let total_ms = Instant::now().duration_since(started_at).as_millis();
    match &result.opened {
        Ok(opened) => info!(
            "content storage commit+open timing content_id={} total_ms={} total_units={} total_paragraphs={} window_units={}",
            content_id.as_str(),
            total_ms,
            opened.total_units,
            opened.paragraphs.len(),
            opened.window.unit_count,
        ),
        Err(err) => info!(
            "content storage commit+open timing failed content_id={} total_ms={} err={:?}",
            content_id.as_str(),
            total_ms,
            err,
        ),
    }

    Ok(result)
}

pub async fn begin_package_stage(
    content_id: InlineText<CONTENT_ID_MAX_BYTES>,
    remote_revision: u64,
) -> Result<(), StorageError> {
    begin_package_stage_traced(TraceContext::none(), content_id, remote_revision).await
}

pub async fn begin_package_stage_traced(
    trace: TraceContext,
    content_id: InlineText<CONTENT_ID_MAX_BYTES>,
    remote_revision: u64,
) -> Result<(), StorageError> {
    if !STORAGE_AVAILABLE.load(AtomicOrdering::Relaxed) {
        return Err(StorageError::Unavailable);
    }
    let command = StorageCommand::BeginPackageStage {
        trace,
        content_id,
        remote_revision,
    };
    STORAGE_CMD_CH.send(command).await;
    storage_queue_on_enqueue(trace, "begin_stage", 0);

    match STORAGE_RESP_SIG.wait().await {
        StorageResponse::Unit(result) => result,
        StorageResponse::CommitAndOpenPackage(_)
        | StorageResponse::Opened(_)
        | StorageResponse::OpenedPackage(_)
        | StorageResponse::LoadedWindow(_)
        | StorageResponse::Snapshot(_) => Err(StorageError::Unavailable),
    }
}

pub async fn write_package_chunk(chunk: &[u8]) -> Result<(), StorageError> {
    write_package_chunk_traced(TraceContext::none(), chunk).await
}

pub async fn write_package_chunk_traced(
    trace: TraceContext,
    chunk: &[u8],
) -> Result<(), StorageError> {
    if !STORAGE_AVAILABLE.load(AtomicOrdering::Relaxed) {
        return Err(StorageError::Unavailable);
    }
    if chunk.len() > STAGE_WRITE_CHUNK_LEN {
        return Err(StorageError::PayloadTooLarge);
    }

    let mut bytes = StageChunkBytes::<STAGE_WRITE_CHUNK_LEN>::allocate_zeroed()?;
    bytes.as_mut_slice()[..chunk.len()].copy_from_slice(chunk);
    let command = StorageCommand::WritePackageChunk {
        trace,
        len: chunk.len(),
        bytes,
    };
    STORAGE_CMD_CH.send(command).await;
    storage_queue_on_enqueue(trace, "write_chunk", chunk.len());
    Ok(())
}

pub async fn commit_package_stage(
    collection: CollectionKind,
    remote_item_id: InlineText<REMOTE_ITEM_ID_MAX_BYTES>,
) -> Result<CollectionManifestState, StorageError> {
    commit_package_stage_traced(TraceContext::none(), collection, remote_item_id).await
}

pub async fn commit_package_stage_traced(
    trace: TraceContext,
    collection: CollectionKind,
    remote_item_id: InlineText<REMOTE_ITEM_ID_MAX_BYTES>,
) -> Result<CollectionManifestState, StorageError> {
    if !STORAGE_AVAILABLE.load(AtomicOrdering::Relaxed) {
        return Err(StorageError::Unavailable);
    }
    let command = StorageCommand::CommitPackageStage {
        trace,
        collection,
        remote_item_id,
    };
    STORAGE_CMD_CH.send(command).await;
    storage_queue_on_enqueue(trace, "commit_stage", 0);

    match STORAGE_RESP_SIG.wait().await {
        StorageResponse::Snapshot(result) => result.map(|snapshot| *snapshot),
        StorageResponse::CommitAndOpenPackage(_)
        | StorageResponse::Opened(_)
        | StorageResponse::OpenedPackage(_)
        | StorageResponse::LoadedWindow(_)
        | StorageResponse::Unit(_) => Err(StorageError::Unavailable),
    }
}

pub async fn abort_package_stage() -> Result<(), StorageError> {
    abort_package_stage_traced(TraceContext::none()).await
}

pub async fn abort_package_stage_traced(trace: TraceContext) -> Result<(), StorageError> {
    if !STORAGE_AVAILABLE.load(AtomicOrdering::Relaxed) {
        return Err(StorageError::Unavailable);
    }
    let command = StorageCommand::AbortPackageStage { trace };
    STORAGE_CMD_CH.send(command).await;
    storage_queue_on_enqueue(trace, "abort_stage", 0);

    match STORAGE_RESP_SIG.wait().await {
        StorageResponse::Unit(result) => result,
        StorageResponse::CommitAndOpenPackage(_)
        | StorageResponse::Opened(_)
        | StorageResponse::OpenedPackage(_)
        | StorageResponse::LoadedWindow(_)
        | StorageResponse::Snapshot(_) => Err(StorageError::Unavailable),
    }
}

pub async fn update_package_state(
    collection: CollectionKind,
    remote_item_id: InlineText<REMOTE_ITEM_ID_MAX_BYTES>,
    package_state: PackageState,
) -> Result<CollectionManifestState, StorageError> {
    update_package_state_traced(
        TraceContext::none(),
        collection,
        remote_item_id,
        package_state,
    )
    .await
}

pub async fn update_package_state_traced(
    trace: TraceContext,
    collection: CollectionKind,
    remote_item_id: InlineText<REMOTE_ITEM_ID_MAX_BYTES>,
    package_state: PackageState,
) -> Result<CollectionManifestState, StorageError> {
    if !STORAGE_AVAILABLE.load(AtomicOrdering::Relaxed) {
        return Err(StorageError::Unavailable);
    }
    let command = StorageCommand::UpdatePackageState {
        trace,
        collection,
        remote_item_id,
        package_state,
    };
    STORAGE_CMD_CH.send(command).await;
    storage_queue_on_enqueue(trace, "update_package_state", 0);

    match STORAGE_RESP_SIG.wait().await {
        StorageResponse::Snapshot(result) => result.map(|snapshot| *snapshot),
        StorageResponse::CommitAndOpenPackage(_)
        | StorageResponse::Opened(_)
        | StorageResponse::OpenedPackage(_)
        | StorageResponse::LoadedWindow(_)
        | StorageResponse::Unit(_) => Err(StorageError::Unavailable),
    }
}

pub async fn open_cached_reader_package(
    content_id: InlineText<CONTENT_ID_MAX_BYTES>,
) -> Result<Box<OpenedReaderPackage>, StorageError> {
    open_cached_reader_package_traced(TraceContext::none(), content_id).await
}

pub async fn open_cached_reader_package_traced(
    trace: TraceContext,
    content_id: InlineText<CONTENT_ID_MAX_BYTES>,
) -> Result<Box<OpenedReaderPackage>, StorageError> {
    if !STORAGE_AVAILABLE.load(AtomicOrdering::Relaxed) {
        return Err(StorageError::Unavailable);
    }
    let started_at = Instant::now();
    let command = StorageCommand::OpenCachedReaderPackage { trace, content_id };
    STORAGE_CMD_CH.send(command).await;
    storage_queue_on_enqueue(trace, "open_cached_reader_package", 0);

    let result = match STORAGE_RESP_SIG.wait().await {
        StorageResponse::OpenedPackage(result) => result,
        StorageResponse::CommitAndOpenPackage(_)
        | StorageResponse::Snapshot(_)
        | StorageResponse::Opened(_)
        | StorageResponse::LoadedWindow(_)
        | StorageResponse::Unit(_) => Err(StorageError::Unavailable),
    };

    let total_ms = Instant::now().duration_since(started_at).as_millis();
    match &result {
        Ok(opened) => info!(
            "content storage cached open timing content_id={} total_ms={} total_units={} total_paragraphs={} window_units={}",
            content_id.as_str(),
            total_ms,
            opened.total_units,
            opened.paragraphs.len(),
            opened.window.unit_count,
        ),
        Err(err) => info!(
            "content storage cached open timing failed content_id={} total_ms={} err={:?}",
            content_id.as_str(),
            total_ms,
            err,
        ),
    }

    result
}

pub async fn load_reader_window(
    content_id: InlineText<CONTENT_ID_MAX_BYTES>,
    window_start_unit_index: u32,
) -> Result<Box<ReaderWindow>, StorageError> {
    load_reader_window_traced(TraceContext::none(), content_id, window_start_unit_index).await
}

pub async fn load_reader_window_traced(
    trace: TraceContext,
    content_id: InlineText<CONTENT_ID_MAX_BYTES>,
    window_start_unit_index: u32,
) -> Result<Box<ReaderWindow>, StorageError> {
    if !STORAGE_AVAILABLE.load(AtomicOrdering::Relaxed) {
        return Err(StorageError::Unavailable);
    }
    let command = StorageCommand::LoadReaderWindow {
        trace,
        content_id,
        window_start_unit_index,
    };
    STORAGE_CMD_CH.send(command).await;
    storage_queue_on_enqueue(trace, "load_reader_window", 0);

    match STORAGE_RESP_SIG.wait().await {
        StorageResponse::LoadedWindow(result) => result,
        StorageResponse::CommitAndOpenPackage(_)
        | StorageResponse::Snapshot(_)
        | StorageResponse::OpenedPackage(_)
        | StorageResponse::Opened(_)
        | StorageResponse::Unit(_) => Err(StorageError::Unavailable),
    }
}

fn storage_now_ms() -> u64 {
    Instant::now().as_millis()
}

fn storage_elapsed_since_ms(started_at_ms: u64) -> u64 {
    storage_now_ms().saturating_sub(started_at_ms)
}

fn read_dir_usage(dir: &SdDirectory<'_, '_>) -> Result<DirUsage, StorageError> {
    let mut usage = DirUsage { files: 0, bytes: 0 };
    dir.iterate_dir(|entry| {
        if !entry.attributes.is_directory() {
            usage.files = usage.files.saturating_add(1);
            usage.bytes = usage.bytes.saturating_add(entry.size as u64);
        }
    })
    .map_err(map_sd_error)?;
    Ok(usage)
}

fn read_fat_volume_free_bytes<D>(device: &mut D) -> Result<(u64, bool, u32), StorageError>
where
    D: BlockDevice,
    D::Error: core::fmt::Debug,
{
    let mut block = [Block::new()];
    device
        .read(&mut block, BlockIdx(0))
        .map_err(|_| StorageError::Unavailable)?;
    let partition = &block[0].contents;
    let lba_start = read_le_u32(partition, 446 + 8).ok_or(StorageError::CorruptData)?;

    device
        .read(&mut block, BlockIdx(lba_start))
        .map_err(|_| StorageError::Unavailable)?;
    let bpb = &block[0].contents;
    let bytes_per_sector = read_le_u16(bpb, 11).ok_or(StorageError::CorruptData)? as u32;
    let sectors_per_cluster = u32::from(*bpb.get(13).ok_or(StorageError::CorruptData)?);
    let reserved_sector_count = read_le_u16(bpb, 14).ok_or(StorageError::CorruptData)? as u32;
    let fat_count = u32::from(*bpb.get(16).ok_or(StorageError::CorruptData)?);
    let root_entry_count = read_le_u16(bpb, 17).ok_or(StorageError::CorruptData)? as u32;
    let total_sectors16 = read_le_u16(bpb, 19).ok_or(StorageError::CorruptData)? as u32;
    let total_sectors = if total_sectors16 != 0 {
        total_sectors16
    } else {
        read_le_u32(bpb, 32).ok_or(StorageError::CorruptData)?
    };
    let fat_size16 = read_le_u16(bpb, 22).ok_or(StorageError::CorruptData)? as u32;
    let fat_size = if fat_size16 != 0 {
        fat_size16
    } else {
        read_le_u32(bpb, 36).ok_or(StorageError::CorruptData)?
    };
    let root_dir_sectors = ((root_entry_count * 32)
        .saturating_add(bytes_per_sector.saturating_sub(1)))
        / bytes_per_sector;
    let data_sectors = total_sectors
        .checked_sub(
            reserved_sector_count
                .saturating_add(fat_count.saturating_mul(fat_size))
                .saturating_add(root_dir_sectors),
        )
        .ok_or(StorageError::CorruptData)?;
    if sectors_per_cluster == 0 {
        return Err(StorageError::CorruptData);
    }
    let cluster_count = data_sectors / sectors_per_cluster;
    let cluster_size_bytes = bytes_per_sector.saturating_mul(sectors_per_cluster);

    if cluster_count < 65_525 {
        return Ok((0, false, cluster_size_bytes));
    }

    let fs_info_sector = read_le_u16(bpb, 48).ok_or(StorageError::CorruptData)? as u32;
    if fs_info_sector == 0 || fs_info_sector == 0xFFFF {
        return Ok((0, false, cluster_size_bytes));
    }

    device
        .read(
            &mut block,
            BlockIdx(lba_start.saturating_add(fs_info_sector)),
        )
        .map_err(|_| StorageError::Unavailable)?;
    let info = &block[0].contents;
    if read_le_u32(info, 0) != Some(0x4161_5252)
        || read_le_u32(info, 484) != Some(0x6141_7272)
        || read_le_u32(info, 508) != Some(0xAA55_0000)
    {
        return Ok((0, false, cluster_size_bytes));
    }

    let free_clusters = read_le_u32(info, 488).ok_or(StorageError::CorruptData)?;
    if free_clusters == u32::MAX {
        return Ok((0, false, cluster_size_bytes));
    }

    Ok((
        (free_clusters as u64).saturating_mul(cluster_size_bytes as u64),
        true,
        cluster_size_bytes,
    ))
}

fn read_le_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    let slice = bytes.get(offset..offset + 2)?;
    Some(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_le_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let slice = bytes.get(offset..offset + 4)?;
    Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

pub async fn open_cached_reader_content(
    content_id: InlineText<CONTENT_ID_MAX_BYTES>,
) -> Result<Box<OpenedReaderContent>, StorageError> {
    open_cached_reader_content_traced(TraceContext::none(), content_id).await
}

pub async fn open_cached_reader_content_traced(
    trace: TraceContext,
    content_id: InlineText<CONTENT_ID_MAX_BYTES>,
) -> Result<Box<OpenedReaderContent>, StorageError> {
    if !STORAGE_AVAILABLE.load(AtomicOrdering::Relaxed) {
        return Err(StorageError::Unavailable);
    }
    let started_at = Instant::now();
    let command = StorageCommand::OpenCachedReaderContent { trace, content_id };
    STORAGE_CMD_CH.send(command).await;
    storage_queue_on_enqueue(trace, "open_cached_reader_content", 0);

    let result = match STORAGE_RESP_SIG.wait().await {
        StorageResponse::Opened(result) => result,
        StorageResponse::CommitAndOpenPackage(_)
        | StorageResponse::Snapshot(_)
        | StorageResponse::OpenedPackage(_)
        | StorageResponse::LoadedWindow(_)
        | StorageResponse::Unit(_) => Err(StorageError::Unavailable),
    };

    let total_ms = Instant::now().duration_since(started_at).as_millis();
    match &result {
        Ok(opened) => info!(
            "content storage cached open timing content_id={} total_ms={} unit_count={} paragraph_count={} truncated={}",
            content_id.as_str(),
            total_ms,
            opened.document.unit_count,
            opened.document.paragraph_count,
            opened.truncated,
        ),
        Err(err) => info!(
            "content storage cached open timing failed content_id={} total_ms={} err={:?}",
            content_id.as_str(),
            total_ms,
            err,
        ),
    }

    result
}

pub(crate) fn parse_reader_content_bytes(
    bytes: &[u8],
) -> Result<OpenedReaderContent, StorageError> {
    let source = SliceJsonSource::new(bytes);
    parse_opened_reader_content(source)
}

#[embassy_executor::task]
async fn content_storage_task(mut storage: Box<SdContentStorage<'static>>) {
    // Defer snapshot publication until backend sync updates the app store. Preloading
    // non-empty SD manifests at boot materially increases heap pressure before the first
    // auth/TLS exchange.
    loop {
        let command = STORAGE_CMD_CH.receive().await;
        storage_queue_on_dequeue(&command);
        let response = match command {
            StorageCommand::PersistSnapshot {
                trace,
                kind,
                snapshot,
            } => StorageResponse::Snapshot(
                storage
                    .persist_snapshot(trace, kind, *snapshot)
                    .map(Box::new),
            ),
            StorageCommand::BeginPackageStage {
                trace,
                content_id,
                remote_revision,
            } => StorageResponse::Unit(storage.begin_stage(trace, content_id, remote_revision)),
            StorageCommand::WritePackageChunk { trace, len, bytes } => {
                storage.queue_stage_chunk(trace, bytes.as_slice(len));
                continue;
            }
            StorageCommand::CommitPackageStage {
                trace,
                collection,
                remote_item_id,
            } => StorageResponse::Snapshot(
                storage
                    .commit_stage(trace, collection, remote_item_id)
                    .map(Box::new),
            ),
            StorageCommand::CommitAndOpenPackageStage {
                trace,
                collection,
                remote_item_id,
                content_id,
            } => StorageResponse::CommitAndOpenPackage(
                storage
                    .commit_stage(trace, collection, remote_item_id)
                    .map(Box::new)
                    .map(|snapshot| {
                        Box::new(CommitAndOpenPackageResult {
                            snapshot,
                            opened: storage
                                .open_cached_reader_package(trace, content_id)
                                .map(Box::new),
                        })
                    }),
            ),
            StorageCommand::AbortPackageStage { trace } => {
                StorageResponse::Unit(storage.abort_stage(trace))
            }
            StorageCommand::UpdatePackageState {
                trace,
                collection,
                remote_item_id,
                package_state,
            } => StorageResponse::Snapshot(
                storage
                    .update_manifest_item_state(trace, collection, remote_item_id, package_state)
                    .map(Box::new),
            ),
            StorageCommand::OpenCachedReaderPackage { trace, content_id } => {
                StorageResponse::OpenedPackage(
                    storage
                        .open_cached_reader_package(trace, content_id)
                        .map(Box::new),
                )
            }
            StorageCommand::LoadReaderWindow {
                trace,
                content_id,
                window_start_unit_index,
            } => StorageResponse::LoadedWindow(storage.load_reader_window(
                trace,
                content_id,
                window_start_unit_index,
            )),
            StorageCommand::OpenCachedReaderContent { trace, content_id } => {
                StorageResponse::Opened(
                    storage
                        .open_cached_reader_content(trace, content_id)
                        .map(Box::new),
                )
            }
        };

        STORAGE_RESP_SIG.signal(response);
    }
}

impl<'d> SdContentStorage<'d> {
    fn snapshot(&self, kind: CollectionKind) -> CollectionManifestState {
        self.snapshots[collection_index(kind)]
            .as_deref()
            .copied()
            .unwrap_or_else(CollectionManifestState::empty)
    }

    fn snapshot_mut(&mut self, kind: CollectionKind) -> &mut CollectionManifestState {
        self.snapshots[collection_index(kind)]
            .get_or_insert_with(|| Box::new(CollectionManifestState::empty()))
            .as_mut()
    }

    fn set_snapshot(&mut self, kind: CollectionKind, snapshot: CollectionManifestState) {
        self.snapshots[collection_index(kind)] = if snapshot.is_empty() {
            None
        } else {
            Some(Box::new(snapshot))
        };
    }

    fn initialize_layout(&mut self) -> Result<(), StorageError> {
        let volume = self
            .volume_mgr
            .open_volume(VolumeIdx(0))
            .map_err(map_sd_error)?;
        let root = volume.open_root_dir().map_err(map_sd_error)?;
        let motif = open_or_create_dir(&root, ROOT_DIR_NAME)?;
        let v1 = open_or_create_dir(&motif, VERSION_DIR_NAME)?;
        let _ = open_or_create_dir(&v1, MANIFEST_DIR_NAME)?;
        let _ = open_or_create_dir(&v1, PACKAGE_DIR_NAME)?;
        let _ = open_or_create_dir(&v1, STAGING_DIR_NAME)?;
        let _ = open_or_create_dir(&v1, CACHE_DIR_NAME)?;
        Ok(())
    }

    fn load_state(&mut self) -> Result<(), StorageError> {
        self.cleanup_active_stage_file()?;
        self.cache_index = self.read_cache_index()?.unwrap_or(CacheIndex::empty());
        self.cleanup_orphan_package_slots()?;

        let saved = self
            .read_manifest_snapshot(CollectionKind::Saved)?
            .unwrap_or(CollectionManifestState::empty());
        let inbox = self
            .read_manifest_snapshot(CollectionKind::Inbox)?
            .unwrap_or(CollectionManifestState::empty());
        let recommendations = self
            .read_manifest_snapshot(CollectionKind::Recommendations)?
            .unwrap_or(CollectionManifestState::empty());

        self.set_snapshot(CollectionKind::Saved, saved);
        self.set_snapshot(CollectionKind::Inbox, inbox);
        self.set_snapshot(CollectionKind::Recommendations, recommendations);

        self.reconcile_all_snapshots();
        self.refresh_collection_flags();
        self.evict_if_needed(TraceContext::none())?;
        Ok(())
    }

    fn reset_dev_data(&mut self) -> Result<(), StorageError> {
        let volume = self
            .volume_mgr
            .open_volume(VolumeIdx(0))
            .map_err(map_sd_error)?;
        let root = volume.open_root_dir().map_err(map_sd_error)?;
        let motif = open_or_replace_dir(&root, ROOT_DIR_NAME)?;
        let v1 = open_or_replace_dir(&motif, VERSION_DIR_NAME)?;

        clear_or_recreate_dir(&v1, MANIFEST_DIR_NAME)?;
        clear_or_recreate_dir(&v1, CACHE_DIR_NAME)?;
        clear_or_recreate_dir(&v1, STAGING_DIR_NAME)?;
        clear_or_recreate_dir(&v1, PACKAGE_DIR_NAME)?;

        self.snapshots = [None, None, None];
        self.cache_index = CacheIndex::empty();
        self.pending_stage = None;
        Ok(())
    }

    fn persist_snapshot(
        &mut self,
        trace: TraceContext,
        kind: CollectionKind,
        snapshot: CollectionManifestState,
    ) -> Result<CollectionManifestState, StorageError> {
        self.set_snapshot(kind, snapshot);
        self.reconcile_snapshot(kind);
        self.refresh_collection_flags();
        self.write_manifest_snapshot(kind)?;
        self.evict_if_needed(TraceContext::none())?;
        let snapshot = self.snapshot(kind);
        let metrics = self.storage_space_metrics()?;
        crate::memtrace!(
            "storage_snapshot",
            "component" = "storage",
            "at_ms" = storage_now_ms(),
            "sync_id" = trace.sync_id,
            "req_id" = trace.req_id,
            "collection" = collection_label(kind),
            "item_count" = snapshot.len(),
            "sd_total_bytes" = metrics.sd_total_bytes,
            "sd_free_bytes" = metrics.sd_free_bytes,
            "sd_free_known" = bool_flag(metrics.sd_free_known),
            "motif_total_bytes" = metrics.motif_total_bytes,
            "manifest_bytes" = metrics.manifest_bytes,
            "cache_bytes" = metrics.cache_bytes,
            "stage_bytes" = metrics.stage_bytes,
            "package_bytes" = metrics.package_bytes,
            "cache_entries" = metrics.cache_entry_count,
            "cache_budget_remaining" = metrics.cache_budget_remaining,
        );
        Ok(snapshot)
    }

    fn begin_stage(
        &mut self,
        trace: TraceContext,
        content_id: InlineText<CONTENT_ID_MAX_BYTES>,
        remote_revision: u64,
    ) -> Result<(), StorageError> {
        let superseded_entry = self.cache_index.find_by_content_id(&content_id);
        let overwritten_entry = match self.cache_index.next_available_slot_id() {
            Some(_) => None,
            None => self
                .select_eviction_candidate_excluding(superseded_entry.map(|entry| entry.slot_id)),
        };
        let slot_id = if let Some(slot_id) = self.cache_index.next_available_slot_id() {
            slot_id
        } else if let Some(entry) = overwritten_entry {
            entry.slot_id
        } else {
            return Err(StorageError::PartitionFull);
        };

        let direct_to_package = overwritten_entry.is_none();
        let (target_kind, stage_volume, stage_file) =
            self.open_pending_stage_writer(slot_id, direct_to_package)?;
        let metrics = self.storage_space_metrics_for_open_volume(stage_volume)?;
        info!(
            "content storage stage begin content_id={} revision={} slot={} target={} superseded_slot={:?} overwritten_slot={:?} cache_entries={} cache_bytes={}",
            content_id.as_str(),
            remote_revision,
            slot_id,
            target_kind.label(),
            superseded_entry.map(|entry| entry.slot_id),
            overwritten_entry.map(|entry| entry.slot_id),
            self.cache_index.len(),
            self.cache_index.total_bytes(),
        );
        self.pending_stage_error = None;
        self.pending_stage = Some(PendingStage {
            trace,
            content_id,
            remote_revision,
            slot_id,
            target_kind,
            stage_volume,
            stage_file,
            bytes_written: 0,
            flushed_bytes: 0,
            crc32: 0xFFFF_FFFF,
            started_at_ms: storage_now_ms(),
            overwritten_entry,
            superseded_entry,
        });
        crate::memtrace!(
            "storage_stage",
            "component" = "storage",
            "at_ms" = storage_now_ms(),
            "action" = "begin",
            "sync_id" = trace.sync_id,
            "req_id" = trace.req_id,
            "content_id" = content_id.as_str(),
            "slot_id" = slot_id,
            "target" = target_kind.label(),
            "remote_revision" = remote_revision,
            "cache_entries" = self.cache_index.len(),
            "cache_bytes" = self.cache_index.total_bytes(),
            "superseded_slot" = superseded_entry.map(|entry| entry.slot_id).unwrap_or(0),
            "overwritten_slot" = overwritten_entry.map(|entry| entry.slot_id).unwrap_or(0),
            "sd_total_bytes" = metrics.sd_total_bytes,
            "sd_free_bytes" = metrics.sd_free_bytes,
            "sd_free_known" = bool_flag(metrics.sd_free_known),
            "motif_total_bytes" = metrics.motif_total_bytes,
            "stage_bytes" = metrics.stage_bytes,
            "package_bytes" = metrics.package_bytes,
            "cache_budget_remaining" = metrics.cache_budget_remaining,
        );
        Ok(())
    }

    fn write_stage_chunk(
        &mut self,
        _trace: TraceContext,
        chunk: &[u8],
    ) -> Result<(), StorageError> {
        let Some(mut stage) = self.pending_stage else {
            return Err(StorageError::Unavailable);
        };
        let trace = stage.trace;

        self.append_stage_file(stage.stage_file, chunk)?;
        stage.bytes_written = stage.bytes_written.saturating_add(chunk.len() as u32);
        stage.crc32 = crc32_continue(stage.crc32, chunk);
        let dirty_bytes = stage.bytes_written.saturating_sub(stage.flushed_bytes);
        if dirty_bytes >= STAGE_FLUSH_INTERVAL_BYTES {
            self.flush_stage_file(stage.stage_file)?;
            stage.flushed_bytes = stage.bytes_written;
            let metrics = self.storage_space_metrics_for_open_volume(stage.stage_volume)?;
            info!(
                "content storage stage flush content_id={} slot={} bytes_written={} dirty_bytes={} elapsed_ms={}",
                stage.content_id.as_str(),
                stage.slot_id,
                stage.bytes_written,
                dirty_bytes,
                storage_elapsed_since_ms(stage.started_at_ms),
            );
            crate::memtrace!(
                "storage_stage",
                "component" = "storage",
                "at_ms" = storage_now_ms(),
                "action" = "flush",
                "sync_id" = trace.sync_id,
                "req_id" = trace.req_id,
                "content_id" = stage.content_id.as_str(),
                "slot_id" = stage.slot_id,
                "bytes_written" = stage.bytes_written,
                "dirty_bytes" = dirty_bytes,
                "elapsed_ms" = storage_elapsed_since_ms(stage.started_at_ms),
                "sd_total_bytes" = metrics.sd_total_bytes,
                "sd_free_bytes" = metrics.sd_free_bytes,
                "sd_free_known" = bool_flag(metrics.sd_free_known),
                "stage_bytes" = metrics.stage_bytes,
                "package_bytes" = metrics.package_bytes,
                "motif_total_bytes" = metrics.motif_total_bytes,
            );
        }
        let crossed_progress_boundary = if stage.bytes_written == chunk.len() as u32 {
            true
        } else {
            stage.bytes_written / STAGE_PROGRESS_LOG_INTERVAL_BYTES
                != (stage.bytes_written.saturating_sub(chunk.len() as u32))
                    / STAGE_PROGRESS_LOG_INTERVAL_BYTES
        };
        if crossed_progress_boundary {
            let metrics = self.storage_space_metrics_for_open_volume(stage.stage_volume)?;
            info!(
                "content storage stage progress content_id={} slot={} bytes_written={} chunk_len={} elapsed_ms={}",
                stage.content_id.as_str(),
                stage.slot_id,
                stage.bytes_written,
                chunk.len(),
                storage_elapsed_since_ms(stage.started_at_ms),
            );
            crate::memtrace!(
                "storage_stage",
                "component" = "storage",
                "at_ms" = storage_now_ms(),
                "action" = "progress",
                "sync_id" = trace.sync_id,
                "req_id" = trace.req_id,
                "content_id" = stage.content_id.as_str(),
                "slot_id" = stage.slot_id,
                "bytes_written" = stage.bytes_written,
                "chunk_len" = chunk.len(),
                "elapsed_ms" = storage_elapsed_since_ms(stage.started_at_ms),
                "sd_total_bytes" = metrics.sd_total_bytes,
                "sd_free_bytes" = metrics.sd_free_bytes,
                "sd_free_known" = bool_flag(metrics.sd_free_known),
                "stage_bytes" = metrics.stage_bytes,
                "package_bytes" = metrics.package_bytes,
                "motif_total_bytes" = metrics.motif_total_bytes,
            );
        }
        self.pending_stage = Some(stage);
        Ok(())
    }

    fn queue_stage_chunk(&mut self, trace: TraceContext, chunk: &[u8]) {
        if self.pending_stage_error.is_some() {
            return;
        }

        if let Err(err) = self.write_stage_chunk(trace, chunk) {
            self.record_pending_stage_error(trace, err);
        }
    }

    fn commit_stage(
        &mut self,
        _trace: TraceContext,
        collection: CollectionKind,
        remote_item_id: InlineText<REMOTE_ITEM_ID_MAX_BYTES>,
    ) -> Result<CollectionManifestState, StorageError> {
        let Some(stage) = self.pending_stage.take() else {
            return Err(StorageError::Unavailable);
        };
        let trace = stage.trace;
        if let Some(err) = self.pending_stage_error.take() {
            self.cleanup_pending_stage_target(&stage)?;
            info!(
                "content storage stage commit failed content_id={} slot={} target={} err={:?}",
                stage.content_id.as_str(),
                stage.slot_id,
                stage.target_kind.label(),
                err,
            );
            crate::memtrace!(
                "storage_stage",
                "component" = "storage",
                "at_ms" = storage_now_ms(),
                "action" = "error",
                "sync_id" = trace.sync_id,
                "req_id" = trace.req_id,
                "content_id" = stage.content_id.as_str(),
                "slot_id" = stage.slot_id,
                "target" = stage.target_kind.label(),
                "error" = storage_error_label(err),
                "bytes_written" = stage.bytes_written,
                "elapsed_ms" = storage_elapsed_since_ms(stage.started_at_ms),
            );
            return Err(err);
        }

        if stage.bytes_written != stage.flushed_bytes {
            self.flush_stage_file(stage.stage_file)?;
            let metrics = self.storage_space_metrics_for_open_volume(stage.stage_volume)?;
            let dirty_bytes = stage.bytes_written.saturating_sub(stage.flushed_bytes);
            info!(
                "content storage stage flush content_id={} slot={} bytes_written={} dirty_bytes={} elapsed_ms={}",
                stage.content_id.as_str(),
                stage.slot_id,
                stage.bytes_written,
                dirty_bytes,
                storage_elapsed_since_ms(stage.started_at_ms),
            );
            crate::memtrace!(
                "storage_stage",
                "component" = "storage",
                "at_ms" = storage_now_ms(),
                "action" = "flush",
                "sync_id" = trace.sync_id,
                "req_id" = trace.req_id,
                "content_id" = stage.content_id.as_str(),
                "slot_id" = stage.slot_id,
                "bytes_written" = stage.bytes_written,
                "dirty_bytes" = dirty_bytes,
                "elapsed_ms" = storage_elapsed_since_ms(stage.started_at_ms),
                "sd_total_bytes" = metrics.sd_total_bytes,
                "sd_free_bytes" = metrics.sd_free_bytes,
                "sd_free_known" = bool_flag(metrics.sd_free_known),
                "stage_bytes" = metrics.stage_bytes,
                "package_bytes" = metrics.package_bytes,
                "motif_total_bytes" = metrics.motif_total_bytes,
            );
        }
        self.close_stage_writer(&stage)?;
        let header = match stage.target_kind {
            PendingStageTargetKind::StagingFile => {
                self.validate_staged_reader_package(stage.bytes_written)?
            }
            PendingStageTargetKind::PackageSlot => {
                self.validate_package_slot_reader_package(stage.slot_id, stage.bytes_written)?
            }
        };

        let copied_bytes = match stage.target_kind {
            PendingStageTargetKind::StagingFile => {
                let copied = self.copy_stage_to_package_slot(stage.slot_id)?;
                self.delete_stage_file()?;
                copied
            }
            PendingStageTargetKind::PackageSlot => 0,
        };
        self.write_package_meta(
            stage.slot_id,
            stage.remote_revision,
            stage.bytes_written,
            !stage.crc32,
        )?;

        if let Some(entry) = stage.overwritten_entry {
            let _ = self.cache_index.remove_slot(entry.slot_id);
        }
        if let Some(entry) = stage.superseded_entry
            && entry.slot_id != stage.slot_id
        {
            if let Err(err) = self.delete_package_slot(entry.slot_id) {
                info!(
                    "content storage stale package cleanup failed slot={} err={:?}",
                    entry.slot_id, err
                );
            }
            let _ = self.cache_index.remove_slot(entry.slot_id);
        }

        self.cache_index.upsert(CacheEntry {
            slot_id: stage.slot_id,
            content_id: stage.content_id,
            remote_revision: stage.remote_revision,
            size_bytes: stage.bytes_written,
            crc32: !stage.crc32,
            last_touch_seq: 0,
            collection_flags: 0,
        });
        self.refresh_collection_flags();
        self.write_cache_index()?;
        let snapshot = self.update_manifest_item_state(
            trace,
            collection,
            remote_item_id,
            PackageState::Cached,
        )?;
        self.evict_if_needed(trace)?;
        let metrics = self.storage_space_metrics()?;
        info!(
            "content storage stage commit content_id={} slot={} target={} bytes_written={} crc32=0x{:08x} collection={:?} remote_item_id={} overwritten_slot={:?} superseded_slot={:?} total_units={} paragraphs={} elapsed_ms={}",
            stage.content_id.as_str(),
            stage.slot_id,
            stage.target_kind.label(),
            stage.bytes_written,
            !stage.crc32,
            collection,
            remote_item_id.as_str(),
            stage.overwritten_entry.map(|entry| entry.slot_id),
            stage.superseded_entry.map(|entry| entry.slot_id),
            header.unit_count,
            header.paragraph_count,
            storage_elapsed_since_ms(stage.started_at_ms),
        );
        crate::memtrace!(
            "storage_stage",
            "component" = "storage",
            "at_ms" = storage_now_ms(),
            "action" = "commit",
            "sync_id" = trace.sync_id,
            "req_id" = trace.req_id,
            "collection" = collection_label(collection),
            "content_id" = stage.content_id.as_str(),
            "remote_item_id" = remote_item_id.as_str(),
            "slot_id" = stage.slot_id,
            "target" = stage.target_kind.label(),
            "bytes_written" = stage.bytes_written,
            "copied_bytes" = copied_bytes,
            "crc32" = !stage.crc32,
            "total_units" = header.unit_count,
            "paragraph_count" = header.paragraph_count,
            "elapsed_ms" = storage_elapsed_since_ms(stage.started_at_ms),
            "sd_total_bytes" = metrics.sd_total_bytes,
            "sd_free_bytes" = metrics.sd_free_bytes,
            "sd_free_known" = bool_flag(metrics.sd_free_known),
            "motif_total_bytes" = metrics.motif_total_bytes,
            "stage_bytes" = metrics.stage_bytes,
            "package_bytes" = metrics.package_bytes,
            "cache_budget_remaining" = metrics.cache_budget_remaining,
        );
        Ok(snapshot)
    }

    fn abort_stage(&mut self, _trace: TraceContext) -> Result<(), StorageError> {
        if let Some(stage) = self.pending_stage {
            let trace = stage.trace;
            let metrics = self.storage_space_metrics_for_open_volume(stage.stage_volume)?;
            info!(
                "content storage stage abort content_id={} slot={} target={} bytes_written={} elapsed_ms={}",
                stage.content_id.as_str(),
                stage.slot_id,
                stage.target_kind.label(),
                stage.bytes_written,
                storage_elapsed_since_ms(stage.started_at_ms),
            );
            crate::memtrace!(
                "storage_stage",
                "component" = "storage",
                "at_ms" = storage_now_ms(),
                "action" = "abort",
                "sync_id" = trace.sync_id,
                "req_id" = trace.req_id,
                "content_id" = stage.content_id.as_str(),
                "slot_id" = stage.slot_id,
                "target" = stage.target_kind.label(),
                "bytes_written" = stage.bytes_written,
                "elapsed_ms" = storage_elapsed_since_ms(stage.started_at_ms),
                "sd_total_bytes" = metrics.sd_total_bytes,
                "sd_free_bytes" = metrics.sd_free_bytes,
                "sd_free_known" = bool_flag(metrics.sd_free_known),
                "stage_bytes" = metrics.stage_bytes,
                "package_bytes" = metrics.package_bytes,
                "motif_total_bytes" = metrics.motif_total_bytes,
            );
            self.cleanup_pending_stage_target(&stage)?;
        }
        self.pending_stage = None;
        self.pending_stage_error = None;
        Ok(())
    }

    fn update_manifest_item_state(
        &mut self,
        trace: TraceContext,
        collection: CollectionKind,
        remote_item_id: InlineText<REMOTE_ITEM_ID_MAX_BYTES>,
        package_state: PackageState,
    ) -> Result<CollectionManifestState, StorageError> {
        {
            let snapshot = self.snapshot_mut(collection);
            let _ = snapshot.update_package_state(&remote_item_id, package_state);
        }
        self.write_manifest_snapshot(collection)?;
        let snapshot = self.snapshot(collection);
        crate::memtrace!(
            "storage_package_state",
            "component" = "storage",
            "at_ms" = storage_now_ms(),
            "sync_id" = trace.sync_id,
            "req_id" = trace.req_id,
            "collection" = collection_label(collection),
            "remote_item_id" = remote_item_id.as_str(),
            "package_state" = package_state as u8,
            "item_count" = snapshot.len(),
        );
        Ok(snapshot)
    }

    fn open_cached_reader_content(
        &mut self,
        trace: TraceContext,
        content_id: InlineText<CONTENT_ID_MAX_BYTES>,
    ) -> Result<OpenedReaderContent, StorageError> {
        let started_at = Instant::now();
        let entry = self
            .cache_index
            .find_by_content_id(&content_id)
            .ok_or(StorageError::Unavailable)?;
        let meta = self.read_package_meta(entry.slot_id)?;
        if meta.remote_revision != entry.remote_revision {
            info!(
                "content storage cached content revision mismatch content_id={} slot={} entry_revision={} meta_revision={}",
                content_id.as_str(),
                entry.slot_id,
                entry.remote_revision,
                meta.remote_revision,
            );
            return Err(StorageError::CorruptData);
        }

        let opened = {
            let volume = self
                .volume_mgr
                .open_volume(VolumeIdx(0))
                .map_err(map_sd_error)?;
            let root = volume.open_root_dir().map_err(map_sd_error)?;
            let motif = root.open_dir(ROOT_DIR_NAME).map_err(map_sd_error)?;
            let v1 = motif.open_dir(VERSION_DIR_NAME).map_err(map_sd_error)?;
            let pkg_dir = v1.open_dir(PACKAGE_DIR_NAME).map_err(map_sd_error)?;
            let file = pkg_dir
                .open_file_in_dir(
                    package_payload_file_name(entry.slot_id).as_str(),
                    Mode::ReadOnly,
                )
                .map_err(map_sd_error)?;

            let mut source = SdPackageSource::new(file);
            let parse_started_at = Instant::now();
            let opened = match parse_opened_reader_content(&mut source) {
                Ok(opened) => opened,
                Err(err) => {
                    let _ = source.finish();
                    info!(
                        "content storage cached content parse failed content_id={} slot={} bytes_read={} crc32=0x{:08x} err={:?}",
                        content_id.as_str(),
                        entry.slot_id,
                        source.bytes_read(),
                        source.crc32(),
                        err,
                    );
                    crate::memtrace!(
                        "reader_open",
                        "component" = "storage",
                        "at_ms" = storage_now_ms(),
                        "action" = "parse_failed",
                        "sync_id" = trace.sync_id,
                        "req_id" = trace.req_id,
                        "content_id" = content_id.as_str(),
                        "slot_id" = entry.slot_id,
                        "bytes_read" = source.bytes_read(),
                        "expected_bytes" = meta.size_bytes,
                        "expected_crc32" = meta.crc32,
                        "actual_crc32" = source.crc32(),
                    );
                    return Err(err);
                }
            };
            let parse_ms = Instant::now().duration_since(parse_started_at).as_millis();
            source.finish()?;
            if source.bytes_read() != meta.size_bytes as usize {
                info!(
                    "content storage cached content size mismatch content_id={} slot={} expected={} actual={}",
                    content_id.as_str(),
                    entry.slot_id,
                    meta.size_bytes,
                    source.bytes_read(),
                );
                return Err(StorageError::CorruptData);
            }
            if source.crc32() != meta.crc32 {
                info!(
                    "content storage cached content crc mismatch content_id={} slot={} expected=0x{:08x} actual=0x{:08x}",
                    content_id.as_str(),
                    entry.slot_id,
                    meta.crc32,
                    source.crc32(),
                );
                return Err(StorageError::CorruptData);
            }
            let total_ms = Instant::now().duration_since(started_at).as_millis();
            info!(
                "content storage cached parse timing content_id={} slot={} bytes_read={} parse_ms={} total_ms={} unit_count={} paragraph_count={} truncated={}",
                content_id.as_str(),
                entry.slot_id,
                source.bytes_read(),
                parse_ms,
                total_ms,
                opened.document.unit_count,
                opened.document.paragraph_count,
                opened.truncated,
            );
            crate::memtrace!(
                "reader_open",
                "component" = "storage",
                "at_ms" = storage_now_ms(),
                "action" = "parsed",
                "sync_id" = trace.sync_id,
                "req_id" = trace.req_id,
                "content_id" = content_id.as_str(),
                "slot_id" = entry.slot_id,
                "bytes_read" = source.bytes_read(),
                "expected_bytes" = meta.size_bytes,
                "parse_ms" = parse_ms,
                "total_ms" = total_ms,
                "unit_count" = opened.document.unit_count,
                "paragraph_count" = opened.document.paragraph_count,
                "truncated" = bool_flag(opened.truncated),
            );
            opened
        };

        if let Some(index) = self.cache_index.find_index_by_content_id(&content_id) {
            self.cache_index.entries[index].last_touch_seq = self.cache_index.bump_touch_seq();
            self.write_cache_index()?;
        }

        Ok(opened)
    }

    fn open_cached_reader_package(
        &mut self,
        trace: TraceContext,
        content_id: InlineText<CONTENT_ID_MAX_BYTES>,
    ) -> Result<OpenedReaderPackage, StorageError> {
        let entry = self
            .cache_index
            .find_by_content_id(&content_id)
            .ok_or(StorageError::Unavailable)?;
        let meta = self.read_package_meta(entry.slot_id)?;
        if meta.remote_revision != entry.remote_revision {
            return Err(StorageError::CorruptData);
        }

        let opened = {
            let volume = self
                .volume_mgr
                .open_volume(VolumeIdx(0))
                .map_err(map_sd_error)?;
            let root = volume.open_root_dir().map_err(map_sd_error)?;
            let motif = root.open_dir(ROOT_DIR_NAME).map_err(map_sd_error)?;
            let v1 = motif.open_dir(VERSION_DIR_NAME).map_err(map_sd_error)?;
            let pkg_dir = v1.open_dir(PACKAGE_DIR_NAME).map_err(map_sd_error)?;
            let mut file = pkg_dir
                .open_file_in_dir(
                    package_payload_file_name(entry.slot_id).as_str(),
                    Mode::ReadOnly,
                )
                .map_err(map_sd_error)?;
            if file.length() != meta.size_bytes {
                return Err(StorageError::CorruptData);
            }

            let header = read_reader_package_header(&mut file)?;
            let title = read_reader_package_title(&mut file, header)?;
            let paragraphs = read_reader_package_paragraphs(&mut file, header)?;
            let window = read_reader_package_window(&mut file, header, 0)?;
            info!(
                "content storage package open content_id={} slot={} size_bytes={} total_units={} paragraphs={} initial_window_start={} initial_window_units={}",
                content_id.as_str(),
                entry.slot_id,
                meta.size_bytes,
                header.unit_count,
                header.paragraph_count,
                window.start_unit_index,
                window.unit_count,
            );
            crate::memtrace!(
                "reader_package",
                "component" = "storage",
                "at_ms" = storage_now_ms(),
                "action" = "open",
                "sync_id" = trace.sync_id,
                "req_id" = trace.req_id,
                "content_id" = content_id.as_str(),
                "slot_id" = entry.slot_id,
                "size_bytes" = meta.size_bytes,
                "title_bytes" = header.title_len,
                "total_units" = header.unit_count,
                "paragraph_count" = header.paragraph_count,
                "initial_window_start" = window.start_unit_index,
                "initial_window_units" = window.unit_count,
            );
            OpenedReaderPackage {
                title,
                total_units: header.unit_count,
                paragraphs,
                window,
            }
        };

        if let Some(index) = self.cache_index.find_index_by_content_id(&content_id) {
            self.cache_index.entries[index].last_touch_seq = self.cache_index.bump_touch_seq();
            self.write_cache_index()?;
        }

        Ok(opened)
    }

    fn load_reader_window(
        &mut self,
        trace: TraceContext,
        content_id: InlineText<CONTENT_ID_MAX_BYTES>,
        window_start_unit_index: u32,
    ) -> Result<Box<ReaderWindow>, StorageError> {
        let entry = self
            .cache_index
            .find_by_content_id(&content_id)
            .ok_or(StorageError::Unavailable)?;
        let meta = self.read_package_meta(entry.slot_id)?;
        if meta.remote_revision != entry.remote_revision {
            return Err(StorageError::CorruptData);
        }

        let volume = self
            .volume_mgr
            .open_volume(VolumeIdx(0))
            .map_err(map_sd_error)?;
        let root = volume.open_root_dir().map_err(map_sd_error)?;
        let motif = root.open_dir(ROOT_DIR_NAME).map_err(map_sd_error)?;
        let v1 = motif.open_dir(VERSION_DIR_NAME).map_err(map_sd_error)?;
        let pkg_dir = v1.open_dir(PACKAGE_DIR_NAME).map_err(map_sd_error)?;
        let mut file = pkg_dir
            .open_file_in_dir(
                package_payload_file_name(entry.slot_id).as_str(),
                Mode::ReadOnly,
            )
            .map_err(map_sd_error)?;
        if file.length() != meta.size_bytes {
            return Err(StorageError::CorruptData);
        }

        let header = read_reader_package_header(&mut file)?;
        let window = read_reader_package_window(&mut file, header, window_start_unit_index)?;
        info!(
            "content storage window load content_id={} slot={} requested_start={} loaded_start={} unit_count={} total_units={} total_paragraphs={}",
            content_id.as_str(),
            entry.slot_id,
            window_start_unit_index,
            window.start_unit_index,
            window.unit_count,
            header.unit_count,
            header.paragraph_count,
        );
        crate::memtrace!(
            "reader_window",
            "component" = "storage",
            "at_ms" = storage_now_ms(),
            "sync_id" = trace.sync_id,
            "req_id" = trace.req_id,
            "content_id" = content_id.as_str(),
            "slot_id" = entry.slot_id,
            "requested_start" = window_start_unit_index,
            "loaded_start" = window.start_unit_index,
            "unit_count" = window.unit_count,
            "total_units" = header.unit_count,
            "paragraph_count" = header.paragraph_count,
        );
        Ok(window)
    }

    fn reconcile_all_snapshots(&mut self) {
        let mut index = 0;
        while index < CollectionKind::ALL.len() {
            self.reconcile_snapshot(CollectionKind::ALL[index]);
            index += 1;
        }
    }

    fn reconcile_snapshot(&mut self, kind: CollectionKind) {
        let Some(snapshot) = self.snapshots[collection_index(kind)].as_mut() else {
            return;
        };
        let len = snapshot.len();
        let mut index = 0;
        while index < len {
            let item = &mut snapshot.items[index];
            let local = self.cache_index.find_by_content_id(&item.content_id);
            item.package_state = match local {
                Some(entry) if entry.remote_revision == item.remote_revision => {
                    PackageState::Cached
                }
                Some(_) => PackageState::Stale,
                None => match item.remote_status {
                    RemoteContentStatus::Ready => PackageState::Missing,
                    RemoteContentStatus::Pending | RemoteContentStatus::Unknown => {
                        PackageState::PendingRemote
                    }
                    RemoteContentStatus::Failed => PackageState::Failed,
                },
            };
            index += 1;
        }
    }

    fn refresh_collection_flags(&mut self) {
        let mut entry_index = 0;
        while entry_index < self.cache_index.len() {
            let content_id = self.cache_index.entries[entry_index].content_id;
            let mut flags = 0u8;

            let mut collection_index_value = 0;
            while collection_index_value < CollectionKind::ALL.len() {
                let kind = CollectionKind::ALL[collection_index_value];
                let Some(snapshot) = self.snapshots[collection_index(kind)].as_ref() else {
                    collection_index_value += 1;
                    continue;
                };
                let mut item_index = 0;
                while item_index < snapshot.len() {
                    if snapshot.items[item_index].content_id == content_id {
                        flags |= collection_flag(kind);
                    }
                    item_index += 1;
                }
                collection_index_value += 1;
            }

            self.cache_index.entries[entry_index].collection_flags = flags;
            entry_index += 1;
        }
    }

    fn evict_if_needed(&mut self, trace: TraceContext) -> Result<(), StorageError> {
        while self.cache_index.len() > CACHE_ENTRY_CAPACITY
            || self.cache_index.total_bytes() > CACHE_SIZE_BUDGET_BYTES
        {
            let Some(candidate) = self.select_eviction_candidate() else {
                break;
            };
            let before = self.storage_space_metrics()?;
            self.delete_package_slot(candidate.slot_id)?;
            let _ = self.cache_index.remove_slot(candidate.slot_id);
            self.reconcile_all_snapshots();
            let after = self.storage_space_metrics()?;
            crate::memtrace!(
                "storage_evict",
                "component" = "storage",
                "at_ms" = storage_now_ms(),
                "sync_id" = trace.sync_id,
                "req_id" = trace.req_id,
                "content_id" = candidate.content_id.as_str(),
                "slot_id" = candidate.slot_id,
                "deleted_bytes" = candidate.size_bytes,
                "cache_entries_before" = before.cache_entry_count,
                "cache_entries_after" = after.cache_entry_count,
                "cache_budget_remaining_before" = before.cache_budget_remaining,
                "cache_budget_remaining_after" = after.cache_budget_remaining,
                "sd_free_bytes_before" = before.sd_free_bytes,
                "sd_free_bytes_after" = after.sd_free_bytes,
                "sd_free_known" = bool_flag(after.sd_free_known),
                "motif_total_bytes_before" = before.motif_total_bytes,
                "motif_total_bytes_after" = after.motif_total_bytes,
                "package_bytes_before" = before.package_bytes,
                "package_bytes_after" = after.package_bytes,
            );
        }

        self.write_cache_index()?;
        self.write_manifest_snapshot(CollectionKind::Saved)?;
        self.write_manifest_snapshot(CollectionKind::Inbox)?;
        self.write_manifest_snapshot(CollectionKind::Recommendations)?;
        Ok(())
    }

    fn select_eviction_candidate(&self) -> Option<CacheEntry> {
        self.select_eviction_candidate_excluding(None)
    }

    fn select_eviction_candidate_excluding(&self, excluded_slot: Option<u8>) -> Option<CacheEntry> {
        let mut best: Option<CacheEntry> = None;
        let mut index = 0;
        while index < self.cache_index.len() {
            let entry = self.cache_index.entries[index];
            if Some(entry.slot_id) == excluded_slot {
                index += 1;
                continue;
            }
            match best {
                None => best = Some(entry),
                Some(current) => {
                    if compare_eviction_priority(entry, current) == Ordering::Less {
                        best = Some(entry);
                    }
                }
            }
            index += 1;
        }
        best
    }

    fn write_manifest_snapshot(&mut self, kind: CollectionKind) -> Result<(), StorageError> {
        let snapshot = self.snapshot(kind);
        let mut bytes = Box::new([0u8; MAX_MANIFEST_SNAPSHOT_LEN]);
        let encoded_len = encode_manifest_snapshot(kind, &snapshot, &mut bytes[..])?;
        self.write_named_file_in_manif_dir(manifest_file_name(kind), &bytes[..encoded_len])
    }

    fn read_manifest_snapshot(
        &mut self,
        kind: CollectionKind,
    ) -> Result<Option<CollectionManifestState>, StorageError> {
        let mut bytes = Box::new([0u8; MAX_MANIFEST_SNAPSHOT_LEN]);
        let Some(read_len) =
            self.read_named_file_in_manif_dir(manifest_file_name(kind), &mut bytes[..])?
        else {
            return Ok(None);
        };

        decode_manifest_snapshot(kind, &bytes[..read_len]).map(Some)
    }

    fn write_cache_index(&mut self) -> Result<(), StorageError> {
        let mut bytes = Box::new([0u8; MAX_CACHE_INDEX_LEN]);
        let encoded_len = encode_cache_index(&self.cache_index, &mut bytes[..])?;
        self.write_named_file_in_cache_dir(CACHE_INDEX_FILE_NAME, &bytes[..encoded_len])
    }

    fn read_cache_index(&mut self) -> Result<Option<CacheIndex>, StorageError> {
        let mut bytes = Box::new([0u8; MAX_CACHE_INDEX_LEN]);
        let Some(read_len) =
            self.read_named_file_in_cache_dir(CACHE_INDEX_FILE_NAME, &mut bytes[..])?
        else {
            return Ok(None);
        };

        decode_cache_index(&bytes[..read_len]).map(Some)
    }

    fn write_package_meta(
        &mut self,
        slot_id: u8,
        remote_revision: u64,
        size_bytes: u32,
        crc32: u32,
    ) -> Result<(), StorageError> {
        let mut bytes = [0u8; MAX_PACKAGE_META_LEN];
        let encoded_len =
            encode_package_meta(slot_id, remote_revision, size_bytes, crc32, &mut bytes)?;
        self.write_named_file_in_pkg_dir(&package_meta_file_name(slot_id), &bytes[..encoded_len])
    }

    fn read_package_meta(&mut self, slot_id: u8) -> Result<PackageMeta, StorageError> {
        let mut bytes = [0u8; MAX_PACKAGE_META_LEN];
        let read_len = self
            .read_named_file_in_pkg_dir(package_meta_file_name(slot_id).as_str(), &mut bytes)?
            .ok_or(StorageError::Unavailable)?;
        decode_package_meta(&bytes[..read_len])
    }

    fn cleanup_active_stage_file(&mut self) -> Result<(), StorageError> {
        if let Some(stage) = self.pending_stage.take() {
            self.cleanup_pending_stage_target(&stage)?;
        }
        self.pending_stage_error = None;
        self.delete_stage_file()
    }

    fn record_pending_stage_error(&mut self, trace: TraceContext, err: StorageError) {
        let Some(stage) = self.pending_stage.as_ref() else {
            self.pending_stage_error = Some(err);
            return;
        };

        info!(
            "content storage stage async error content_id={} slot={} target={} err={:?}",
            stage.content_id.as_str(),
            stage.slot_id,
            stage.target_kind.label(),
            err,
        );
        crate::memtrace!(
            "storage_stage",
            "component" = "storage",
            "at_ms" = storage_now_ms(),
            "action" = "async_error",
            "sync_id" = trace.sync_id,
            "req_id" = trace.req_id,
            "content_id" = stage.content_id.as_str(),
            "slot_id" = stage.slot_id,
            "target" = stage.target_kind.label(),
            "error" = storage_error_label(err),
            "bytes_written" = stage.bytes_written,
            "elapsed_ms" = storage_elapsed_since_ms(stage.started_at_ms),
        );
        self.pending_stage_error = Some(err);
    }

    fn validate_staged_reader_package(
        &mut self,
        expected_size: u32,
    ) -> Result<ReaderPackageHeader, StorageError> {
        self.validate_reader_package_file(STAGING_DIR_NAME, ACTIVE_STAGE_FILE_NAME, expected_size)
    }

    fn validate_package_slot_reader_package(
        &mut self,
        slot_id: u8,
        expected_size: u32,
    ) -> Result<ReaderPackageHeader, StorageError> {
        let payload_name = package_payload_file_name(slot_id);
        self.validate_reader_package_file(PACKAGE_DIR_NAME, payload_name.as_str(), expected_size)
    }

    fn validate_reader_package_file(
        &mut self,
        subdir_name: &str,
        file_name: &str,
        expected_size: u32,
    ) -> Result<ReaderPackageHeader, StorageError> {
        let volume = self
            .volume_mgr
            .open_volume(VolumeIdx(0))
            .map_err(map_sd_error)?;
        let root = volume.open_root_dir().map_err(map_sd_error)?;
        let motif = root.open_dir(ROOT_DIR_NAME).map_err(map_sd_error)?;
        let v1 = motif.open_dir(VERSION_DIR_NAME).map_err(map_sd_error)?;
        let dir = v1.open_dir(subdir_name).map_err(map_sd_error)?;
        let mut file = dir
            .open_file_in_dir(file_name, Mode::ReadOnly)
            .map_err(map_sd_error)?;
        if file.length() != expected_size {
            return Err(StorageError::CorruptData);
        }
        let header = read_reader_package_header(&mut file)?;
        info!(
            "content storage stage validate size_bytes={} title_len={} total_units={} paragraphs={} paragraph_table_offset={} unit_table_offset={}",
            expected_size,
            header.title_len,
            header.unit_count,
            header.paragraph_count,
            header.paragraph_table_offset,
            header.unit_table_offset,
        );
        Ok(header)
    }

    fn delete_stage_file(&mut self) -> Result<(), StorageError> {
        let volume = self
            .volume_mgr
            .open_volume(VolumeIdx(0))
            .map_err(map_sd_error)?;
        let root = volume.open_root_dir().map_err(map_sd_error)?;
        let motif = root.open_dir(ROOT_DIR_NAME).map_err(map_sd_error)?;
        let v1 = motif.open_dir(VERSION_DIR_NAME).map_err(map_sd_error)?;
        let stage_dir = v1.open_dir(STAGING_DIR_NAME).map_err(map_sd_error)?;
        match stage_dir.delete_file_in_dir(ACTIVE_STAGE_FILE_NAME) {
            Ok(()) => Ok(()),
            Err(SdError::NotFound) => Ok(()),
            Err(err) => Err(map_sd_error(err)),
        }
    }

    fn open_pending_stage_writer(
        &mut self,
        slot_id: u8,
        direct_to_package: bool,
    ) -> Result<(PendingStageTargetKind, RawVolume, RawFile), StorageError> {
        let volume = self
            .volume_mgr
            .open_volume(VolumeIdx(0))
            .map_err(map_sd_error)?;
        let target_kind = if direct_to_package {
            PendingStageTargetKind::PackageSlot
        } else {
            PendingStageTargetKind::StagingFile
        };
        let stage_file = {
            let root = volume.open_root_dir().map_err(map_sd_error)?;
            let motif = root.open_dir(ROOT_DIR_NAME).map_err(map_sd_error)?;
            let v1 = motif.open_dir(VERSION_DIR_NAME).map_err(map_sd_error)?;
            match target_kind {
                PendingStageTargetKind::StagingFile => {
                    let stage_dir = v1.open_dir(STAGING_DIR_NAME).map_err(map_sd_error)?;
                    stage_dir
                        .open_file_in_dir(ACTIVE_STAGE_FILE_NAME, Mode::ReadWriteCreateOrTruncate)
                        .map_err(map_sd_error)?
                        .to_raw_file()
                }
                PendingStageTargetKind::PackageSlot => {
                    let pkg_dir = v1.open_dir(PACKAGE_DIR_NAME).map_err(map_sd_error)?;
                    let payload_name = package_payload_file_name(slot_id);
                    pkg_dir
                        .open_file_in_dir(payload_name.as_str(), Mode::ReadWriteCreateOrTruncate)
                        .map_err(map_sd_error)?
                        .to_raw_file()
                }
            }
        };
        Ok((target_kind, volume.to_raw_volume(), stage_file))
    }

    fn append_stage_file(&self, stage_file: RawFile, bytes: &[u8]) -> Result<(), StorageError> {
        self.volume_mgr
            .write(stage_file, bytes)
            .map_err(map_sd_error)
    }

    fn flush_stage_file(&self, stage_file: RawFile) -> Result<(), StorageError> {
        self.volume_mgr.flush_file(stage_file).map_err(map_sd_error)
    }

    fn close_stage_writer(&self, stage: &PendingStage) -> Result<(), StorageError> {
        let file_result = self
            .volume_mgr
            .close_file(stage.stage_file)
            .map_err(map_sd_error);
        let volume_result = self
            .volume_mgr
            .close_volume(stage.stage_volume)
            .map_err(map_sd_error);
        file_result.and(volume_result)
    }

    fn cleanup_pending_stage_target(&mut self, stage: &PendingStage) -> Result<(), StorageError> {
        self.close_stage_writer(stage)?;
        match stage.target_kind {
            PendingStageTargetKind::StagingFile => self.delete_stage_file(),
            PendingStageTargetKind::PackageSlot => self.delete_package_slot(stage.slot_id),
        }
    }

    fn copy_stage_to_package_slot(&mut self, slot_id: u8) -> Result<u32, StorageError> {
        self.write_named_file_in_pkg_dir(&package_payload_file_name(slot_id), &[])?;

        let volume = self
            .volume_mgr
            .open_volume(VolumeIdx(0))
            .map_err(map_sd_error)?;
        let root = volume.open_root_dir().map_err(map_sd_error)?;
        let motif = root.open_dir(ROOT_DIR_NAME).map_err(map_sd_error)?;
        let v1 = motif.open_dir(VERSION_DIR_NAME).map_err(map_sd_error)?;
        let stage_dir = v1.open_dir(STAGING_DIR_NAME).map_err(map_sd_error)?;
        let pkg_dir = v1.open_dir(PACKAGE_DIR_NAME).map_err(map_sd_error)?;
        let stage_file = stage_dir
            .open_file_in_dir(ACTIVE_STAGE_FILE_NAME, Mode::ReadOnly)
            .map_err(map_sd_error)?;
        let payload_file = pkg_dir
            .open_file_in_dir(
                package_payload_file_name(slot_id).as_str(),
                Mode::ReadWriteCreateOrTruncate,
            )
            .map_err(map_sd_error)?;

        let mut buffer = [0u8; PACKAGE_COPY_BUFFER_LEN];
        let mut copied_bytes = 0u32;
        loop {
            let read = stage_file.read(&mut buffer).map_err(map_sd_error)?;
            if read == 0 {
                break;
            }
            payload_file.write(&buffer[..read]).map_err(map_sd_error)?;
            copied_bytes = copied_bytes.saturating_add(read as u32);
        }
        payload_file.flush().map_err(map_sd_error)?;
        Ok(copied_bytes)
    }

    fn delete_package_slot(&mut self, slot_id: u8) -> Result<(), StorageError> {
        let volume = self
            .volume_mgr
            .open_volume(VolumeIdx(0))
            .map_err(map_sd_error)?;
        let root = volume.open_root_dir().map_err(map_sd_error)?;
        let motif = root.open_dir(ROOT_DIR_NAME).map_err(map_sd_error)?;
        let v1 = motif.open_dir(VERSION_DIR_NAME).map_err(map_sd_error)?;
        let pkg_dir = v1.open_dir(PACKAGE_DIR_NAME).map_err(map_sd_error)?;

        for name in [
            package_payload_file_name(slot_id),
            package_meta_file_name(slot_id),
        ] {
            match pkg_dir.delete_file_in_dir(name.as_str()) {
                Ok(()) | Err(SdError::NotFound) => {}
                Err(err) => return Err(map_sd_error(err)),
            }
        }

        Ok(())
    }

    fn storage_space_metrics(&self) -> Result<StorageSpaceMetrics, StorageError> {
        let volume = self
            .volume_mgr
            .open_volume(VolumeIdx(0))
            .map_err(map_sd_error)?;
        let raw_volume = volume.to_raw_volume();
        let metrics = self.storage_space_metrics_for_open_volume(raw_volume)?;
        self.volume_mgr
            .close_volume(raw_volume)
            .map_err(map_sd_error)?;
        Ok(metrics)
    }

    fn storage_space_metrics_for_open_volume(
        &self,
        volume: RawVolume,
    ) -> Result<StorageSpaceMetrics, StorageError> {
        let (manifest, cache, stage, package) = {
            let root = self
                .volume_mgr
                .open_root_dir(volume)
                .map_err(map_sd_error)?
                .to_directory(&self.volume_mgr);
            let motif = root.open_dir(ROOT_DIR_NAME).map_err(map_sd_error)?;
            let v1 = motif.open_dir(VERSION_DIR_NAME).map_err(map_sd_error)?;
            let manifest = read_dir_usage(&v1.open_dir(MANIFEST_DIR_NAME).map_err(map_sd_error)?)?;
            let cache = read_dir_usage(&v1.open_dir(CACHE_DIR_NAME).map_err(map_sd_error)?)?;
            let stage = read_dir_usage(&v1.open_dir(STAGING_DIR_NAME).map_err(map_sd_error)?)?;
            let package = read_dir_usage(&v1.open_dir(PACKAGE_DIR_NAME).map_err(map_sd_error)?)?;
            (manifest, cache, stage, package)
        };
        let (sd_free_bytes, sd_free_known, sd_cluster_size_bytes) =
            self.read_sd_free_bytes().unwrap_or((0, false, 0));

        Ok(StorageSpaceMetrics {
            sd_total_bytes: self.total_bytes,
            sd_free_bytes,
            sd_free_known,
            sd_cluster_size_bytes,
            motif_total_bytes: manifest
                .bytes
                .saturating_add(cache.bytes)
                .saturating_add(stage.bytes)
                .saturating_add(package.bytes),
            manifest_bytes: manifest.bytes,
            cache_bytes: cache.bytes,
            stage_bytes: stage.bytes,
            package_bytes: package.bytes,
            package_files: package.files,
            cache_entry_count: self.cache_index.len(),
            cache_budget_remaining: CACHE_SIZE_BUDGET_BYTES
                .saturating_sub(self.cache_index.total_bytes()),
        })
    }

    fn cleanup_orphan_package_slots(&mut self) -> Result<(), StorageError> {
        let mut slot_id = 1u8;
        while (slot_id as usize) <= CACHE_ENTRY_CAPACITY {
            if !self.cache_index.contains_slot(slot_id) {
                self.delete_package_slot(slot_id)?;
            }
            slot_id = slot_id.saturating_add(1);
        }
        Ok(())
    }

    fn read_sd_free_bytes(&self) -> Result<(u64, bool, u32), StorageError> {
        let mut result = None;
        let _ = self.volume_mgr.device(|device| {
            result = Some(read_fat_volume_free_bytes(device));
            FixedTimeSource
        });
        result.unwrap_or(Err(StorageError::Unavailable))
    }

    fn write_named_file_in_manif_dir(
        &mut self,
        name: &str,
        bytes: &[u8],
    ) -> Result<(), StorageError> {
        self.write_named_file_in_subdir(MANIFEST_DIR_NAME, name, bytes)
    }

    fn read_named_file_in_manif_dir(
        &mut self,
        name: &str,
        out: &mut [u8],
    ) -> Result<Option<usize>, StorageError> {
        self.read_named_file_in_subdir(MANIFEST_DIR_NAME, name, out)
    }

    fn write_named_file_in_cache_dir(
        &mut self,
        name: &str,
        bytes: &[u8],
    ) -> Result<(), StorageError> {
        self.write_named_file_in_subdir(CACHE_DIR_NAME, name, bytes)
    }

    fn read_named_file_in_cache_dir(
        &mut self,
        name: &str,
        out: &mut [u8],
    ) -> Result<Option<usize>, StorageError> {
        self.read_named_file_in_subdir(CACHE_DIR_NAME, name, out)
    }

    fn write_named_file_in_stage_dir(
        &mut self,
        name: &str,
        bytes: &[u8],
    ) -> Result<(), StorageError> {
        self.write_named_file_in_subdir(STAGING_DIR_NAME, name, bytes)
    }

    fn write_named_file_in_pkg_dir(
        &mut self,
        name: &str,
        bytes: &[u8],
    ) -> Result<(), StorageError> {
        self.write_named_file_in_subdir(PACKAGE_DIR_NAME, name, bytes)
    }

    fn read_named_file_in_pkg_dir(
        &mut self,
        name: &str,
        out: &mut [u8],
    ) -> Result<Option<usize>, StorageError> {
        self.read_named_file_in_subdir(PACKAGE_DIR_NAME, name, out)
    }

    fn write_named_file_in_subdir(
        &mut self,
        subdir_name: &str,
        file_name: &str,
        bytes: &[u8],
    ) -> Result<(), StorageError> {
        let volume = self
            .volume_mgr
            .open_volume(VolumeIdx(0))
            .map_err(map_sd_error)?;
        let root = volume.open_root_dir().map_err(map_sd_error)?;
        let motif = root.open_dir(ROOT_DIR_NAME).map_err(map_sd_error)?;
        let v1 = motif.open_dir(VERSION_DIR_NAME).map_err(map_sd_error)?;
        let dir = v1.open_dir(subdir_name).map_err(map_sd_error)?;
        match dir.delete_file_in_dir(file_name) {
            Ok(()) | Err(SdError::NotFound) => {}
            Err(err) => return Err(map_sd_error(err)),
        }
        let file = dir
            .open_file_in_dir(file_name, Mode::ReadWriteCreateOrTruncate)
            .map_err(map_sd_error)?;
        if !bytes.is_empty() {
            file.write(bytes).map_err(map_sd_error)?;
        }
        file.flush().map_err(map_sd_error)?;
        Ok(())
    }

    fn read_named_file_in_subdir(
        &mut self,
        subdir_name: &str,
        file_name: &str,
        out: &mut [u8],
    ) -> Result<Option<usize>, StorageError> {
        let volume = self
            .volume_mgr
            .open_volume(VolumeIdx(0))
            .map_err(map_sd_error)?;
        let root = volume.open_root_dir().map_err(map_sd_error)?;
        let motif = root.open_dir(ROOT_DIR_NAME).map_err(map_sd_error)?;
        let v1 = motif.open_dir(VERSION_DIR_NAME).map_err(map_sd_error)?;
        let dir = v1.open_dir(subdir_name).map_err(map_sd_error)?;
        let file = match dir.open_file_in_dir(file_name, Mode::ReadOnly) {
            Ok(file) => file,
            Err(SdError::NotFound) => return Ok(None),
            Err(err) => return Err(map_sd_error(err)),
        };

        let mut total = 0usize;
        loop {
            if total == out.len() {
                return Err(StorageError::PayloadTooLarge);
            }

            let read = file.read(&mut out[total..]).map_err(map_sd_error)?;
            if read == 0 {
                break;
            }
            total += read;
        }

        Ok(Some(total))
    }
}

fn open_or_create_dir<'a, 'd>(
    parent: &'a SdDirectory<'a, 'd>,
    name: &str,
) -> Result<SdDirectory<'a, 'd>, StorageError> {
    match parent.open_dir(name) {
        Ok(dir) => Ok(dir),
        Err(SdError::NotFound) => {
            parent.make_dir_in_dir(name).map_err(map_sd_error)?;
            parent.open_dir(name).map_err(map_sd_error)
        }
        Err(err) => Err(map_sd_error(err)),
    }
}

fn open_or_replace_dir<'a, 'd>(
    parent: &'a SdDirectory<'a, 'd>,
    name: &str,
) -> Result<SdDirectory<'a, 'd>, StorageError> {
    match parent.open_dir(name) {
        Ok(dir) => Ok(dir),
        Err(SdError::NotFound) => {
            parent.make_dir_in_dir(name).map_err(map_sd_error)?;
            parent.open_dir(name).map_err(map_sd_error)
        }
        Err(SdError::OpenedFileAsDir) => {
            parent.delete_file_in_dir(name).map_err(map_sd_error)?;
            parent.make_dir_in_dir(name).map_err(map_sd_error)?;
            parent.open_dir(name).map_err(map_sd_error)
        }
        Err(err) => Err(map_sd_error(err)),
    }
}

fn clear_or_recreate_dir(parent: &SdDirectory<'_, '_>, name: &str) -> Result<(), StorageError> {
    let dir = open_or_replace_dir(parent, name)?;
    clear_directory_files(dir)
}

fn clear_directory_files(dir: SdDirectory<'_, '_>) -> Result<(), StorageError> {
    let mut file_names = Vec::<ShortFileName>::new();
    dir.iterate_dir(|entry| {
        if entry.attributes.is_directory() {
            return;
        }
        file_names.push(entry.name.clone());
    })
    .map_err(map_sd_error)?;

    for file_name in file_names {
        match dir.delete_file_in_dir(&file_name) {
            Ok(()) | Err(SdError::NotFound) => {}
            Err(err) => return Err(map_sd_error(err)),
        }
    }

    Ok(())
}
fn compare_eviction_priority(candidate: CacheEntry, current: CacheEntry) -> Ordering {
    eviction_sort_key(candidate)
        .cmp(&eviction_sort_key(current))
        .then(candidate.last_touch_seq.cmp(&current.last_touch_seq))
}

fn eviction_sort_key(entry: CacheEntry) -> (u8, u8) {
    let referenced = if entry.collection_flags == 0 {
        0u8
    } else {
        1u8
    };
    let priority = if entry.collection_flags & collection_flag(CollectionKind::Saved) != 0 {
        2
    } else if entry.collection_flags & collection_flag(CollectionKind::Inbox) != 0 {
        1
    } else {
        0
    };

    (referenced, priority)
}

fn collection_index(kind: CollectionKind) -> usize {
    match kind {
        CollectionKind::Saved => 0,
        CollectionKind::Inbox => 1,
        CollectionKind::Recommendations => 2,
    }
}

const fn storage_error_label(error: StorageError) -> &'static str {
    match error {
        StorageError::Unavailable => "unavailable",
        StorageError::PartitionMissing => "partition_missing",
        StorageError::InvalidPartition => "invalid_partition",
        StorageError::CorruptData => "corrupt_data",
        StorageError::PayloadTooLarge => "payload_too_large",
        StorageError::PartitionFull => "partition_full",
        StorageError::UnsupportedLayout => "unsupported_layout",
        StorageError::TooManyKeys => "too_many_keys",
        StorageError::FlashFailure => "flash_failure",
        StorageError::CodecFailure => "codec_failure",
    }
}

const fn collection_flag(kind: CollectionKind) -> u8 {
    match kind {
        CollectionKind::Saved => 0b001,
        CollectionKind::Inbox => 0b010,
        CollectionKind::Recommendations => 0b100,
    }
}

fn manifest_file_name(kind: CollectionKind) -> &'static str {
    match kind {
        CollectionKind::Saved => SAVED_MANIFEST_FILE_NAME,
        CollectionKind::Inbox => INBOX_MANIFEST_FILE_NAME,
        CollectionKind::Recommendations => RECOMMENDATION_MANIFEST_FILE_NAME,
    }
}

fn package_payload_file_name(slot_id: u8) -> heapless::String<12> {
    let mut name = heapless::String::<12>::new();
    let _ = core::fmt::write(&mut name, format_args!("P{:07}.PKG", slot_id));
    name
}

fn package_meta_file_name(slot_id: u8) -> heapless::String<12> {
    let mut name = heapless::String::<12>::new();
    let _ = core::fmt::write(&mut name, format_args!("P{:07}.MTA", slot_id));
    name
}

fn map_sd_error<E: core::fmt::Debug>(error: SdError<E>) -> StorageError {
    match error {
        SdError::NotFound => StorageError::Unavailable,
        SdError::OpenedDirAsFile | SdError::OpenedFileAsDir | SdError::DeleteDirAsFile => {
            StorageError::CorruptData
        }
        SdError::FormatError(_) | SdError::BadCluster | SdError::InvalidOffset => {
            StorageError::CorruptData
        }
        SdError::DiskFull | SdError::NotEnoughSpace => StorageError::PartitionFull,
        SdError::FilenameError(_) | SdError::Unsupported | SdError::BadBlockSize(_) => {
            StorageError::UnsupportedLayout
        }
        SdError::ReadOnly => StorageError::Unavailable,
        SdError::DeviceError(_) => StorageError::FlashFailure,
        _ => StorageError::Unavailable,
    }
}

fn encode_manifest_snapshot(
    kind: CollectionKind,
    snapshot: &CollectionManifestState,
    out: &mut [u8],
) -> Result<usize, StorageError> {
    let needed =
        16 + RECOMMENDATION_SERVE_ID_MAX_BYTES + snapshot.len() * manifest_item_encoded_len();
    if out.len() < needed {
        return Err(StorageError::PayloadTooLarge);
    }

    out.fill(0);
    write_u32(out, 0, MANIFEST_MAGIC);
    write_u16(out, 4, FORMAT_VERSION);
    out[6] = collection_index(kind) as u8;
    out[7] = snapshot.len() as u8;
    out[8] = snapshot.serve_id.len() as u8;
    write_inline_text(
        &mut out[16..16 + RECOMMENDATION_SERVE_ID_MAX_BYTES],
        &snapshot.serve_id,
    );

    let mut offset = 16 + RECOMMENDATION_SERVE_ID_MAX_BYTES;
    let mut index = 0;
    while index < snapshot.len() {
        offset += encode_manifest_item(&snapshot.items[index], &mut out[offset..])?;
        index += 1;
    }

    Ok(offset)
}

fn decode_manifest_snapshot(
    kind: CollectionKind,
    bytes: &[u8],
) -> Result<CollectionManifestState, StorageError> {
    if bytes.len() < 16 + RECOMMENDATION_SERVE_ID_MAX_BYTES {
        return Err(StorageError::CorruptData);
    }
    if read_u32(bytes, 0) != MANIFEST_MAGIC || read_u16(bytes, 4) != FORMAT_VERSION {
        return Err(StorageError::CorruptData);
    }
    if bytes[6] != collection_index(kind) as u8 {
        return Err(StorageError::CorruptData);
    }

    let len = bytes[7] as usize;
    if len > MANIFEST_ITEM_CAPACITY {
        return Err(StorageError::CorruptData);
    }

    let mut snapshot = CollectionManifestState::empty();
    let serve_id_len = bytes[8] as usize;
    read_inline_text(
        &mut snapshot.serve_id,
        serve_id_len,
        &bytes[16..16 + RECOMMENDATION_SERVE_ID_MAX_BYTES],
    );

    let mut offset = 16 + RECOMMENDATION_SERVE_ID_MAX_BYTES;
    let mut index = 0;
    while index < len {
        let (item, consumed) = decode_manifest_item(&bytes[offset..])?;
        let _ = snapshot.try_push(item);
        offset += consumed;
        index += 1;
    }

    Ok(snapshot)
}

const fn manifest_item_encoded_len() -> usize {
    1 + REMOTE_ITEM_ID_MAX_BYTES
        + 1
        + CONTENT_ID_MAX_BYTES
        + 1
        + 1
        + 1
        + CONTENT_META_MAX_BYTES
        + 1
        + CONTENT_TITLE_MAX_BYTES
        + 8
        + 1
        + 1
}

fn encode_manifest_item(
    item: &CollectionManifestItem,
    out: &mut [u8],
) -> Result<usize, StorageError> {
    let needed = manifest_item_encoded_len();
    if out.len() < needed {
        return Err(StorageError::PayloadTooLarge);
    }

    out.fill(0);
    out[0] = item.remote_item_id.len() as u8;
    write_inline_text(
        &mut out[1..1 + REMOTE_ITEM_ID_MAX_BYTES],
        &item.remote_item_id,
    );
    let content_id_offset = 1 + REMOTE_ITEM_ID_MAX_BYTES;
    out[content_id_offset] = item.content_id.len() as u8;
    write_inline_text(
        &mut out[content_id_offset + 1..content_id_offset + 1 + CONTENT_ID_MAX_BYTES],
        &item.content_id,
    );
    let detail_offset = content_id_offset + 1 + CONTENT_ID_MAX_BYTES;
    out[detail_offset] = detail_locator_to_byte(item.detail_locator);
    out[detail_offset + 1] = source_kind_to_byte(item.source);
    out[detail_offset + 2] = item.meta.len() as u8;
    let meta_offset = detail_offset + 3;
    write_inline_text(
        &mut out[meta_offset..meta_offset + CONTENT_META_MAX_BYTES],
        &item.meta,
    );
    let title_offset = meta_offset + CONTENT_META_MAX_BYTES;
    out[title_offset] = item.title.len() as u8;
    write_inline_text(
        &mut out[title_offset + 1..title_offset + 1 + CONTENT_TITLE_MAX_BYTES],
        &item.title,
    );
    let revision_offset = title_offset + 1 + CONTENT_TITLE_MAX_BYTES;
    write_u64(out, revision_offset, item.remote_revision);
    out[revision_offset + 8] = remote_status_to_byte(item.remote_status);
    out[revision_offset + 9] = package_state_to_byte(item.package_state);
    Ok(needed)
}

fn decode_manifest_item(bytes: &[u8]) -> Result<(CollectionManifestItem, usize), StorageError> {
    let needed = manifest_item_encoded_len();
    if bytes.len() < needed {
        return Err(StorageError::CorruptData);
    }

    let remote_id_offset = 1 + REMOTE_ITEM_ID_MAX_BYTES;
    let detail_offset = remote_id_offset + 1 + CONTENT_ID_MAX_BYTES;
    let meta_offset = detail_offset + 3;
    let title_offset = meta_offset + CONTENT_META_MAX_BYTES;
    let revision_offset = title_offset + 1 + CONTENT_TITLE_MAX_BYTES;

    let mut item = CollectionManifestItem::empty();
    read_inline_text(
        &mut item.remote_item_id,
        bytes[0] as usize,
        &bytes[1..1 + REMOTE_ITEM_ID_MAX_BYTES],
    );
    read_inline_text(
        &mut item.content_id,
        bytes[remote_id_offset] as usize,
        &bytes[remote_id_offset + 1..remote_id_offset + 1 + CONTENT_ID_MAX_BYTES],
    );
    item.detail_locator = detail_locator_from_byte(bytes[detail_offset])?;
    item.source = source_kind_from_byte(bytes[detail_offset + 1])?;
    read_inline_text(
        &mut item.meta,
        bytes[detail_offset + 2] as usize,
        &bytes[meta_offset..meta_offset + CONTENT_META_MAX_BYTES],
    );
    read_inline_text(
        &mut item.title,
        bytes[title_offset] as usize,
        &bytes[title_offset + 1..title_offset + 1 + CONTENT_TITLE_MAX_BYTES],
    );
    item.remote_revision = read_u64(bytes, revision_offset);
    item.remote_status = remote_status_from_byte(bytes[revision_offset + 8])?;
    item.package_state = package_state_from_byte(bytes[revision_offset + 9])?;
    Ok((item, needed))
}

fn encode_cache_index(index: &CacheIndex, out: &mut [u8]) -> Result<usize, StorageError> {
    let needed = 16 + index.len() * cache_entry_encoded_len();
    if out.len() < needed {
        return Err(StorageError::PayloadTooLarge);
    }

    out.fill(0);
    write_u32(out, 0, CACHE_INDEX_MAGIC);
    write_u16(out, 4, FORMAT_VERSION);
    out[6] = index.len;
    write_u32(out, 8, index.next_touch_seq);

    let mut offset = 16;
    let mut entry_index = 0;
    while entry_index < index.len() {
        offset += encode_cache_entry(&index.entries[entry_index], &mut out[offset..])?;
        entry_index += 1;
    }

    Ok(offset)
}

fn decode_cache_index(bytes: &[u8]) -> Result<CacheIndex, StorageError> {
    if bytes.len() < 16 {
        return Err(StorageError::CorruptData);
    }
    if read_u32(bytes, 0) != CACHE_INDEX_MAGIC || read_u16(bytes, 4) != FORMAT_VERSION {
        return Err(StorageError::CorruptData);
    }

    let len = bytes[6] as usize;
    if len > CACHE_ENTRY_CAPACITY {
        return Err(StorageError::CorruptData);
    }

    let mut index = CacheIndex::empty();
    index.next_touch_seq = read_u32(bytes, 8).max(1);
    let mut offset = 16;
    let mut entry_index = 0;
    while entry_index < len {
        let (entry, consumed) = decode_cache_entry(&bytes[offset..])?;
        index.entries[entry_index] = entry;
        index.len = index.len.saturating_add(1);
        offset += consumed;
        entry_index += 1;
    }
    Ok(index)
}

const fn cache_entry_encoded_len() -> usize {
    1 + 1 + CONTENT_ID_MAX_BYTES + 8 + 4 + 4 + 4 + 1
}

fn encode_cache_entry(entry: &CacheEntry, out: &mut [u8]) -> Result<usize, StorageError> {
    let needed = cache_entry_encoded_len();
    if out.len() < needed {
        return Err(StorageError::PayloadTooLarge);
    }

    out.fill(0);
    out[0] = entry.slot_id;
    out[1] = entry.content_id.len() as u8;
    write_inline_text(&mut out[2..2 + CONTENT_ID_MAX_BYTES], &entry.content_id);
    let offset = 2 + CONTENT_ID_MAX_BYTES;
    write_u64(out, offset, entry.remote_revision);
    write_u32(out, offset + 8, entry.size_bytes);
    write_u32(out, offset + 12, entry.crc32);
    write_u32(out, offset + 16, entry.last_touch_seq);
    out[offset + 20] = entry.collection_flags;
    Ok(needed)
}

fn decode_cache_entry(bytes: &[u8]) -> Result<(CacheEntry, usize), StorageError> {
    let needed = cache_entry_encoded_len();
    if bytes.len() < needed {
        return Err(StorageError::CorruptData);
    }
    let offset = 2 + CONTENT_ID_MAX_BYTES;
    let mut entry = CacheEntry::empty();
    entry.slot_id = bytes[0];
    read_inline_text(
        &mut entry.content_id,
        bytes[1] as usize,
        &bytes[2..2 + CONTENT_ID_MAX_BYTES],
    );
    entry.remote_revision = read_u64(bytes, offset);
    entry.size_bytes = read_u32(bytes, offset + 8);
    entry.crc32 = read_u32(bytes, offset + 12);
    entry.last_touch_seq = read_u32(bytes, offset + 16);
    entry.collection_flags = bytes[offset + 20];
    Ok((entry, needed))
}

fn encode_package_meta(
    slot_id: u8,
    remote_revision: u64,
    size_bytes: u32,
    crc32: u32,
    out: &mut [u8],
) -> Result<usize, StorageError> {
    if out.len() < 24 {
        return Err(StorageError::PayloadTooLarge);
    }

    out.fill(0);
    write_u32(out, 0, PACKAGE_META_MAGIC);
    write_u16(out, 4, FORMAT_VERSION);
    out[6] = slot_id;
    write_u64(out, 8, remote_revision);
    write_u32(out, 16, size_bytes);
    write_u32(out, 20, crc32);
    Ok(24)
}

fn decode_package_meta(bytes: &[u8]) -> Result<PackageMeta, StorageError> {
    if bytes.len() < 24 {
        return Err(StorageError::CorruptData);
    }
    if read_u32(bytes, 0) != PACKAGE_META_MAGIC || read_u16(bytes, 4) != FORMAT_VERSION {
        return Err(StorageError::CorruptData);
    }

    Ok(PackageMeta {
        slot_id: bytes[6],
        remote_revision: read_u64(bytes, 8),
        size_bytes: read_u32(bytes, 16),
        crc32: read_u32(bytes, 20),
    })
}

fn write_inline_text<const N: usize>(out: &mut [u8], value: &InlineText<N>) {
    let bytes = value.as_str().as_bytes();
    out[..bytes.len()].copy_from_slice(bytes);
}

fn read_inline_text<const N: usize>(target: &mut InlineText<N>, len: usize, bytes: &[u8]) {
    target.clear();
    let copy_len = len.min(bytes.len());
    let slice = core::str::from_utf8(&bytes[..copy_len]).unwrap_or("");
    target.set_truncated(slice);
}

fn write_u16(buffer: &mut [u8], offset: usize, value: u16) {
    buffer[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn write_u32(buffer: &mut [u8], offset: usize, value: u32) {
    buffer[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_u64(buffer: &mut [u8], offset: usize, value: u64) {
    buffer[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn read_u16(buffer: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([buffer[offset], buffer[offset + 1]])
}

fn read_u32(buffer: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        buffer[offset],
        buffer[offset + 1],
        buffer[offset + 2],
        buffer[offset + 3],
    ])
}

fn read_u64(buffer: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        buffer[offset],
        buffer[offset + 1],
        buffer[offset + 2],
        buffer[offset + 3],
        buffer[offset + 4],
        buffer[offset + 5],
        buffer[offset + 6],
        buffer[offset + 7],
    ])
}

fn read_exact_file(file: &mut SdFile<'_, '_>, out: &mut [u8]) -> Result<(), StorageError> {
    let mut offset = 0usize;
    while offset < out.len() {
        let read = file.read(&mut out[offset..]).map_err(map_sd_error)?;
        if read == 0 {
            return Err(StorageError::CorruptData);
        }
        offset += read;
    }
    Ok(())
}

fn decode_reader_package_header(
    bytes: &[u8],
    file_len: u32,
) -> Result<ReaderPackageHeader, StorageError> {
    if bytes.len() < READER_PACKAGE_HEADER_LEN {
        return Err(StorageError::CorruptData);
    }
    if read_u32(bytes, 0) != READER_PACKAGE_MAGIC
        || read_u16(bytes, 4) != READER_PACKAGE_FORMAT_VERSION
    {
        return Err(StorageError::CorruptData);
    }

    let title_len = read_u16(bytes, 8);
    let paragraph_count = read_u16(bytes, 10);
    let unit_count = read_u32(bytes, 12);
    let paragraph_table_offset = read_u32(bytes, 16);
    let unit_table_offset = read_u32(bytes, 20);
    let expected_paragraph_offset = (READER_PACKAGE_HEADER_LEN as u32)
        .checked_add(title_len as u32)
        .ok_or(StorageError::CorruptData)?;
    let expected_unit_offset = expected_paragraph_offset
        .checked_add(
            (paragraph_count as u32)
                .checked_mul(READER_PACKAGE_PARAGRAPH_ENTRY_LEN as u32)
                .ok_or(StorageError::CorruptData)?,
        )
        .ok_or(StorageError::CorruptData)?;
    let expected_file_len = expected_unit_offset
        .checked_add(
            unit_count
                .checked_mul(READER_PACKAGE_UNIT_ENTRY_LEN as u32)
                .ok_or(StorageError::CorruptData)?,
        )
        .ok_or(StorageError::CorruptData)?;

    if paragraph_count == 0
        || unit_count == 0
        || paragraph_table_offset != expected_paragraph_offset
        || unit_table_offset != expected_unit_offset
        || expected_file_len != file_len
    {
        return Err(StorageError::CorruptData);
    }

    Ok(ReaderPackageHeader {
        title_len,
        paragraph_count,
        unit_count,
        paragraph_table_offset,
        unit_table_offset,
    })
}

fn read_reader_package_header(
    file: &mut SdFile<'_, '_>,
) -> Result<ReaderPackageHeader, StorageError> {
    file.seek_from_start(0).map_err(map_sd_error)?;
    let mut bytes = [0u8; READER_PACKAGE_HEADER_LEN];
    read_exact_file(file, &mut bytes)?;
    decode_reader_package_header(&bytes, file.length())
}

fn read_reader_package_title(
    file: &mut SdFile<'_, '_>,
    header: ReaderPackageHeader,
) -> Result<InlineText<CONTENT_TITLE_MAX_BYTES>, StorageError> {
    let mut title = InlineText::new();
    let title_len = header.title_len as usize;
    if title_len == 0 {
        title.set_truncated("UNTITLED ARTICLE");
        return Ok(title);
    }

    let mut bytes = alloc::vec![0u8; title_len];
    file.seek_from_start(READER_PACKAGE_HEADER_LEN as u32)
        .map_err(map_sd_error)?;
    read_exact_file(file, &mut bytes)?;
    let value = str::from_utf8(&bytes).map_err(|_| StorageError::CorruptData)?;
    title.set_truncated(value);
    if title.is_empty() {
        title.set_truncated("UNTITLED ARTICLE");
    }
    Ok(title)
}

fn decode_reader_package_paragraph_entry(
    bytes: &[u8],
) -> Result<ReaderParagraphInfo, StorageError> {
    if bytes.len() < READER_PACKAGE_PARAGRAPH_ENTRY_LEN {
        return Err(StorageError::CorruptData);
    }

    let preview_len = bytes[4] as usize;
    if preview_len > MAX_PARAGRAPH_PREVIEW_BYTES || 8 + preview_len > bytes.len() {
        return Err(StorageError::CorruptData);
    }

    let preview_text =
        str::from_utf8(&bytes[8..8 + preview_len]).map_err(|_| StorageError::CorruptData)?;
    let mut preview = InlineText::new();
    preview.set_truncated(preview_text);
    Ok(ReaderParagraphInfo {
        start_unit_index: read_u32(bytes, 0),
        preview,
    })
}

fn read_reader_package_paragraphs(
    file: &mut SdFile<'_, '_>,
    header: ReaderPackageHeader,
) -> Result<Box<[ReaderParagraphInfo]>, StorageError> {
    file.seek_from_start(header.paragraph_table_offset)
        .map_err(map_sd_error)?;
    let mut paragraphs = Vec::with_capacity(header.paragraph_count as usize);
    let mut bytes = [0u8; READER_PACKAGE_PARAGRAPH_ENTRY_LEN];
    let mut index = 0usize;
    let mut previous_start = None;
    while index < header.paragraph_count as usize {
        read_exact_file(file, &mut bytes)?;
        let paragraph = decode_reader_package_paragraph_entry(&bytes)?;
        if paragraph.start_unit_index >= header.unit_count
            || previous_start.is_some_and(|previous| paragraph.start_unit_index < previous)
        {
            return Err(StorageError::CorruptData);
        }
        previous_start = Some(paragraph.start_unit_index);
        paragraphs.push(paragraph);
        index += 1;
    }
    Ok(crate::memory_policy::external_or_global_boxed_slice(
        paragraphs,
    ))
}

fn font_from_byte(byte: u8) -> Result<StageFont, StorageError> {
    match byte {
        0 => Ok(StageFont::Large),
        1 => Ok(StageFont::Medium),
        2 => Ok(StageFont::Small),
        _ => Err(StorageError::CorruptData),
    }
}

fn flags_from_byte(byte: u8) -> UnitFlags {
    UnitFlags {
        clause_pause: (byte & 0b0001) != 0,
        sentence_pause: (byte & 0b0010) != 0,
        paragraph_start: (byte & 0b0100) != 0,
        paragraph_end: (byte & 0b1000) != 0,
    }
}

fn decode_reader_package_unit_entry(
    bytes: &[u8],
    paragraph_count: u16,
) -> Result<domain::formatter::ReadingUnit, StorageError> {
    if bytes.len() < READER_PACKAGE_UNIT_ENTRY_LEN {
        return Err(StorageError::CorruptData);
    }

    let paragraph_index = read_u16(bytes, 0);
    let display_len = bytes[6] as usize;
    if paragraph_index == 0
        || paragraph_index > paragraph_count
        || display_len == 0
        || display_len > MAX_READING_TOKEN_BYTES
        || 8 + display_len > bytes.len()
    {
        return Err(StorageError::CorruptData);
    }

    let display_text =
        str::from_utf8(&bytes[8..8 + display_len]).map_err(|_| StorageError::CorruptData)?;
    let mut display = InlineText::new();
    display.set_truncated(display_text);
    if display.is_empty() {
        return Err(StorageError::CorruptData);
    }

    let char_count = bytes[3];
    if char_count == 0 {
        return Err(StorageError::CorruptData);
    }

    Ok(domain::formatter::ReadingUnit {
        display,
        paragraph_index: paragraph_index.min(u8::MAX as u16) as u8,
        anchor_index: bytes[2].min(char_count.saturating_sub(1)),
        char_count,
        font: font_from_byte(bytes[4])?,
        flags: flags_from_byte(bytes[5]),
    })
}

fn read_reader_package_window(
    file: &mut SdFile<'_, '_>,
    header: ReaderPackageHeader,
    window_start_unit_index: u32,
) -> Result<Box<ReaderWindow>, StorageError> {
    if window_start_unit_index >= header.unit_count {
        return Err(StorageError::CorruptData);
    }

    let mut window = ReaderWindow::empty();
    let remaining = header.unit_count.saturating_sub(window_start_unit_index);
    let unit_count = remaining.min(READER_WINDOW_MAX_UNITS as u32) as usize;
    let start_offset = header
        .unit_table_offset
        .checked_add(
            window_start_unit_index
                .checked_mul(READER_PACKAGE_UNIT_ENTRY_LEN as u32)
                .ok_or(StorageError::CorruptData)?,
        )
        .ok_or(StorageError::CorruptData)?;
    file.seek_from_start(start_offset).map_err(map_sd_error)?;

    let mut bytes = [0u8; READER_PACKAGE_UNIT_ENTRY_LEN];
    let mut index = 0usize;
    while index < unit_count {
        read_exact_file(file, &mut bytes)?;
        window.units[index] = decode_reader_package_unit_entry(&bytes, header.paragraph_count)?;
        index += 1;
    }
    window.start_unit_index = window_start_unit_index;
    window.unit_count = unit_count as u16;
    Ok(crate::memory_policy::external_or_global_box(window))
}

fn detail_locator_to_byte(locator: DetailLocator) -> u8 {
    match locator {
        DetailLocator::Saved => 0,
        DetailLocator::Inbox => 1,
        DetailLocator::Content => 2,
    }
}

fn detail_locator_from_byte(byte: u8) -> Result<DetailLocator, StorageError> {
    match byte {
        0 => Ok(DetailLocator::Saved),
        1 => Ok(DetailLocator::Inbox),
        2 => Ok(DetailLocator::Content),
        _ => Err(StorageError::CorruptData),
    }
}

fn source_kind_to_byte(kind: domain::source::SourceKind) -> u8 {
    match kind {
        domain::source::SourceKind::PersonalQueue => 0,
        domain::source::SourceKind::EditorialFeed => 1,
        domain::source::SourceKind::Import => 2,
        domain::source::SourceKind::Unknown => 3,
    }
}

fn source_kind_from_byte(byte: u8) -> Result<domain::source::SourceKind, StorageError> {
    match byte {
        0 => Ok(domain::source::SourceKind::PersonalQueue),
        1 => Ok(domain::source::SourceKind::EditorialFeed),
        2 => Ok(domain::source::SourceKind::Import),
        3 => Ok(domain::source::SourceKind::Unknown),
        _ => Err(StorageError::CorruptData),
    }
}

fn remote_status_to_byte(status: RemoteContentStatus) -> u8 {
    match status {
        RemoteContentStatus::Ready => 0,
        RemoteContentStatus::Pending => 1,
        RemoteContentStatus::Failed => 2,
        RemoteContentStatus::Unknown => 3,
    }
}

fn remote_status_from_byte(byte: u8) -> Result<RemoteContentStatus, StorageError> {
    match byte {
        0 => Ok(RemoteContentStatus::Ready),
        1 => Ok(RemoteContentStatus::Pending),
        2 => Ok(RemoteContentStatus::Failed),
        3 => Ok(RemoteContentStatus::Unknown),
        _ => Err(StorageError::CorruptData),
    }
}

fn package_state_to_byte(state: PackageState) -> u8 {
    match state {
        PackageState::Missing => 0,
        PackageState::Cached => 1,
        PackageState::Stale => 2,
        PackageState::Fetching => 3,
        PackageState::PendingRemote => 4,
        PackageState::Failed => 5,
    }
}

fn package_state_from_byte(byte: u8) -> Result<PackageState, StorageError> {
    match byte {
        0 => Ok(PackageState::Missing),
        1 => Ok(PackageState::Cached),
        2 => Ok(PackageState::Stale),
        3 => Ok(PackageState::Fetching),
        4 => Ok(PackageState::PendingRemote),
        5 => Ok(PackageState::Failed),
        _ => Err(StorageError::CorruptData),
    }
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn crc32_continue(current: u32, bytes: &[u8]) -> u32 {
    let mut crc = current;
    for byte in bytes {
        crc ^= *byte as u32;
        let mut bit = 0;
        while bit < 8 {
            let mask = (crc & 1).wrapping_neg() & 0xEDB8_8320;
            crc = (crc >> 1) ^ mask;
            bit += 1;
        }
    }
    crc
}

trait JsonSource {
    fn read_chunk(&mut self, out: &mut [u8]) -> Result<usize, StorageError>;
}

impl<T> JsonSource for &mut T
where
    T: JsonSource + ?Sized,
{
    fn read_chunk(&mut self, out: &mut [u8]) -> Result<usize, StorageError> {
        (**self).read_chunk(out)
    }
}

struct SliceJsonSource<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> SliceJsonSource<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }
}

impl JsonSource for SliceJsonSource<'_> {
    fn read_chunk(&mut self, out: &mut [u8]) -> Result<usize, StorageError> {
        let remaining = &self.bytes[self.offset..];
        if remaining.is_empty() {
            return Ok(0);
        }

        let read = remaining.len().min(out.len());
        out[..read].copy_from_slice(&remaining[..read]);
        self.offset += read;
        Ok(read)
    }
}

struct SdPackageSource<'a, 'd> {
    file: SdFile<'a, 'd>,
    bytes_read: usize,
    crc32: u32,
}

impl<'a, 'd> SdPackageSource<'a, 'd> {
    fn new(file: SdFile<'a, 'd>) -> Self {
        Self {
            file,
            bytes_read: 0,
            crc32: 0xFFFF_FFFF,
        }
    }

    fn finish(&mut self) -> Result<(), StorageError> {
        let mut buffer = [0u8; PACKAGE_READ_BUFFER_LEN];
        loop {
            let read = self.read_chunk(&mut buffer)?;
            if read == 0 {
                break;
            }
        }
        Ok(())
    }

    fn bytes_read(&self) -> usize {
        self.bytes_read
    }

    fn crc32(&self) -> u32 {
        !self.crc32
    }
}

impl JsonSource for SdPackageSource<'_, '_> {
    fn read_chunk(&mut self, out: &mut [u8]) -> Result<usize, StorageError> {
        let read = self.file.read(out).map_err(map_sd_error)?;
        self.bytes_read = self.bytes_read.saturating_add(read);
        self.crc32 = crc32_continue(self.crc32, &out[..read]);
        Ok(read)
    }
}

#[derive(Debug, Default)]
struct ParsedString {
    value: String,
    truncated: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
enum BlockKind {
    Text,
    List,
    #[default]
    Other,
}

#[derive(Debug, Default)]
struct BlockDraft {
    kind: BlockKind,
    text: Option<String>,
    ordered: bool,
    items: Vec<String>,
    truncated: bool,
    list_bytes: usize,
}

impl BlockDraft {
    fn parse<S: JsonSource>(stream: &mut JsonStream<S>) -> Result<Self, StorageError> {
        let mut draft = Self::default();
        stream.parse_object_fields(|stream, key| match key.as_str() {
            "t" => {
                let parsed = stream.parse_string_limited(8)?;
                if parsed.truncated {
                    draft.truncated = true;
                }
                draft.kind = match parsed.value.as_str() {
                    "h" | "p" | "q" | "c" => BlockKind::Text,
                    "l" => BlockKind::List,
                    _ => BlockKind::Other,
                };
                Ok(())
            }
            "x" => {
                let parsed = stream.parse_string_limited(MAX_PARSED_BLOCK_TEXT_BYTES)?;
                if parsed.truncated {
                    draft.truncated = true;
                }
                draft.text = Some(parsed.value);
                Ok(())
            }
            "o" => {
                draft.ordered = stream.parse_bool()?;
                Ok(())
            }
            "i" => draft.parse_list_items(stream),
            _ => stream.skip_value(),
        })?;
        Ok(draft)
    }

    fn parse_list_items<S: JsonSource>(
        &mut self,
        stream: &mut JsonStream<S>,
    ) -> Result<(), StorageError> {
        stream.parse_array_values(|stream, first| {
            if first != b'"' {
                return Err(StorageError::CorruptData);
            }

            let parsed = stream.parse_string_body_limited(MAX_PARSED_BLOCK_TEXT_BYTES)?;
            if parsed.truncated {
                self.truncated = true;
            }

            let trimmed = parsed.value.trim();
            if trimmed.is_empty() {
                return Ok(());
            }

            if self.items.len() >= MAX_PARSED_LIST_ITEMS
                || self.list_bytes.saturating_add(trimmed.len()) > MAX_PARSED_LIST_TOTAL_BYTES
            {
                self.truncated = true;
                return Ok(());
            }

            self.list_bytes = self.list_bytes.saturating_add(trimmed.len());
            self.items.push(String::from(trimmed));
            Ok(())
        })
    }

    fn apply(self, document: &mut ReadingDocument) -> bool {
        let mut truncated = self.truncated;

        match self.kind {
            BlockKind::Text => {
                if let Some(text) = self.text {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() && !document.push_paragraph_text(trimmed) {
                        truncated = true;
                    }
                }
            }
            BlockKind::List => {
                let mut item_index = 0usize;
                while item_index < self.items.len() {
                    let line = format_list_line(&self.items[item_index], self.ordered, item_index);
                    if !document.push_paragraph_text(line.as_str()) {
                        truncated = true;
                        break;
                    }
                    item_index += 1;
                }
            }
            BlockKind::Other => {}
        }

        truncated
    }
}

struct JsonStream<S> {
    source: S,
    buffer: [u8; PACKAGE_READ_BUFFER_LEN],
    cursor: usize,
    buffered: usize,
    unread: Option<u8>,
}

impl<S: JsonSource> JsonStream<S> {
    fn new(source: S) -> Self {
        Self {
            source,
            buffer: [0u8; PACKAGE_READ_BUFFER_LEN],
            cursor: 0,
            buffered: 0,
            unread: None,
        }
    }

    fn next_byte(&mut self) -> Result<Option<u8>, StorageError> {
        if let Some(byte) = self.unread.take() {
            return Ok(Some(byte));
        }

        if self.cursor == self.buffered {
            self.buffered = self.source.read_chunk(&mut self.buffer)?;
            self.cursor = 0;
            if self.buffered == 0 {
                return Ok(None);
            }
        }

        let byte = self.buffer[self.cursor];
        self.cursor += 1;
        Ok(Some(byte))
    }

    fn unread_byte(&mut self, byte: u8) {
        debug_assert!(self.unread.is_none());
        self.unread = Some(byte);
    }

    fn next_significant_byte(&mut self) -> Result<Option<u8>, StorageError> {
        loop {
            let Some(byte) = self.next_byte()? else {
                return Ok(None);
            };
            if !byte.is_ascii_whitespace() {
                return Ok(Some(byte));
            }
        }
    }

    fn expect_byte(&mut self, expected: u8) -> Result<(), StorageError> {
        let byte = self
            .next_significant_byte()?
            .ok_or(StorageError::CorruptData)?;
        if byte != expected {
            return Err(StorageError::CorruptData);
        }
        Ok(())
    }

    fn parse_bool(&mut self) -> Result<bool, StorageError> {
        let first = self
            .next_significant_byte()?
            .ok_or(StorageError::CorruptData)?;
        self.parse_bool_from(first)
    }

    fn parse_bool_from(&mut self, first: u8) -> Result<bool, StorageError> {
        match first {
            b't' => {
                self.expect_literal(b"rue")?;
                Ok(true)
            }
            b'f' => {
                self.expect_literal(b"alse")?;
                Ok(false)
            }
            _ => Err(StorageError::CorruptData),
        }
    }

    fn parse_string_limited(&mut self, max_bytes: usize) -> Result<ParsedString, StorageError> {
        let opening = self
            .next_significant_byte()?
            .ok_or(StorageError::CorruptData)?;
        if opening != b'"' {
            return Err(StorageError::CorruptData);
        }
        self.parse_string_body_limited(max_bytes)
    }

    fn parse_string_body_limited(
        &mut self,
        max_bytes: usize,
    ) -> Result<ParsedString, StorageError> {
        let mut parsed = ParsedString {
            value: String::new(),
            truncated: false,
        };

        loop {
            let byte = self.next_byte()?.ok_or(StorageError::CorruptData)?;
            match byte {
                b'"' => return Ok(parsed),
                b'\\' => {
                    let escaped = self.next_byte()?.ok_or(StorageError::CorruptData)?;
                    match escaped {
                        b'"' | b'\\' | b'/' => {
                            push_limited_char(
                                &mut parsed.value,
                                escaped as char,
                                max_bytes,
                                &mut parsed.truncated,
                            );
                        }
                        b'b' | b'f' | b'n' | b'r' | b't' => {
                            push_limited_char(
                                &mut parsed.value,
                                ' ',
                                max_bytes,
                                &mut parsed.truncated,
                            );
                        }
                        b'u' => {
                            let codepoint = self.parse_unicode_escape()?;
                            push_limited_char(
                                &mut parsed.value,
                                char::from_u32(codepoint).unwrap_or('?'),
                                max_bytes,
                                &mut parsed.truncated,
                            );
                        }
                        _ => return Err(StorageError::CorruptData),
                    }
                }
                byte if byte.is_ascii() => {
                    push_limited_char(
                        &mut parsed.value,
                        byte as char,
                        max_bytes,
                        &mut parsed.truncated,
                    );
                }
                byte => {
                    let continuation = utf8_continuation_len(byte)?;
                    let mut utf8 = [0u8; 4];
                    utf8[0] = byte;
                    let mut index = 0usize;
                    while index < continuation {
                        let next = self.next_byte()?.ok_or(StorageError::CorruptData)?;
                        if next & 0b1100_0000 != 0b1000_0000 {
                            return Err(StorageError::CorruptData);
                        }
                        utf8[index + 1] = next;
                        index += 1;
                    }
                    let text = core::str::from_utf8(&utf8[..continuation + 1])
                        .map_err(|_| StorageError::CorruptData)?;
                    push_limited_str(&mut parsed.value, text, max_bytes, &mut parsed.truncated);
                }
            }
        }
    }

    fn parse_object_fields<F>(&mut self, mut handler: F) -> Result<(), StorageError>
    where
        F: FnMut(&mut Self, String) -> Result<(), StorageError>,
    {
        self.expect_byte(b'{')?;
        let mut first = true;

        loop {
            let next = self
                .next_significant_byte()?
                .ok_or(StorageError::CorruptData)?;
            match next {
                b'}' => return Ok(()),
                b',' if !first => {}
                _ if first => self.unread_byte(next),
                _ => return Err(StorageError::CorruptData),
            }

            let opening = self
                .next_significant_byte()?
                .ok_or(StorageError::CorruptData)?;
            if opening != b'"' {
                return Err(StorageError::CorruptData);
            }

            let key = self.parse_string_body_limited(MAX_JSON_KEY_BYTES)?.value;
            self.expect_byte(b':')?;
            handler(self, key)?;
            first = false;
        }
    }

    fn parse_array_values<F>(&mut self, mut handler: F) -> Result<(), StorageError>
    where
        F: FnMut(&mut Self, u8) -> Result<(), StorageError>,
    {
        self.expect_byte(b'[')?;
        let mut first = true;

        loop {
            let next = self
                .next_significant_byte()?
                .ok_or(StorageError::CorruptData)?;
            match next {
                b']' => return Ok(()),
                b',' if !first => {}
                _ if first => self.unread_byte(next),
                _ => return Err(StorageError::CorruptData),
            }

            let first_byte = self
                .next_significant_byte()?
                .ok_or(StorageError::CorruptData)?;
            handler(self, first_byte)?;
            first = false;
        }
    }

    fn skip_value(&mut self) -> Result<(), StorageError> {
        let first = self
            .next_significant_byte()?
            .ok_or(StorageError::CorruptData)?;
        self.skip_value_from(first)
    }

    fn skip_value_from(&mut self, first: u8) -> Result<(), StorageError> {
        match first {
            b'{' => self.skip_object_body(),
            b'[' => self.skip_array_body(),
            b'"' => {
                let _ = self.parse_string_body_limited(0)?;
                Ok(())
            }
            b't' => self.expect_literal(b"rue"),
            b'f' => self.expect_literal(b"alse"),
            b'n' => self.expect_literal(b"ull"),
            b'-' | b'0'..=b'9' => self.skip_number_from(first),
            _ => Err(StorageError::CorruptData),
        }
    }

    fn skip_object_body(&mut self) -> Result<(), StorageError> {
        let mut first = true;
        loop {
            let next = self
                .next_significant_byte()?
                .ok_or(StorageError::CorruptData)?;
            match next {
                b'}' => return Ok(()),
                b',' if !first => {}
                _ if first => self.unread_byte(next),
                _ => return Err(StorageError::CorruptData),
            }

            let opening = self
                .next_significant_byte()?
                .ok_or(StorageError::CorruptData)?;
            if opening != b'"' {
                return Err(StorageError::CorruptData);
            }
            let _ = self.parse_string_body_limited(0)?;
            self.expect_byte(b':')?;
            self.skip_value()?;
            first = false;
        }
    }

    fn skip_array_body(&mut self) -> Result<(), StorageError> {
        let mut first = true;
        loop {
            let next = self
                .next_significant_byte()?
                .ok_or(StorageError::CorruptData)?;
            match next {
                b']' => return Ok(()),
                b',' if !first => {}
                _ if first => self.unread_byte(next),
                _ => return Err(StorageError::CorruptData),
            }

            let first_byte = self
                .next_significant_byte()?
                .ok_or(StorageError::CorruptData)?;
            self.skip_value_from(first_byte)?;
            first = false;
        }
    }

    fn skip_number_from(&mut self, first: u8) -> Result<(), StorageError> {
        if !matches!(first, b'-' | b'0'..=b'9') {
            return Err(StorageError::CorruptData);
        }

        loop {
            let Some(byte) = self.next_byte()? else {
                return Ok(());
            };
            if byte.is_ascii_whitespace() || matches!(byte, b',' | b']' | b'}') {
                self.unread_byte(byte);
                return Ok(());
            }
            if !matches!(byte, b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-') {
                return Err(StorageError::CorruptData);
            }
        }
    }

    fn expect_literal(&mut self, literal: &[u8]) -> Result<(), StorageError> {
        let mut index = 0usize;
        while index < literal.len() {
            let byte = self.next_byte()?.ok_or(StorageError::CorruptData)?;
            if byte != literal[index] {
                return Err(StorageError::CorruptData);
            }
            index += 1;
        }
        Ok(())
    }

    fn parse_unicode_escape(&mut self) -> Result<u32, StorageError> {
        let mut value = 0u32;
        let mut index = 0usize;
        while index < 4 {
            let byte = self.next_byte()?.ok_or(StorageError::CorruptData)?;
            value = (value << 4) | hex_value(byte)? as u32;
            index += 1;
        }
        Ok(value)
    }
}

fn parse_opened_reader_content<S: JsonSource>(
    source: S,
) -> Result<OpenedReaderContent, StorageError> {
    let mut stream = JsonStream::new(source);
    let mut title = InlineText::new();
    let mut document = ReadingDocument::boxed_empty();
    let mut truncated = false;
    let mut content_found = false;
    let mut body_found = false;
    let mut blocks_found = false;
    let mut body_kind_supported = false;

    let parse_result = stream.parse_object_fields(|stream, key| match key.as_str() {
        "content" => {
            content_found = true;
            stream.parse_object_fields(|stream, key| match key.as_str() {
                "title" => {
                    let parsed = stream.parse_string_limited(MAX_PARSED_TITLE_BYTES)?;
                    if parsed.truncated {
                        truncated = true;
                    }
                    title.set_truncated(parsed.value.as_str());
                    Ok(())
                }
                "body" => {
                    body_found = true;
                    stream.parse_object_fields(|stream, key| match key.as_str() {
                        "kind" => {
                            let parsed = stream.parse_string_limited(16)?;
                            if parsed.truncated {
                                truncated = true;
                            }
                            body_kind_supported =
                                is_supported_reader_body_kind(parsed.value.as_str());
                            Ok(())
                        }
                        "blocks" => {
                            blocks_found = true;
                            stream.parse_array_values(|stream, first| {
                                if first != b'{' {
                                    return Err(StorageError::CorruptData);
                                }
                                stream.unread_byte(first);
                                let block = BlockDraft::parse(stream)?;
                                if block.apply(&mut document) {
                                    truncated = true;
                                }
                                Ok(())
                            })
                        }
                        _ => stream.skip_value(),
                    })
                }
                _ => stream.skip_value(),
            })
        }
        _ => stream.skip_value(),
    });
    if let Err(err) = parse_result {
        info!(
            "content storage reader parse stream error content_found={} body_found={} blocks_found={} body_kind_supported={} title_empty={} document_empty={} truncated={} err={:?}",
            content_found,
            body_found,
            blocks_found,
            body_kind_supported,
            title.is_empty(),
            document.is_empty(),
            truncated,
            err,
        );
        return Err(err);
    }

    if !content_found || !body_found || !blocks_found || !body_kind_supported || document.is_empty()
    {
        info!(
            "content storage reader parse rejected content_found={} body_found={} blocks_found={} body_kind_supported={} title_empty={} document_empty={} truncated={}",
            content_found,
            body_found,
            blocks_found,
            body_kind_supported,
            title.is_empty(),
            document.is_empty(),
            truncated,
        );
        return Err(StorageError::CorruptData);
    }

    if title.is_empty() {
        title.set_truncated("UNTITLED ARTICLE");
    }

    Ok(OpenedReaderContent {
        title,
        document,
        truncated,
    })
}

fn is_supported_reader_body_kind(kind: &str) -> bool {
    matches!(
        kind,
        "compact" | "article" | "thread" | "post" | "website" | "video" | "podcast" | "pdf"
    )
}

fn push_limited_char(target: &mut String, ch: char, max_bytes: usize, truncated: &mut bool) {
    let mut utf8 = [0u8; 4];
    let encoded = ch.encode_utf8(&mut utf8);
    push_limited_str(target, encoded, max_bytes, truncated);
}

fn push_limited_str(target: &mut String, value: &str, max_bytes: usize, truncated: &mut bool) {
    if *truncated || value.is_empty() {
        return;
    }

    if target.len().saturating_add(value.len()) > max_bytes {
        *truncated = true;
        return;
    }

    target.push_str(value);
}

fn utf8_continuation_len(first: u8) -> Result<usize, StorageError> {
    match first {
        0xC2..=0xDF => Ok(1),
        0xE0..=0xEF => Ok(2),
        0xF0..=0xF4 => Ok(3),
        _ => Err(StorageError::CorruptData),
    }
}

fn hex_value(byte: u8) -> Result<u8, StorageError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(StorageError::CorruptData),
    }
}

fn format_list_line(item: &str, ordered: bool, index: usize) -> String {
    let mut line = String::new();
    if ordered {
        let number = index + 1;
        if number >= 10 {
            let hundreds = (number / 100) % 10;
            if hundreds > 0 {
                line.push((b'0' + hundreds as u8) as char);
            }
            let tens = (number / 10) % 10;
            line.push((b'0' + tens as u8) as char);
        }
        line.push((b'0' + (number % 10) as u8) as char);
        line.push_str(". ");
    } else {
        line.push_str("- ");
    }
    line.push_str(item);
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;
    use core::mem::size_of;

    #[test]
    fn manifest_snapshot_round_trips() {
        let mut snapshot = CollectionManifestState::empty();
        snapshot.serve_id.set_truncated("serve-1");
        let mut item = CollectionManifestItem::empty();
        item.remote_item_id.set_truncated("saved-1");
        item.content_id.set_truncated("content-1");
        item.title.set_truncated("Example Title");
        item.meta.set_truncated("EXAMPLE / SAVED");
        item.remote_revision = 42;
        item.remote_status = RemoteContentStatus::Ready;
        item.package_state = PackageState::Cached;
        let _ = snapshot.try_push(item);

        let mut encoded = [0u8; MAX_MANIFEST_SNAPSHOT_LEN];
        let encoded_len =
            encode_manifest_snapshot(CollectionKind::Saved, &snapshot, &mut encoded).unwrap();
        let decoded =
            decode_manifest_snapshot(CollectionKind::Saved, &encoded[..encoded_len]).unwrap();

        assert_eq!(decoded, snapshot);
    }

    #[test]
    fn cache_index_round_trips() {
        let mut index = CacheIndex::empty();
        let mut content_id = InlineText::new();
        content_id.set_truncated("content-1");
        index.upsert(CacheEntry {
            slot_id: 1,
            content_id,
            remote_revision: 88,
            size_bytes: 1234,
            crc32: 0xDEADBEEF,
            last_touch_seq: 0,
            collection_flags: collection_flag(CollectionKind::Saved),
        });

        let mut encoded = [0u8; MAX_CACHE_INDEX_LEN];
        let encoded_len = encode_cache_index(&index, &mut encoded).unwrap();
        let decoded = decode_cache_index(&encoded[..encoded_len]).unwrap();

        assert_eq!(decoded, index);
    }

    #[test]
    fn eviction_prefers_recommendations_before_saved() {
        let mut saved_id = InlineText::new();
        saved_id.set_truncated("saved");
        let mut rec_id = InlineText::new();
        rec_id.set_truncated("rec");

        let saved = CacheEntry {
            slot_id: 1,
            content_id: saved_id,
            remote_revision: 1,
            size_bytes: 1,
            crc32: 1,
            last_touch_seq: 10,
            collection_flags: collection_flag(CollectionKind::Saved),
        };
        let recommendation = CacheEntry {
            slot_id: 2,
            content_id: rec_id,
            remote_revision: 1,
            size_bytes: 1,
            crc32: 1,
            last_touch_seq: 10,
            collection_flags: collection_flag(CollectionKind::Recommendations),
        };

        assert_eq!(
            compare_eviction_priority(recommendation, saved),
            Ordering::Less
        );
    }

    #[test]
    fn reader_content_parser_opens_backend_article_payload() {
        let payload = br#"{
            "content": {
                "title": "Example article",
                "body": {
                    "kind": "article",
                    "blocks": [
                        {"x": "First paragraph for Motif.", "t": "p"},
                        {"i": ["Alpha", "Beta"], "o": true, "t": "l"}
                    ]
                }
            }
        }"#;

        let opened = parse_reader_content_bytes(payload).unwrap();

        assert_eq!(opened.title.as_str(), "Example article");
        assert!(!opened.truncated);
        assert_eq!(opened.document.paragraph_count, 3);
        assert_eq!(
            opened.document.preview_for_paragraph(1).as_str(),
            "First paragraph for Motif."
        );
    }

    #[test]
    fn reader_content_parser_keeps_legacy_compact_kind_compatibility() {
        let payload = br#"{
            "content": {
                "title": "Legacy article",
                "body": {
                    "kind": "compact",
                    "blocks": [{"x": "Legacy paragraph.", "t": "p"}]
                }
            }
        }"#;

        let opened = parse_reader_content_bytes(payload).unwrap();

        assert_eq!(opened.title.as_str(), "Legacy article");
        assert_eq!(opened.document.paragraph_count, 1);
        assert_eq!(
            opened.document.preview_for_paragraph(1).as_str(),
            "Legacy paragraph."
        );
    }

    #[test]
    fn reader_content_parser_rejects_invalid_kind() {
        let payload = br#"{
            "content": {
                "title": "Broken",
                "body": {
                    "kind": "full",
                    "blocks": [{"t": "p", "x": "ignored"}]
                }
            }
        }"#;

        assert_eq!(
            parse_reader_content_bytes(payload).unwrap_err(),
            StorageError::CorruptData
        );
    }

    #[test]
    fn reader_content_parser_truncates_oversized_payloads_safely() {
        let mut payload =
            String::from(r#"{"content":{"title":"Oversized","body":{"kind":"compact","blocks":["#);

        let mut index = 0usize;
        while index < 40 {
            if index > 0 {
                payload.push(',');
            }
            payload.push_str(r#"{"t":"p","x":"alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho sigma tau upsilon phi chi psi omega alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho sigma tau upsilon phi chi psi omega"}"#);
            index += 1;
        }

        payload.push_str("]}}}");
        let opened = parse_reader_content_bytes(payload.as_bytes()).unwrap();

        assert!(opened.truncated);
        assert!(opened.document.unit_count as usize <= MAX_READING_UNITS);
        assert!(opened.document.paragraph_count as usize <= MAX_READING_PARAGRAPHS);
        assert!(!opened.document.is_empty());
    }

    #[test]
    fn reader_content_and_session_sizes_stay_bounded() {
        assert!(size_of::<OpenedReaderContent>() < 256);
        assert!(size_of::<domain::reader::ReaderSession>() < 256);
        assert!(size_of::<domain::runtime::BootstrapSnapshot>() < 128);
    }

    #[test]
    fn sd_layout_conflicts_map_to_corruption() {
        assert_eq!(
            map_sd_error::<()>(SdError::OpenedFileAsDir),
            StorageError::CorruptData
        );
        assert_eq!(
            map_sd_error::<()>(SdError::DeleteDirAsFile),
            StorageError::CorruptData
        );
    }
}
