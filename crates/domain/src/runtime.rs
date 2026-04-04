use alloc::boxed::Box;

use crate::{
    content::{
        CONTENT_TITLE_MAX_BYTES, CollectionKind, CollectionManifestState, PackageState,
        PrepareContentRequest, REMOTE_ITEM_ID_MAX_BYTES,
    },
    device::DeviceState,
    input::InputGesture,
    network::NetworkState,
    network::NetworkStatus,
    reader::{ReaderParagraphInfo, ReaderWindow, ReaderWindowLoadRequest},
    settings::PersistedSettings,
    storage::StorageHealth,
    sync::SyncStatus,
    text::InlineText,
};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum CollectionConfirmIgnoredReason {
    #[default]
    EmptyCollection,
    StorageUnavailable,
    BackendUnavailable,
    AlreadyFetching,
    PendingRemote,
    Failed,
    NotReady,
}

impl CollectionConfirmIgnoredReason {
    pub const fn label(self) -> &'static str {
        match self {
            Self::EmptyCollection => "empty_collection",
            Self::StorageUnavailable => "storage_unavailable",
            Self::BackendUnavailable => "backend_unavailable",
            Self::AlreadyFetching => "already_fetching",
            Self::PendingRemote => "pending_remote",
            Self::Failed => "failed",
            Self::NotReady => "not_ready",
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum UiCommand {
    #[default]
    Noop,
    FocusPrevious,
    FocusNext,
    Confirm,
    Back,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum Command {
    #[default]
    Noop,
    Boot,
    RequestDeepSleep,
    Ui(UiCommand),
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Eq, PartialEq, Default)]
pub enum Event {
    #[default]
    Noop,
    BootCompleted,
    InputGestureReceived(InputGesture),
    NetworkStatusChanged(NetworkStatus),
    BackendSyncStatusChanged(SyncStatus),
    CollectionContentUpdated(CollectionKind, Box<CollectionManifestState>),
    ReaderContentOpened {
        collection: CollectionKind,
        content_id: InlineText<{ crate::content::CONTENT_ID_MAX_BYTES }>,
        title: InlineText<CONTENT_TITLE_MAX_BYTES>,
        total_units: u32,
        paragraphs: Box<[ReaderParagraphInfo]>,
        window: Box<ReaderWindow>,
    },
    ContentPackageStateChanged {
        collection: CollectionKind,
        remote_item_id: InlineText<REMOTE_ITEM_ID_MAX_BYTES>,
        package_state: PackageState,
    },
    UiTick(u64),
    ReaderTick(u64),
    WokeFromDeepSleep,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum Effect {
    #[default]
    Noop,
    EnterDeepSleep,
    CollectionConfirmIgnored {
        collection: CollectionKind,
        reason: CollectionConfirmIgnoredReason,
    },
    OpenCachedContent(PrepareContentRequest),
    LoadReaderWindow(ReaderWindowLoadRequest),
    PrepareContent(PrepareContentRequest),
    PersistSettings(PersistedSettings),
}

#[derive(Debug, Eq, PartialEq)]
pub struct BootstrapSnapshot {
    pub device: DeviceState,
    pub boot_at_ms: u64,
    pub content: Option<Box<crate::content::ContentState>>,
    pub settings: Option<PersistedSettings>,
    pub storage: StorageHealth,
    pub network: NetworkState,
}

impl BootstrapSnapshot {
    pub fn new(
        device: DeviceState,
        boot_at_ms: u64,
        content: Option<Box<crate::content::ContentState>>,
        settings: Option<PersistedSettings>,
        storage: StorageHealth,
        network: NetworkState,
    ) -> Self {
        Self {
            device,
            boot_at_ms,
            content,
            settings,
            storage,
            network,
        }
    }
}

impl Default for BootstrapSnapshot {
    fn default() -> Self {
        Self::new(
            DeviceState::new(),
            0,
            None,
            None,
            StorageHealth::new(),
            NetworkState::disabled(),
        )
    }
}
