#![cfg_attr(
    not(all(
        feature = "telemetry-memtrace",
        feature = "telemetry-verbose-diagnostics"
    )),
    allow(unused_imports, unused_variables)
)]

extern crate alloc;

use alloc::boxed::Box;
use core::net::{IpAddr, Ipv4Addr};
use core::{ffi::CStr, fmt::Write as _, future::Future, mem::size_of, net::SocketAddr, str};

use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};
use embassy_net::{
    Stack,
    dns::DnsSocket,
    tcp::client::{TcpClient, TcpClientState, TcpConnection},
};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};
use embassy_time::{Duration, Instant, Timer, with_timeout};
use embedded_io as eio06;
use embedded_io_07 as eio07;
use embedded_io_async::{Read as AsyncRead06, Write as AsyncWrite06};
use embedded_io_async_07::{Read as AsyncRead07, Write as AsyncWrite07};
use embedded_nal_async::{AddrType, Dns as _, TcpConnect as _};
use esp_hal::{
    peripherals::{ADC1, RNG},
    rng::{Trng, TrngSource},
};
use log::{info, warn};
use mbedtls_rs::{
    Certificate, ClientSessionConfig, SerializedClientSession, Session, SessionConfig,
    SessionError, Tls, TlsReference, TlsVersion, X509,
};
use services::backend_sync::SyncStatus;
use services::storage::StorageError;

use crate::{
    bootstrap::{persist_backend_credential, publish_event},
    content_storage,
    storage::{BACKEND_REFRESH_TOKEN_MAX_LEN, BackendCredential},
    telemetry::{TraceContext, bool_flag, collection_label, next_request_id, next_sync_id},
    transfer_tuning,
};
use domain::{
    content::{
        CONTENT_META_MAX_BYTES, CONTENT_TITLE_MAX_BYTES, CollectionKind, CollectionManifestItem,
        CollectionManifestState, DetailLocator, MANIFEST_ITEM_CAPACITY, PackageState,
        PrepareContentPhase, PrepareContentProgress, PrepareContentRequest,
        RECOMMENDATION_SERVE_ID_MAX_BYTES, REMOTE_ITEM_ID_MAX_BYTES, RemoteContentStatus,
    },
    runtime::Event,
    text::InlineText,
};

pub(crate) const BACKEND_HOST: &str = "motif-backend-production-a143.up.railway.app";
const BACKEND_HOST_CSTR_BYTES: &[u8] = b"motif-backend-production-a143.up.railway.app\0";
const BACKEND_BASE_URL: &str = "https://motif-backend-production-a143.up.railway.app";
const HEALTH_PATH: &str = "/health";
const REFRESH_PATH: &str = "/device/v1/auth/session/refresh";
const ME_PATH: &str = "/device/v1/me";
const INBOX_PATH: &str = "/device/v1/me/inbox";
const SAVED_CONTENT_PATH: &str = "/device/v1/me/saved-content";
const RECOMMENDATIONS_PATH: &str = "/device/v1/me/recommendations/content";
pub(crate) const BACKEND_PORT: u16 = 443;
const NETWORK_POLL_MS: u64 = 500;
const RETRY_BACKOFF_MS: u64 = 10_000;
const TRANSPORT_RETRY_ATTEMPTS: usize = 2;
const PACKAGE_TRANSPORT_RETRY_ATTEMPTS: usize = 3;
const PACKAGE_RETRY_RECOVERY_ATTEMPTS: usize = 2;
const TRANSPORT_RETRY_BACKOFF_MS: u64 = 750;
const CONNECT_TIMEOUT_SECS: u64 = 5;
const AUTH_REFRESH_TLS_HANDSHAKE_TIMEOUT_SECS: u64 = 12;
const AUTH_REFRESH_IO_TIMEOUT_SECS: u64 = 12;
const AUTH_REFRESH_NETWORK_READY_TIMEOUT_SECS: u64 = 8;
const BUFFERED_METADATA_TLS_HANDSHAKE_TIMEOUT_SECS: u64 = 10;
const BUFFERED_METADATA_IO_TIMEOUT_SECS: u64 = 20;
const BUFFERED_METADATA_NETWORK_READY_TIMEOUT_SECS: u64 = 6;
const STREAMING_PACKAGE_TLS_HANDSHAKE_TIMEOUT_SECS: u64 = 12;
const STREAMING_PACKAGE_IO_TIMEOUT_SECS: u64 = 25;
const STREAMING_PACKAGE_NETWORK_READY_TIMEOUT_SECS: u64 = 12;
const PACKAGE_RETRY_NETWORK_READY_MAX_TIMEOUT_SECS: u64 = 30;
const REQUEST_NETWORK_READY_POLL_MS: u64 = 250;
// Real device traces showed that an aggressive background `/device/v1/me`
// keepalive could kill an otherwise healthy reusable TLS session between two
// article opens. Keep passive reuse for truly nearby follow-up requests, but
// apply a much stricter age limit to streaming package fetches because the
// server path was already resetting ~20 s idle package sockets on first write
// and even 10 s reuse windows were producing multi-second stalls before the
// next request reached first byte.
const REUSABLE_BUFFERED_SESSION_IDLE_TIMEOUT_SECS: u64 = 60;
const REUSABLE_STREAMING_SESSION_IDLE_TIMEOUT_SECS: u64 = 3;
const MBEDTLS_DEBUG_LEVEL: u32 = 0;
const STREAM_PROGRESS_LOG_INTERVAL_BYTES: usize = 16 * 1024;
const HTTP_RESPONSE_MAX_LEN: usize = 8 * 1024;
const HTTP_STREAM_HEADER_MAX_LEN: usize = 2048;
const COLLECTION_PAGE_LIMIT: usize = 4;
const COLLECTION_CURSOR_MAX_LEN: usize = 192;
const COLLECTION_PAGE_PATH_MAX_LEN: usize = 320;
const COLLECTION_FETCH_MAX_PAGES: usize = 32;
const REFRESH_BODY_OVERHEAD_LEN: usize = "{\"refresh_token\":\"\"}".len();
const REQUEST_BODY_MAX_LEN: usize = REFRESH_BODY_OVERHEAD_LEN + (BACKEND_REFRESH_TOKEN_MAX_LEN * 2);
const INBOX_LOG_PREVIEW_MAX_LEN: usize = 256;
const PACKAGE_DOWNLOAD_CHUNK_LEN: usize = transfer_tuning::PACKAGE_TRANSFER_CHUNK_LEN;
const PACKAGE_STORAGE_HANDOFF_CHUNK_LEN: usize =
    transfer_tuning::PACKAGE_TRANSFER_STORAGE_HANDOFF_CHUNK_LEN;
const PREPARE_PROGRESS_DOWNLOAD_STEP_BYTES: usize = 24 * 1024;
const PREPARE_PROGRESS_MIN_DOWNLOAD_STEPS: u16 = 3;
const PREPARE_PROGRESS_MAX_DOWNLOAD_STEPS: u16 = 8;
const PREPARE_PROGRESS_FIXED_STEPS: u16 = 3;
// Package prefetch materially increases boot-time latency and was timing out on
// real device/package responses. Keep startup focused on refresh + manifest sync.
const STARTUP_SAVED_PREFETCH_ENABLED: bool = false;
const STARTUP_SAVED_PREFETCH_LIMIT: usize = 1;
const USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));
const BACKEND_CA_CHAIN_PEM: &str =
    concat!(include_str!("../certs/letsencrypt_isrg_root_x1.pem"), "\0");
const BACKEND_CMD_QUEUE_CAPACITY: usize = 4;
// Large article fetches are now dominated by body transfer rather than SD or
// commit/open overhead. Widen only the receive side so the TCP window can hold
// more inbound package data without paying for a larger transmit buffer we do
// not meaningfully use on tiny GET/refresh requests.
const BACKEND_TCP_RX_BUFFER_LEN: usize = 16 * 1024;
const BACKEND_TCP_TX_BUFFER_LEN: usize = 4 * 1024;
type BackendTcpClientState =
    TcpClientState<1, BACKEND_TCP_RX_BUFFER_LEN, BACKEND_TCP_TX_BUFFER_LEN>;
type BackendTcpClient<'a> = TcpClient<'a, 1, BACKEND_TCP_RX_BUFFER_LEN, BACKEND_TCP_TX_BUFFER_LEN>;
type BackendTcpConnection<'a> =
    TcpConnection<'a, 1, BACKEND_TCP_RX_BUFFER_LEN, BACKEND_TCP_TX_BUFFER_LEN>;
type BackendTlsSession<'a> = Session<'a, CompatConnection<BackendTcpConnection<'a>>>;

static BACKEND_CMD_CH: Channel<
    CriticalSectionRawMutex,
    BackendCommand,
    BACKEND_CMD_QUEUE_CAPACITY,
> = Channel::new();

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum CredentialSource {
    CompileTime,
    Stored,
}

impl CredentialSource {
    fn label(self) -> &'static str {
        match self {
            Self::CompileTime => "compile_time",
            Self::Stored => "stored",
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct StartupCredential {
    pub credential: BackendCredential,
    pub source: CredentialSource,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct NetworkReady {
    address: embassy_net::Ipv4Cidr,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct RefreshSession {
    access_token: heapless::String<1536>,
    refresh_token: heapless::String<BACKEND_REFRESH_TOKEN_MAX_LEN>,
    expires_in: u64,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct ActiveAccessSession {
    access_token: heapless::String<1536>,
    valid_until_ms: u64,
}

impl ActiveAccessSession {
    fn from_refresh_session(session: &RefreshSession, now_ms: u64) -> Self {
        let ttl_ms = session.expires_in.saturating_mul(1000);
        let refresh_margin_ms = 60_000u64.min(ttl_ms.saturating_div(4));
        Self {
            access_token: session.access_token.clone(),
            valid_until_ms: now_ms.saturating_add(ttl_ms.saturating_sub(refresh_margin_ms)),
        }
    }

    fn is_valid_at(&self, now_ms: u64) -> bool {
        now_ms < self.valid_until_ms && !self.access_token.is_empty()
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct MeProfile {
    user_id: heapless::String<64>,
    role: heapless::String<32>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct CollectionFetchSummary {
    trace: TraceContext,
    item_count: usize,
    next_cursor_present: bool,
    page_count: usize,
    body_bytes_total: usize,
    truncated_by_capacity: bool,
    body_preview: Option<heapless::String<INBOX_LOG_PREVIEW_MAX_LEN>>,
    body_preview_truncated: bool,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct CollectionFetchResult {
    summary: CollectionFetchSummary,
    collection: CollectionManifestState,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum CollectionEndpoint {
    Inbox,
    Saved,
    Recommendations,
}

impl CollectionEndpoint {
    const fn kind(self) -> CollectionKind {
        match self {
            Self::Inbox => CollectionKind::Inbox,
            Self::Saved => CollectionKind::Saved,
            Self::Recommendations => CollectionKind::Recommendations,
        }
    }

    const fn path(self) -> &'static str {
        match self {
            Self::Inbox => INBOX_PATH,
            Self::Saved => SAVED_CONTENT_PATH,
            Self::Recommendations => RECOMMENDATIONS_PATH,
        }
    }

    const fn extra_query(self) -> &'static str {
        match self {
            Self::Inbox => "",
            Self::Saved => "&archived=false",
            Self::Recommendations => "",
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct CollectionFetchPage {
    collection: CollectionManifestState,
    next_cursor: Option<heapless::String<COLLECTION_CURSOR_MAX_LEN>>,
    body_preview: Option<heapless::String<INBOX_LOG_PREVIEW_MAX_LEN>>,
    body_preview_truncated: bool,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct CollectionFetchAccumulator {
    trace: TraceContext,
    collection: CollectionManifestState,
    page_count: usize,
    body_bytes_total: usize,
    next_cursor: Option<heapless::String<COLLECTION_CURSOR_MAX_LEN>>,
    truncated_by_capacity: bool,
    body_preview: Option<heapless::String<INBOX_LOG_PREVIEW_MAX_LEN>>,
    body_preview_truncated: bool,
}

impl CollectionFetchAccumulator {
    fn new(trace: TraceContext) -> Self {
        Self {
            trace,
            collection: CollectionManifestState::empty(),
            page_count: 0,
            body_bytes_total: 0,
            next_cursor: None,
            truncated_by_capacity: false,
            body_preview: None,
            body_preview_truncated: false,
        }
    }

    fn absorb_page(
        &mut self,
        endpoint: CollectionEndpoint,
        path: &str,
        body_bytes: usize,
        page_index: usize,
        page: CollectionFetchPage,
    ) {
        let page_item_count = page.collection.len();
        let next_cursor_present = page.next_cursor.is_some();
        let response_headroom = HTTP_RESPONSE_MAX_LEN.saturating_sub(body_bytes);

        self.page_count = self.page_count.saturating_add(1);
        self.body_bytes_total = self.body_bytes_total.saturating_add(body_bytes);

        if self.body_preview.is_none()
            && let Some(preview) = page.body_preview.as_ref()
        {
            self.body_preview = Some(preview.clone());
            self.body_preview_truncated = page.body_preview_truncated;
        }

        if matches!(endpoint, CollectionEndpoint::Recommendations)
            && self.collection.serve_id.is_empty()
            && !page.collection.serve_id.is_empty()
        {
            self.collection.serve_id = page.collection.serve_id;
        }

        let mut accepted_items = 0usize;
        while accepted_items < page.collection.len() {
            if !self
                .collection
                .try_push(page.collection.items[accepted_items])
            {
                self.truncated_by_capacity = true;
                break;
            }
            accepted_items += 1;
        }

        self.next_cursor = page.next_cursor;
        log_collection_fetch_page_metrics(
            self.trace,
            endpoint.kind(),
            path,
            page_index,
            body_bytes,
            page_item_count,
            accepted_items,
            next_cursor_present,
            response_headroom,
            self.collection.len(),
            self.truncated_by_capacity,
        );
    }

    fn should_continue(&self) -> bool {
        self.next_cursor.is_some()
            && !self.truncated_by_capacity
            && self.collection.len() < MANIFEST_ITEM_CAPACITY
            && self.page_count < COLLECTION_FETCH_MAX_PAGES
    }

    fn into_result(self) -> CollectionFetchResult {
        CollectionFetchResult {
            summary: CollectionFetchSummary {
                trace: self.trace,
                item_count: self.collection.len(),
                next_cursor_present: self.next_cursor.is_some() || self.truncated_by_capacity,
                page_count: self.page_count,
                body_bytes_total: self.body_bytes_total,
                truncated_by_capacity: self.truncated_by_capacity,
                body_preview: self.body_preview,
                body_preview_truncated: self.body_preview_truncated,
            },
            collection: self.collection,
        }
    }
}

struct StartupSyncResult<'a> {
    refresh_session: RefreshSession,
    saved_result: Result<CollectionFetchResult, CollectionQueryError>,
    reusable_session: Option<ReusableBackendSession<'a>>,
    tls_session_cache: Option<SerializedClientSession>,
}

struct ReusableBackendSession<'a> {
    session: BackendTlsSession<'a>,
    network_address: embassy_net::Ipv4Cidr,
    last_used_ms: u64,
}

impl ReusableBackendSession<'_> {
    fn is_usable_on(&self, stack: Stack<'static>, now_ms: u64, streaming: bool) -> bool {
        stack.is_link_up()
            && stack
                .config_v4()
                .is_some_and(|config| config.address == self.network_address)
            && is_reusable_session_age_usable(self.last_used_ms, now_ms, streaming)
    }

    fn mark_used(&mut self, now_ms: u64) {
        self.last_used_ms = now_ms;
    }
}

struct ConnectedBackendSession<'a> {
    session: BackendTlsSession<'a>,
    network_address: embassy_net::Ipv4Cidr,
    metrics: RequestMetrics,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum RequestClass {
    AuthRefresh,
    BufferedMetadata,
    StreamingPackage,
}

impl RequestClass {
    const fn label(self) -> &'static str {
        match self {
            Self::AuthRefresh => "auth_refresh",
            Self::BufferedMetadata => "buffered_metadata",
            Self::StreamingPackage => "streaming_package",
        }
    }

    const fn is_streaming(self) -> bool {
        matches!(self, Self::StreamingPackage)
    }

    const fn connect_timeout_secs(self) -> u64 {
        let _ = self;
        CONNECT_TIMEOUT_SECS
    }

    const fn tls_handshake_timeout_secs(self) -> u64 {
        match self {
            Self::AuthRefresh => AUTH_REFRESH_TLS_HANDSHAKE_TIMEOUT_SECS,
            Self::BufferedMetadata => BUFFERED_METADATA_TLS_HANDSHAKE_TIMEOUT_SECS,
            Self::StreamingPackage => STREAMING_PACKAGE_TLS_HANDSHAKE_TIMEOUT_SECS,
        }
    }

    const fn io_timeout_secs(self) -> u64 {
        match self {
            Self::AuthRefresh => AUTH_REFRESH_IO_TIMEOUT_SECS,
            Self::BufferedMetadata => BUFFERED_METADATA_IO_TIMEOUT_SECS,
            Self::StreamingPackage => STREAMING_PACKAGE_IO_TIMEOUT_SECS,
        }
    }

    const fn socket_timeout_secs(self) -> u64 {
        self.io_timeout_secs()
    }

    const fn network_ready_timeout_secs(self) -> u64 {
        match self {
            Self::AuthRefresh => AUTH_REFRESH_NETWORK_READY_TIMEOUT_SECS,
            Self::BufferedMetadata => BUFFERED_METADATA_NETWORK_READY_TIMEOUT_SECS,
            Self::StreamingPackage => STREAMING_PACKAGE_NETWORK_READY_TIMEOUT_SECS,
        }
    }

    fn network_ready_max_timeout_secs(self, reason: &str) -> u64 {
        if matches!(self, Self::StreamingPackage) && reason == "package_retry" {
            PACKAGE_RETRY_NETWORK_READY_MAX_TIMEOUT_SECS
        } else {
            self.network_ready_timeout_secs()
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum BackendEndpointSource {
    Dns,
    Cached,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct ResolvedBackendRemote {
    addr: Ipv4Addr,
    source: BackendEndpointSource,
    cache_age_ms: Option<u32>,
    session_epoch: Option<u32>,
}

#[derive(Clone, Copy)]
struct BackendRequestContext<'a> {
    stack: Stack<'static>,
    tls: TlsReference<'a>,
    ca_chain: &'a Certificate<'static>,
    tcp_client: &'a BackendTcpClient<'a>,
    tcp_state: &'a BackendTcpClientState,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum BackendCommand {
    PrepareContent(PrepareContentRequest),
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct HttpResponse<'a> {
    status: u16,
    body: &'a str,
    connection_reusable: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct HttpRequest<'a> {
    trace: TraceContext,
    class: RequestClass,
    method: &'a str,
    path: &'a str,
    content_type: Option<&'a str>,
    bearer_token: Option<&'a str>,
    body: &'a [u8],
    connection_close: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct HttpResponseMetadata {
    status: u16,
    body_start: usize,
    content_length: Option<usize>,
    chunked: bool,
    connection_close: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct StreamingHttpResponse {
    status: u16,
    connection_reusable: bool,
    prepare_progress: Option<PrepareProgressState>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct PrepareProgressState {
    download_steps: u16,
    total_steps: u16,
}

#[derive(Debug, Clone, Copy)]
struct PrepareProgressReporter {
    content_id: InlineText<{ domain::content::CONTENT_ID_MAX_BYTES }>,
    download_steps: u16,
    total_steps: u16,
    last_progress: PrepareContentProgress,
}

impl PrepareProgressReporter {
    fn new(content_id: InlineText<{ domain::content::CONTENT_ID_MAX_BYTES }>) -> Self {
        Self {
            content_id,
            download_steps: PREPARE_PROGRESS_MIN_DOWNLOAD_STEPS,
            total_steps: PREPARE_PROGRESS_MIN_DOWNLOAD_STEPS + PREPARE_PROGRESS_FIXED_STEPS,
            last_progress: PrepareContentProgress::connecting(),
        }
    }

    fn begin_download(&mut self, total_bytes: Option<usize>) {
        if let Some(total_bytes) = total_bytes {
            self.download_steps = prepare_download_step_count(total_bytes);
            self.total_steps = self.download_steps + PREPARE_PROGRESS_FIXED_STEPS;
        }

        self.publish(PrepareContentProgress {
            phase: PrepareContentPhase::Downloading,
            completed_steps: 1,
            total_steps: self.total_steps,
        });
    }

    fn publish_download_progress(&mut self, received_bytes: usize, total_bytes: Option<usize>) {
        let completed_download_steps = match total_bytes {
            Some(total_bytes) if total_bytes > 0 => {
                let scaled = ((received_bytes as u64 * self.download_steps as u64)
                    / total_bytes as u64) as u16;
                if received_bytes > 0 { scaled.max(1) } else { 0 }
            }
            _ => ((received_bytes / PREPARE_PROGRESS_DOWNLOAD_STEP_BYTES) as u16)
                .min(self.download_steps)
                .max(u16::from(received_bytes > 0)),
        };

        self.publish(PrepareContentProgress {
            phase: PrepareContentPhase::Downloading,
            completed_steps: 1 + completed_download_steps.min(self.download_steps),
            total_steps: self.total_steps,
        });
    }

    fn publish(&mut self, progress: PrepareContentProgress) {
        if progress == self.last_progress {
            return;
        }

        self.last_progress = progress;
        publish_event(
            Event::ContentPrepareProgress {
                content_id: self.content_id,
                progress,
            },
            now_ms(),
        );
    }

    const fn state(&self) -> PrepareProgressState {
        PrepareProgressState {
            download_steps: self.download_steps,
            total_steps: self.total_steps,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct RequestMetrics {
    trace: TraceContext,
    class: RequestClass,
    started_ms: u64,
    dns_ms: u64,
    connect_ms: u64,
    tls_ms: u64,
    first_byte_ms: Option<u64>,
    total_ms: u64,
    reused_session: bool,
    tls_resume_offered: bool,
    streaming: bool,
    header_bytes: usize,
    body_bytes: usize,
    response_bytes: usize,
    response_buffer_capacity: usize,
    response_buffer_headroom: usize,
    content_length: usize,
    content_length_known: bool,
    stream_header_capacity: usize,
    stream_header_headroom: usize,
}

impl RequestMetrics {
    fn new(trace: TraceContext, reused_session: bool, class: RequestClass) -> Self {
        Self {
            trace,
            class,
            started_ms: now_ms(),
            dns_ms: 0,
            connect_ms: 0,
            tls_ms: 0,
            first_byte_ms: None,
            total_ms: 0,
            reused_session,
            tls_resume_offered: false,
            streaming: class.is_streaming(),
            header_bytes: 0,
            body_bytes: 0,
            response_bytes: 0,
            response_buffer_capacity: 0,
            response_buffer_headroom: 0,
            content_length: 0,
            content_length_known: false,
            stream_header_capacity: 0,
            stream_header_headroom: 0,
        }
    }

    fn elapsed_ms(&self) -> u64 {
        now_ms().saturating_sub(self.started_ms)
    }

    fn mark_first_byte(&mut self) {
        if self.first_byte_ms.is_none() {
            self.first_byte_ms = Some(self.elapsed_ms());
        }
    }

    fn finish(&mut self) {
        self.total_ms = self.elapsed_ms();
    }
}

fn next_request_trace(sync_id: u32) -> TraceContext {
    TraceContext {
        sync_id,
        req_id: next_request_id(),
    }
}

pub(crate) fn log_static_inventory() {
    crate::memtrace!(
        "static_inventory",
        "component" = "backend",
        "at_ms" = now_ms(),
        "driver_loop_future_storage" = "external_pinned_box",
        "backend_tcp_state_bytes" = size_of::<BackendTcpClientState>(),
        "http_request_bytes" = size_of::<HttpRequest<'static>>(),
        "http_response_metadata_bytes" = size_of::<HttpResponseMetadata>(),
        "request_metrics_bytes" = size_of::<RequestMetrics>(),
        "reusable_session_bytes" = size_of::<ReusableBackendSession<'static>>(),
        "active_access_session_bytes" = size_of::<ActiveAccessSession>(),
        "collection_fetch_summary_bytes" = size_of::<CollectionFetchSummary>(),
        "http_response_buffer_len" = HTTP_RESPONSE_MAX_LEN,
        "http_response_buffer_storage" = "external_preferred_box",
        "http_stream_header_max_len" = HTTP_STREAM_HEADER_MAX_LEN,
        "http_stream_header_storage" = "external_preferred_box",
        "package_download_chunk_len" = PACKAGE_DOWNLOAD_CHUNK_LEN,
        "package_storage_handoff_chunk_len" = PACKAGE_STORAGE_HANDOFF_CHUNK_LEN,
        "package_download_chunk_source" = transfer_tuning::PACKAGE_TRANSFER_SOURCE,
        "package_download_chunk_storage" = "external_preferred_box",
        "collection_page_limit" = COLLECTION_PAGE_LIMIT,
        "collection_cursor_max_len" = COLLECTION_CURSOR_MAX_LEN,
        "collection_page_path_max_len" = COLLECTION_PAGE_PATH_MAX_LEN,
        "stream_progress_log_interval_bytes" = STREAM_PROGRESS_LOG_INTERVAL_BYTES,
        "tcp_rx_buffer_bytes" = BACKEND_TCP_RX_BUFFER_LEN,
        "tcp_tx_buffer_bytes" = BACKEND_TCP_TX_BUFFER_LEN,
        "auth_refresh_connect_timeout_secs" = RequestClass::AuthRefresh.connect_timeout_secs(),
        "auth_refresh_tls_handshake_timeout_secs" =
            RequestClass::AuthRefresh.tls_handshake_timeout_secs(),
        "auth_refresh_io_timeout_secs" = RequestClass::AuthRefresh.io_timeout_secs(),
        "auth_refresh_network_ready_timeout_secs" =
            RequestClass::AuthRefresh.network_ready_timeout_secs(),
        "buffered_metadata_connect_timeout_secs" =
            RequestClass::BufferedMetadata.connect_timeout_secs(),
        "buffered_metadata_tls_handshake_timeout_secs" =
            RequestClass::BufferedMetadata.tls_handshake_timeout_secs(),
        "buffered_metadata_io_timeout_secs" = RequestClass::BufferedMetadata.io_timeout_secs(),
        "buffered_metadata_network_ready_timeout_secs" =
            RequestClass::BufferedMetadata.network_ready_timeout_secs(),
        "streaming_package_connect_timeout_secs" =
            RequestClass::StreamingPackage.connect_timeout_secs(),
        "streaming_package_tls_handshake_timeout_secs" =
            RequestClass::StreamingPackage.tls_handshake_timeout_secs(),
        "streaming_package_io_timeout_secs" = RequestClass::StreamingPackage.io_timeout_secs(),
        "streaming_package_network_ready_timeout_secs" =
            RequestClass::StreamingPackage.network_ready_timeout_secs(),
        "streaming_package_network_ready_max_timeout_secs" =
            PACKAGE_RETRY_NETWORK_READY_MAX_TIMEOUT_SECS,
        "backend_cmd_queue_capacity" = BACKEND_CMD_QUEUE_CAPACITY,
    );
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum BackendError {
    Alloc,
    Dns,
    Connect,
    Tls,
    Io,
    InvalidResponse,
    InvalidUtf8,
    ResponseTooLarge,
    MissingField,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum RefreshError {
    Rejected(u16),
    Other(BackendError),
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum IdentityError {
    Rejected(u16),
    Other(BackendError),
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum CollectionQueryError {
    Rejected(u16),
    Other(BackendError),
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum PackagePrepareError {
    PendingRemote,
    Rejected(u16),
    Other(BackendError),
}

pub fn initial_pairing_state(
    stored_credential: Option<BackendCredential>,
) -> domain::device::PairingState {
    if select_startup_credential(compile_time_refresh_token(), stored_credential).is_some() {
        domain::device::PairingState::Paired
    } else {
        domain::device::PairingState::Unpaired
    }
}

pub fn install(
    spawner: Spawner,
    stack: Option<Stack<'static>>,
    stored_credential: Option<BackendCredential>,
    rng: RNG<'static>,
    adc1: ADC1<'static>,
) {
    let Some(stack) = stack else {
        info!("backend disabled: internet stack unavailable");
        return;
    };

    let startup = match select_startup_credential(compile_time_refresh_token(), stored_credential) {
        Some(startup) => startup,
        None => {
            info!(
                "backend disabled: no refresh token configured via MOTIF_BACKEND_REFRESH_TOKEN or storage"
            );
            return;
        }
    };

    info!(
        "backend configured: source={} base_url={}",
        startup.source.label(),
        BACKEND_BASE_URL
    );

    if spawner
        .spawn(backend_task(stack, startup, rng, adc1))
        .is_err()
    {
        warn!("backend failed to spawn auth task");
    }
}

pub async fn request_prepare_content(request: PrepareContentRequest) {
    BACKEND_CMD_CH
        .send(BackendCommand::PrepareContent(request))
        .await;
}

#[embassy_executor::task]
async fn backend_task(
    stack: Stack<'static>,
    startup: StartupCredential,
    rng: RNG<'static>,
    adc1: ADC1<'static>,
) {
    let _trng_source = TrngSource::new(rng, adc1);
    let mut trng = match Trng::try_new() {
        Ok(trng) => trng,
        Err(_) => {
            log_status(SyncStatus::TransportFailed);
            warn!("backend tls init failed: TRNG unavailable");
            return;
        }
    };

    let mut tls = match Tls::new(&mut trng) {
        Ok(tls) => tls,
        Err(err) => {
            log_status(SyncStatus::TransportFailed);
            warn!("backend tls init failed: {:?}", err);
            return;
        }
    };
    tls.set_debug(MBEDTLS_DEBUG_LEVEL);
    info!(
        "backend tls config debug_level={} in_content_len={} out_content_len={}",
        MBEDTLS_DEBUG_LEVEL,
        mbedtls_rs::sys::MBEDTLS_SSL_IN_CONTENT_LEN,
        mbedtls_rs::sys::MBEDTLS_SSL_OUT_CONTENT_LEN,
    );

    let ca_chain = match backend_ca_chain() {
        Ok(ca_chain) => ca_chain,
        Err(err) => {
            log_status(SyncStatus::TransportFailed);
            warn!("backend ca chain init failed: {:?}", err);
            return;
        }
    };

    let mut current = startup;
    let tcp_state = Box::new(BackendTcpClientState::new());
    let event_loop = crate::memory_policy::try_external_pinned_box(async move {
        loop {
            log_status(SyncStatus::WaitingForNetwork);
            let network = wait_for_network(stack).await;
            info!("backend network ready ip={:?}", network.address);
            log_heap("backend network ready");
            let mut tcp_client = BackendTcpClient::new(stack, tcp_state.as_ref());
            tcp_client.set_timeout(Some(Duration::from_secs(
                RequestClass::StreamingPackage.socket_timeout_secs(),
            )));

            log_status(SyncStatus::RefreshingSession);
            crate::internet::set_probe_suspended(true);
            let startup_sync_id = next_sync_id();
            crate::memtrace!(
                "backend_sync",
                "component" = "backend",
                "at_ms" = now_ms(),
                "action" = "startup_begin",
                "sync_id" = startup_sync_id,
                "req_id" = 0,
            );
            let startup_sync = perform_startup_refresh_and_saved_sync(
                stack,
                tls.reference(),
                &ca_chain,
                &tcp_client,
                &current.credential,
                startup_sync_id,
            )
            .await;
            crate::internet::set_probe_suspended(false);

            let startup_sync = match startup_sync {
                Ok(result) => result,
                Err(RefreshError::Rejected(status)) => {
                    crate::memtrace!(
                        "backend_sync",
                        "component" = "backend",
                        "at_ms" = now_ms(),
                        "action" = "startup_auth_failed",
                        "sync_id" = startup_sync_id,
                        "req_id" = 0,
                        "status" = status,
                    );
                    log_status(SyncStatus::AuthFailed);
                    info!(
                        "backend refresh rejected status={} source={}",
                        status,
                        current.source.label(),
                    );
                    return;
                }
                Err(RefreshError::Other(err)) => {
                    crate::memtrace!(
                        "backend_sync",
                        "component" = "backend",
                        "at_ms" = now_ms(),
                        "action" = "startup_transport_failed",
                        "sync_id" = startup_sync_id,
                        "req_id" = 0,
                        "error" = backend_error_label(err),
                    );
                    log_status(SyncStatus::TransportFailed);
                    warn!("backend refresh failed: {:?}", err);
                    Timer::after(Duration::from_millis(RETRY_BACKOFF_MS)).await;
                    continue;
                }
            };
            crate::memtrace!(
                "backend_sync",
                "component" = "backend",
                "at_ms" = now_ms(),
                "action" = "startup_ok",
                "sync_id" = startup_sync_id,
                "req_id" = 0,
                "expires_in" = startup_sync.refresh_session.expires_in,
            );

            info!(
                "backend refresh ok expires_in={}s",
                startup_sync.refresh_session.expires_in
            );

            match BackendCredential::from_refresh_token(
                startup_sync.refresh_session.refresh_token.as_str(),
            ) {
                Ok(credential) => {
                    current = StartupCredential {
                        credential,
                        source: CredentialSource::Stored,
                    };
                    persist_backend_credential(credential).await;
                    info!("backend credential persisted");
                }
                Err(err) => {
                    info!("backend credential persistence skipped: {:?}", err);
                }
            }
            let mut access_session = Some(ActiveAccessSession::from_refresh_session(
                &startup_sync.refresh_session,
                now_ms(),
            ));
            let initial_reusable_session = startup_sync.reusable_session;
            let initial_tls_session_cache = startup_sync.tls_session_cache;

            sync_one_collection(
                CollectionKind::Saved,
                startup_sync.saved_result,
                "backend saved",
            )
            .await;
            log_status(SyncStatus::Ready);
            log_heap("backend ready");
            run_backend_command_loop(
                stack,
                tls.reference(),
                &ca_chain,
                &tcp_client,
                tcp_state.as_ref(),
                &mut current,
                &mut access_session,
                initial_reusable_session,
                initial_tls_session_cache,
            )
            .await;
        }
    });
    let event_loop = match event_loop {
        Ok(event_loop) => event_loop,
        Err(_) => {
            log_status(SyncStatus::TransportFailed);
            warn!("backend event loop alloc failed");
            return;
        }
    };
    event_loop.await;
}

#[allow(clippy::too_many_arguments)]
async fn run_backend_command_loop<'a>(
    stack: Stack<'static>,
    tls: TlsReference<'a>,
    ca_chain: &'a Certificate<'static>,
    tcp_client: &'a BackendTcpClient<'a>,
    tcp_state: &'a BackendTcpClientState,
    current: &mut StartupCredential,
    access_session: &mut Option<ActiveAccessSession>,
    initial_reusable_session: Option<ReusableBackendSession<'a>>,
    initial_tls_session_cache: Option<SerializedClientSession>,
) {
    let mut reusable_session: Option<ReusableBackendSession<'a>> = initial_reusable_session;
    let mut tls_session_cache = initial_tls_session_cache;
    let context = BackendRequestContext {
        stack,
        tls,
        ca_chain,
        tcp_client,
        tcp_state,
    };

    loop {
        let command = if reusable_session.is_some() {
            match select(
                BACKEND_CMD_CH.receive(),
                Timer::after(Duration::from_millis(
                    reusable_session_idle_reap_timeout_ms(),
                )),
            )
            .await
            {
                Either::First(command) => Some(command),
                Either::Second(()) => {
                    let idle_age_ms = reusable_session
                        .as_ref()
                        .map(|session| now_ms().saturating_sub(session.last_used_ms))
                        .unwrap_or_default();
                    info!(
                        "backend reusable session idle reap age_ms={} idle_limit_ms={}",
                        idle_age_ms,
                        reusable_session_idle_reap_timeout_ms(),
                    );
                    close_reusable_session(&mut reusable_session, "idle reap").await;
                    None
                }
            }
        } else {
            Some(BACKEND_CMD_CH.receive().await)
        };

        let Some(command) = command else {
            continue;
        };

        match command {
            BackendCommand::PrepareContent(request) => {
                handle_prepare_content_request(
                    context,
                    current,
                    access_session,
                    &mut reusable_session,
                    &mut tls_session_cache,
                    request,
                )
                .await;
                log_status(SyncStatus::Ready);
            }
        }
    }
}

async fn wait_for_network(stack: Stack<'static>) -> NetworkReady {
    loop {
        if stack.is_link_up()
            && crate::internet::backend_path_ready()
            && let Some(config) = stack.config_v4()
        {
            return NetworkReady {
                address: config.address,
            };
        }

        Timer::after(Duration::from_millis(NETWORK_POLL_MS)).await;
    }
}

async fn perform_health_check(
    stack: Stack<'static>,
    tls: TlsReference<'_>,
    ca_chain: &Certificate<'static>,
    tcp_state: &BackendTcpClientState,
) -> Result<(), BackendError> {
    let mut response_buffer = allocate_standard_response_buffer(HEALTH_PATH)?;
    let mut tls_session_cache = None;
    let mut last_error = None;
    let mut attempt = 0;
    while attempt < 3 {
        let response = send_https_request(
            stack,
            tls,
            ca_chain,
            tcp_state,
            &mut tls_session_cache,
            HttpRequest {
                trace: TraceContext::none(),
                class: RequestClass::BufferedMetadata,
                method: "GET",
                path: HEALTH_PATH,
                content_type: None,
                bearer_token: None,
                body: b"",
                connection_close: true,
            },
            response_buffer.as_mut_slice(),
        )
        .await;

        match response {
            Ok(response) => {
                if response.status != 200 {
                    info!(
                        "backend health unexpected status={} path={}",
                        response.status, HEALTH_PATH
                    );
                    return Err(BackendError::InvalidResponse);
                }

                return match extract_json_string(response.body, "\"status\"") {
                    Some("ok") => Ok(()),
                    _ => Err(BackendError::MissingField),
                };
            }
            Err(err) => {
                last_error = Some(err);
                attempt += 1;
                if attempt < 3 {
                    warn!("backend health retry attempt={} err={:?}", attempt, err);
                    Timer::after(Duration::from_millis(750)).await;
                }
            }
        }
    }

    Err(last_error.unwrap_or(BackendError::Io))
}

async fn perform_refresh(
    stack: Stack<'static>,
    tls: TlsReference<'_>,
    ca_chain: &Certificate<'static>,
    tcp_state: &BackendTcpClientState,
    tls_session_cache: &mut Option<SerializedClientSession>,
    credential: &BackendCredential,
    sync_id: u32,
) -> Result<RefreshSession, RefreshError> {
    let refresh_token = credential
        .refresh_token()
        .map_err(|_| RefreshError::Other(BackendError::InvalidResponse))?;
    info!(
        "backend refresh building request token_len={}",
        refresh_token.len()
    );

    let mut response_buffer =
        allocate_standard_response_buffer(REFRESH_PATH).map_err(RefreshError::Other)?;
    let body = Box::new(
        build_refresh_body(refresh_token)
            .map_err(|_| RefreshError::Other(BackendError::InvalidResponse))?,
    );
    info!("backend refresh request ready body_len={}", body.len());
    let response = send_https_request(
        stack,
        tls,
        ca_chain,
        tcp_state,
        tls_session_cache,
        HttpRequest {
            trace: next_request_trace(sync_id),
            class: RequestClass::AuthRefresh,
            method: "POST",
            path: REFRESH_PATH,
            content_type: Some("application/json"),
            bearer_token: None,
            body: body.as_bytes(),
            connection_close: true,
        },
        response_buffer.as_mut_slice(),
    )
    .await
    .map_err(RefreshError::Other)?;

    if (400..500).contains(&response.status) {
        info!(
            "backend refresh rejected status={} path={}",
            response.status, REFRESH_PATH
        );
        return Err(RefreshError::Rejected(response.status));
    }
    if response.status != 200 {
        info!(
            "backend refresh unexpected status={} path={}",
            response.status, REFRESH_PATH
        );
        return Err(RefreshError::Other(BackendError::InvalidResponse));
    }

    parse_refresh_session(response.body)
}

async fn perform_identity_check(
    stack: Stack<'static>,
    tls: TlsReference<'_>,
    ca_chain: &Certificate<'static>,
    tcp_state: &BackendTcpClientState,
    tls_session_cache: &mut Option<SerializedClientSession>,
    access_token: &str,
    sync_id: u32,
) -> Result<MeProfile, IdentityError> {
    let mut response_buffer =
        allocate_standard_response_buffer(ME_PATH).map_err(IdentityError::Other)?;
    let response = send_https_request(
        stack,
        tls,
        ca_chain,
        tcp_state,
        tls_session_cache,
        HttpRequest {
            trace: next_request_trace(sync_id),
            class: RequestClass::BufferedMetadata,
            method: "GET",
            path: ME_PATH,
            content_type: Some("application/json"),
            bearer_token: Some(access_token),
            body: b"",
            connection_close: true,
        },
        response_buffer.as_mut_slice(),
    )
    .await
    .map_err(IdentityError::Other)?;

    if (400..500).contains(&response.status) {
        info!(
            "backend identity rejected status={} path={}",
            response.status, ME_PATH
        );
        return Err(IdentityError::Rejected(response.status));
    }
    if response.status != 200 {
        info!(
            "backend identity unexpected status={} path={}",
            response.status, ME_PATH
        );
        return Err(IdentityError::Other(BackendError::InvalidResponse));
    }

    parse_me_profile(response.body)
}

async fn sync_collection_manifests(
    stack: Stack<'static>,
    tls: TlsReference<'_>,
    ca_chain: &Certificate<'static>,
    tcp_state: &BackendTcpClientState,
    tls_session_cache: &mut Option<SerializedClientSession>,
    access_token: &str,
) {
    info!("backend startup sync mode=saved-only");
    sync_one_collection(
        CollectionKind::Saved,
        perform_saved_content_fetch(
            stack,
            tls,
            ca_chain,
            tcp_state,
            tls_session_cache,
            access_token,
            0,
        )
        .await,
        "backend saved",
    )
    .await;
}

async fn perform_startup_refresh_and_saved_sync<'a>(
    stack: Stack<'static>,
    tls: TlsReference<'a>,
    ca_chain: &Certificate<'static>,
    tcp_client: &'a BackendTcpClient<'a>,
    credential: &BackendCredential,
    sync_id: u32,
) -> Result<StartupSyncResult<'a>, RefreshError> {
    let mut attempt = 0usize;
    let mut tls_session_cache = None;

    loop {
        let result = perform_startup_refresh_and_saved_sync_once(
            stack,
            tls,
            ca_chain,
            tcp_client,
            &mut tls_session_cache,
            credential,
            sync_id,
        )
        .await;
        match result {
            Err(RefreshError::Other(err))
                if is_transient_transport_error(err) && attempt + 1 < TRANSPORT_RETRY_ATTEMPTS =>
            {
                attempt += 1;
                warn!("backend startup retry attempt={} err={:?}", attempt, err);
                // Startup sync normally suspends the background probe task to
                // keep the shared socket set focused on refresh + manifest
                // fetches. If the first attempt invalidates backend-path
                // readiness, briefly re-enable probing so the retry waits for a
                // fresh, real connectivity check instead of blindly burning its
                // final attempt on a still-bad path.
                crate::internet::set_probe_suspended(false);
                let recovery = wait_for_backend_request_path_ready(
                    stack,
                    TraceContext { sync_id, req_id: 0 },
                    REFRESH_PATH,
                    RequestClass::AuthRefresh,
                    "startup_retry_reprobe",
                )
                .await;
                crate::internet::set_probe_suspended(true);
                recovery.map_err(RefreshError::Other)?;
                Timer::after(Duration::from_millis(TRANSPORT_RETRY_BACKOFF_MS)).await;
            }
            other => return other,
        }
    }
}

async fn perform_startup_refresh_and_saved_sync_once<'a>(
    stack: Stack<'static>,
    tls: TlsReference<'a>,
    ca_chain: &Certificate<'static>,
    tcp_client: &'a BackendTcpClient<'a>,
    tls_session_cache: &mut Option<SerializedClientSession>,
    credential: &BackendCredential,
    sync_id: u32,
) -> Result<StartupSyncResult<'a>, RefreshError> {
    let refresh_token = credential
        .refresh_token()
        .map_err(|_| RefreshError::Other(BackendError::InvalidResponse))?;
    info!(
        "backend refresh building request token_len={}",
        refresh_token.len()
    );

    let refresh_trace = next_request_trace(sync_id);
    let mut refresh_metrics = RequestMetrics::new(refresh_trace, false, RequestClass::AuthRefresh);
    log_request_phase(
        refresh_trace,
        REFRESH_PATH,
        RequestClass::AuthRefresh,
        "open",
        0,
    );
    let resolved_remote = resolve_backend_remote(
        stack,
        refresh_trace,
        REFRESH_PATH,
        RequestClass::AuthRefresh,
        &mut refresh_metrics,
    )
    .await
    .map_err(RefreshError::Other)?;
    let connect_started_ms = now_ms();
    log_request_phase(
        refresh_trace,
        REFRESH_PATH,
        RequestClass::AuthRefresh,
        "dns_ok",
        refresh_metrics.dns_ms,
    );
    let connection = with_timeout(
        Duration::from_secs(RequestClass::AuthRefresh.connect_timeout_secs()),
        tcp_client.connect(SocketAddr::new(
            IpAddr::V4(resolved_remote.addr),
            BACKEND_PORT,
        )),
    )
    .await
    .map_err(|_| {
        crate::internet::invalidate_backend_path("connect_timeout");
        warn!("backend request connect timed out path={}", REFRESH_PATH);
        log_request_heap(REFRESH_PATH, "connect timeout");
        RefreshError::Other(BackendError::Connect)
    })?
    .map_err(|_| {
        crate::internet::invalidate_backend_path("connect_failed");
        warn!("backend request connect failed path={}", REFRESH_PATH);
        log_request_heap(REFRESH_PATH, "connect failed");
        RefreshError::Other(BackendError::Connect)
    })?;
    crate::internet::record_backend_endpoint(resolved_remote.addr, "request_connect_ok");
    refresh_metrics.connect_ms = elapsed_since_ms(connect_started_ms);
    if matches!(resolved_remote.source, BackendEndpointSource::Cached) {
        info!(
            "backend request dns fallback succeeded path={} remote_ip={} cache_age_ms={} session_epoch={} connect_ms={}",
            REFRESH_PATH,
            resolved_remote.addr,
            resolved_remote.cache_age_ms.unwrap_or_default(),
            resolved_remote.session_epoch.unwrap_or_default(),
            refresh_metrics.connect_ms
        );
    }
    log_request_phase(
        refresh_trace,
        REFRESH_PATH,
        RequestClass::AuthRefresh,
        "connect_ok",
        refresh_metrics.elapsed_ms(),
    );
    log_request_heap(REFRESH_PATH, "tls setup start");
    log_request_phase(
        refresh_trace,
        REFRESH_PATH,
        RequestClass::AuthRefresh,
        "tls_setup_start",
        refresh_metrics.elapsed_ms(),
    );
    let mut session = open_tls_session(tls, ca_chain, CompatConnection::new(connection))
        .inspect_err(|_err| {
            warn!("backend request tls setup failed path={}", REFRESH_PATH);
            log_request_heap(REFRESH_PATH, "tls setup failed");
        })
        .map_err(RefreshError::Other)?;
    prepare_tls_session_resume(
        &mut session,
        tls_session_cache,
        REFRESH_PATH,
        false,
        &mut refresh_metrics,
    );
    log_request_heap(REFRESH_PATH, "tls setup ok");
    log_request_phase(
        refresh_trace,
        REFRESH_PATH,
        RequestClass::AuthRefresh,
        "tls_setup_ok",
        refresh_metrics.elapsed_ms(),
    );
    let tls_started_ms = now_ms();
    log_request_heap(REFRESH_PATH, "tls handshake start");
    log_request_phase(
        refresh_trace,
        REFRESH_PATH,
        RequestClass::AuthRefresh,
        "tls_handshake_start",
        refresh_metrics.elapsed_ms(),
    );
    if let Err(err) =
        await_tls_handshake(&mut session, REFRESH_PATH, RequestClass::AuthRefresh).await
    {
        discard_failed_tls_session_resume(tls_session_cache, REFRESH_PATH, false, &refresh_metrics);
        crate::internet::invalidate_backend_path(backend_error_label(err));
        return Err(RefreshError::Other(err));
    }
    refresh_metrics.tls_ms = elapsed_since_ms(tls_started_ms);
    log_request_heap(REFRESH_PATH, "tls handshake ok");
    log_request_phase(
        refresh_trace,
        REFRESH_PATH,
        RequestClass::AuthRefresh,
        "tls_handshake_ok",
        refresh_metrics.elapsed_ms(),
    );
    let verification_flags = session.tls_verification_details();
    if verification_flags != 0 {
        info!(
            "backend request tls verification flags path={} flags=0x{:08x}",
            REFRESH_PATH, verification_flags
        );
    }
    cache_negotiated_tls_session(&session, tls_session_cache, REFRESH_PATH, false);

    let mut response_buffer =
        allocate_standard_response_buffer(REFRESH_PATH).map_err(RefreshError::Other)?;
    let body = Box::new(
        build_refresh_body(refresh_token)
            .map_err(|_| RefreshError::Other(BackendError::InvalidResponse))?,
    );
    info!("backend refresh request ready body_len={}", body.len());
    let refresh_response = send_https_request_over_session_with_metrics(
        &mut session,
        HttpRequest {
            trace: refresh_trace,
            class: RequestClass::AuthRefresh,
            method: "POST",
            path: REFRESH_PATH,
            content_type: Some("application/json"),
            bearer_token: None,
            body: body.as_bytes(),
            connection_close: false,
        },
        response_buffer.as_mut_slice(),
        refresh_metrics,
    )
    .await
    .map_err(RefreshError::Other)?;

    if (400..500).contains(&refresh_response.status) {
        info!(
            "backend refresh rejected status={} path={}",
            refresh_response.status, REFRESH_PATH
        );
        if let Err(err) = session.close().await {
            info!("backend tls close failed: {:?}", err);
        }
        return Err(RefreshError::Rejected(refresh_response.status));
    }
    if refresh_response.status != 200 {
        info!(
            "backend refresh unexpected status={} path={}",
            refresh_response.status, REFRESH_PATH
        );
        if let Err(err) = session.close().await {
            info!("backend tls close failed: {:?}", err);
        }
        return Err(RefreshError::Other(BackendError::InvalidResponse));
    }

    let refresh_session = parse_refresh_session(refresh_response.body)?;
    log_status(SyncStatus::SyncingContent);
    info!("backend startup sync mode=saved-only");

    let mut saved_result = perform_saved_content_fetch_over_session(
        &mut session,
        refresh_session.access_token.as_str(),
        false,
        sync_id,
    )
    .await;

    if let Err(err) = saved_result.as_ref()
        && let CollectionQueryError::Other(err) = *err
        && is_transient_transport_error(err)
    {
        if let Err(close_err) = session.close().await {
            info!("backend tls close failed: {:?}", close_err);
        }
        return Err(RefreshError::Other(err));
    }

    if let Ok(result) = &mut saved_result {
        prefetch_startup_saved_content(
            &mut session,
            refresh_session.access_token.as_str(),
            result,
            sync_id,
        )
        .await;
    }

    let reusable_session = if saved_result.is_ok() {
        if let Some(network_address) = current_network_address(stack) {
            info!(
                "backend startup session retained for steady state network_ip={:?}",
                network_address
            );
            Some(ReusableBackendSession {
                session,
                network_address,
                last_used_ms: now_ms(),
            })
        } else {
            info!("backend startup session closing before steady state reason=missing_network");
            if let Err(err) = session.close().await {
                info!("backend tls close failed reason=startup done err={:?}", err);
            }
            None
        }
    } else {
        warn!("backend startup session closing before steady state reason=sync_failed");
        if let Err(err) = session.close().await {
            info!("backend tls close failed reason=startup done err={:?}", err);
        }
        None
    };

    Ok(StartupSyncResult {
        refresh_session,
        saved_result,
        reusable_session,
        tls_session_cache: tls_session_cache.clone(),
    })
}

async fn prefetch_startup_saved_content<T>(
    session: &mut Session<'_, T>,
    access_token: &str,
    result: &mut CollectionFetchResult,
    sync_id: u32,
) where
    T: AsyncRead07 + AsyncWrite07,
{
    if !STARTUP_SAVED_PREFETCH_ENABLED {
        return;
    }

    let mut prefetched = 0usize;
    let mut index = 0usize;
    while index < result.collection.len() && prefetched < STARTUP_SAVED_PREFETCH_LIMIT {
        let item = result.collection.items[index];
        index += 1;

        if !item.can_prepare() {
            continue;
        }
        let request = PrepareContentRequest::from_manifest(CollectionKind::Saved, item);
        match fetch_and_stage_package_over_session(session, access_token, request, sync_id).await {
            Ok(_snapshot) => {
                mark_prefetched_item_cached(&mut result.collection, &request);
                prefetched += 1;
                info!(
                    "backend startup saved prefetch ok content_id={} count={}",
                    request.content_id.as_str(),
                    prefetched,
                );
            }
            Err(PackagePrepareError::PendingRemote) => {
                info!(
                    "backend startup saved prefetch pending remote content_id={}",
                    request.content_id.as_str(),
                );
            }
            Err(PackagePrepareError::Rejected(status)) => {
                info!(
                    "backend startup saved prefetch rejected status={} content_id={}",
                    status,
                    request.content_id.as_str(),
                );
                break;
            }
            Err(PackagePrepareError::Other(err)) => {
                info!(
                    "backend startup saved prefetch failed content_id={} err={:?}",
                    request.content_id.as_str(),
                    err,
                );
                break;
            }
        }
    }
}

fn mark_prefetched_item_cached(
    collection: &mut CollectionManifestState,
    request: &PrepareContentRequest,
) {
    let _ = collection.update_package_state(&request.remote_item_id, PackageState::Cached);
}

#[allow(clippy::too_many_arguments)]
async fn ensure_access_session<'a>(
    stack: Stack<'static>,
    tls: TlsReference<'a>,
    ca_chain: &Certificate<'static>,
    tcp_state: &'a BackendTcpClientState,
    current: &mut StartupCredential,
    access_session: &mut Option<ActiveAccessSession>,
    reusable_session: &mut Option<ReusableBackendSession<'a>>,
    tls_session_cache: &mut Option<SerializedClientSession>,
    sync_id: u32,
) -> Result<(), RefreshError> {
    let now_ms = now_ms();
    match access_session.as_ref() {
        Some(session) if session.is_valid_at(now_ms) => {
            info!(
                "backend access session reuse valid_for_ms={} token_len={}",
                session.valid_until_ms.saturating_sub(now_ms),
                session.access_token.len(),
            );
            return Ok(());
        }
        Some(session) => {
            info!(
                "backend access session refresh reason=expired expired_by_ms={} token_len={}",
                now_ms.saturating_sub(session.valid_until_ms),
                session.access_token.len(),
            );
        }
        None => {
            info!("backend access session refresh reason=missing");
        }
    }

    close_reusable_session(reusable_session, "refresh").await;
    log_status(SyncStatus::RefreshingSession);
    let mut refresh_attempt = 0usize;
    let refresh_session = loop {
        wait_for_backend_request_path_ready(
            stack,
            TraceContext { sync_id, req_id: 0 },
            REFRESH_PATH,
            RequestClass::AuthRefresh,
            if refresh_attempt == 0 {
                "prepare_refresh"
            } else {
                "prepare_refresh_retry"
            },
        )
        .await
        .map_err(RefreshError::Other)?;

        match perform_refresh(
            stack,
            tls,
            ca_chain,
            tcp_state,
            tls_session_cache,
            &current.credential,
            sync_id,
        )
        .await
        {
            Err(RefreshError::Other(err))
                if is_transient_transport_error(err)
                    && refresh_attempt + 1 < TRANSPORT_RETRY_ATTEMPTS =>
            {
                refresh_attempt += 1;
                info!(
                    "backend access session refresh retry attempt={} err={:?}",
                    refresh_attempt, err
                );
                Timer::after(Duration::from_millis(TRANSPORT_RETRY_BACKOFF_MS)).await;
            }
            other => break other?,
        }
    };
    info!(
        "backend access session refreshed expires_in={}s",
        refresh_session.expires_in
    );

    if let Ok(credential) =
        BackendCredential::from_refresh_token(refresh_session.refresh_token.as_str())
    {
        *current = StartupCredential {
            credential,
            source: CredentialSource::Stored,
        };
        persist_backend_credential(credential).await;
        info!("backend credential persisted");
    }

    *access_session = Some(ActiveAccessSession::from_refresh_session(
        &refresh_session,
        now_ms,
    ));
    Ok(())
}

async fn sync_one_collection(
    kind: CollectionKind,
    result: Result<CollectionFetchResult, CollectionQueryError>,
    label: &str,
) {
    match result {
        Ok(result) => {
            let collection = match content_storage::persist_snapshot_traced(
                result.summary.trace,
                kind,
                result.collection,
            )
            .await
            {
                Ok(snapshot) => snapshot,
                Err(err) => {
                    info!("{} persist failed: {:?}", label, err);
                    result.collection
                }
            };
            publish_event(
                Event::CollectionContentUpdated(kind, Box::new(collection)),
                now_ms(),
            );
            info!(
                "{} ok item_count={} pages={} body_bytes_total={} truncated={} next_cursor={}",
                label,
                result.summary.item_count,
                result.summary.page_count,
                result.summary.body_bytes_total,
                result.summary.truncated_by_capacity,
                if result.summary.next_cursor_present {
                    "present"
                } else {
                    "null"
                }
            );
            if let Some(preview) = result.summary.body_preview {
                if result.summary.body_preview_truncated {
                    info!("{} preview={}...", label, preview);
                } else {
                    info!("{} preview={}", label, preview);
                }
            }
        }
        Err(CollectionQueryError::Rejected(status)) => {
            if is_auth_status(status) {
                log_status(SyncStatus::AuthFailed);
            }
            info!("{} rejected status={}", label, status);
        }
        Err(CollectionQueryError::Other(err)) => {
            info!("{} failed: {:?}", label, err);
        }
    }
}

async fn handle_prepare_content_request<'a>(
    context: BackendRequestContext<'a>,
    current: &mut StartupCredential,
    access_session: &mut Option<ActiveAccessSession>,
    reusable_session: &mut Option<ReusableBackendSession<'a>>,
    tls_session_cache: &mut Option<SerializedClientSession>,
    request: PrepareContentRequest,
) {
    if request.remote_item_id.is_empty() || request.content_id.is_empty() {
        return;
    }
    let operation_sync_id = next_sync_id();
    let operation_trace = TraceContext {
        sync_id: operation_sync_id,
        req_id: 0,
    };
    crate::memtrace!(
        "backend_sync",
        "component" = "backend",
        "at_ms" = now_ms(),
        "action" = "prepare_begin",
        "sync_id" = operation_sync_id,
        "req_id" = 0,
        "collection" = collection_label(request.collection),
        "content_id" = request.content_id.as_str(),
        "remote_item_id" = request.remote_item_id.as_str(),
        "remote_revision" = request.remote_revision,
    );

    if let Err(err) = ensure_access_session(
        context.stack,
        context.tls,
        context.ca_chain,
        context.tcp_state,
        current,
        access_session,
        reusable_session,
        tls_session_cache,
        operation_sync_id,
    )
    .await
    {
        match err {
            RefreshError::Rejected(status) => {
                crate::memtrace!(
                    "backend_sync",
                    "component" = "backend",
                    "at_ms" = now_ms(),
                    "action" = "prepare_auth_failed",
                    "sync_id" = operation_sync_id,
                    "req_id" = 0,
                    "status" = status,
                    "content_id" = request.content_id.as_str(),
                );
                log_status(SyncStatus::AuthFailed);
                warn!(
                    "backend content prepare refresh rejected status={} source={}",
                    status,
                    current.source.label(),
                );
                let _ = publish_package_state(
                    operation_trace,
                    request.collection,
                    request.remote_item_id,
                    PackageState::Failed,
                )
                .await;
            }
            RefreshError::Other(err) => {
                crate::memtrace!(
                    "backend_sync",
                    "component" = "backend",
                    "at_ms" = now_ms(),
                    "action" = "prepare_refresh_failed",
                    "sync_id" = operation_sync_id,
                    "req_id" = 0,
                    "error" = backend_error_label(err),
                    "content_id" = request.content_id.as_str(),
                );
                *access_session = None;
                log_status(SyncStatus::TransportFailed);
                warn!("backend content prepare refresh failed: {:?}", err);
                let _ = publish_package_state(
                    operation_trace,
                    request.collection,
                    request.remote_item_id,
                    prepare_error_package_state(err),
                )
                .await;
            }
        }
        return;
    }

    log_status(SyncStatus::SyncingContent);
    let access_token = access_session
        .as_ref()
        .map(|session| session.access_token.as_str())
        .unwrap_or("");

    match fetch_and_stage_package(
        context.stack,
        context.tls,
        context.ca_chain,
        context.tcp_client,
        access_token,
        reusable_session,
        tls_session_cache,
        request,
        operation_sync_id,
    )
    .await
    {
        Ok(result) => {
            let snapshot = *result.snapshot;
            publish_event(
                Event::CollectionContentUpdated(request.collection, Box::new(snapshot)),
                now_ms(),
            );
            crate::memtrace!(
                "backend_sync",
                "component" = "backend",
                "at_ms" = now_ms(),
                "action" = "prepare_cached",
                "sync_id" = operation_sync_id,
                "req_id" = 0,
                "collection" = collection_label(request.collection),
                "content_id" = request.content_id.as_str(),
                "remote_item_id" = request.remote_item_id.as_str(),
            );
            info!(
                "backend content cached collection={:?} content_id={}",
                request.collection,
                request.content_id.as_str(),
            );
            match result.opened {
                Ok(opened) => {
                    let total_units = opened.total_units;
                    let paragraph_count = opened.paragraphs.len();
                    let window_unit_count = opened.window.unit_count;
                    publish_event(
                        Event::ReaderContentOpened {
                            collection: request.collection,
                            content_id: request.content_id,
                            title: opened.title,
                            total_units,
                            paragraphs: opened.paragraphs,
                            window: opened.window,
                        },
                        now_ms(),
                    );
                    info!(
                        "backend content opened after prepare collection={:?} content_id={} total_units={} paragraph_count={} window_units={}",
                        request.collection,
                        request.content_id.as_str(),
                        total_units,
                        paragraph_count,
                        window_unit_count,
                    );
                    crate::memtrace!(
                        "backend_sync",
                        "component" = "backend",
                        "at_ms" = now_ms(),
                        "action" = "prepare_opened",
                        "sync_id" = operation_sync_id,
                        "req_id" = 0,
                        "collection" = collection_label(request.collection),
                        "content_id" = request.content_id.as_str(),
                        "remote_item_id" = request.remote_item_id.as_str(),
                        "total_units" = total_units,
                        "paragraph_count" = paragraph_count,
                        "window_units" = window_unit_count,
                    );
                }
                Err(err) => {
                    let _ = publish_package_state(
                        operation_trace,
                        request.collection,
                        request.remote_item_id,
                        PackageState::Failed,
                    )
                    .await;
                    warn!(
                        "backend content open after prepare failed collection={:?} content_id={} err={:?}",
                        request.collection,
                        request.content_id.as_str(),
                        err,
                    );
                }
            }
        }
        Err(PackagePrepareError::PendingRemote) => {
            crate::memtrace!(
                "backend_sync",
                "component" = "backend",
                "at_ms" = now_ms(),
                "action" = "prepare_pending_remote",
                "sync_id" = operation_sync_id,
                "req_id" = 0,
                "content_id" = request.content_id.as_str(),
            );
            let _ = publish_package_state(
                operation_trace,
                request.collection,
                request.remote_item_id,
                PackageState::PendingRemote,
            )
            .await;
            info!(
                "backend content pending remote collection={:?} content_id={}",
                request.collection,
                request.content_id.as_str(),
            );
        }
        Err(PackagePrepareError::Rejected(status)) => {
            crate::memtrace!(
                "backend_sync",
                "component" = "backend",
                "at_ms" = now_ms(),
                "action" = "prepare_rejected",
                "sync_id" = operation_sync_id,
                "req_id" = 0,
                "status" = status,
                "content_id" = request.content_id.as_str(),
            );
            if is_auth_status(status) {
                invalidate_access_state(access_session, reusable_session).await;
                log_status(SyncStatus::AuthFailed);
            }
            let _ = publish_package_state(
                operation_trace,
                request.collection,
                request.remote_item_id,
                PackageState::Failed,
            )
            .await;
            warn!("backend content fetch rejected status={}", status);
        }
        Err(PackagePrepareError::Other(err)) => {
            crate::memtrace!(
                "backend_sync",
                "component" = "backend",
                "at_ms" = now_ms(),
                "action" = "prepare_failed",
                "sync_id" = operation_sync_id,
                "req_id" = 0,
                "error" = backend_error_label(err),
                "content_id" = request.content_id.as_str(),
            );
            *access_session = None;
            let _ = publish_package_state(
                operation_trace,
                request.collection,
                request.remote_item_id,
                prepare_error_package_state(err),
            )
            .await;
            warn!("backend content fetch failed: {:?}", err);
        }
    }
}

async fn send_https_request<'a>(
    stack: Stack<'static>,
    tls: TlsReference<'a>,
    ca_chain: &Certificate<'static>,
    tcp_state: &'a BackendTcpClientState,
    tls_session_cache: &mut Option<SerializedClientSession>,
    request: HttpRequest<'_>,
    response_buffer: &'a mut [u8],
) -> Result<HttpResponse<'a>, BackendError> {
    // The second attempt only happens after the first attempt returned an error,
    // so no successful response still borrows the shared buffer.
    let response_buffer_ptr: *mut [u8] = response_buffer;
    let first_attempt = unsafe {
        send_https_request_once(
            stack,
            tls,
            ca_chain,
            tcp_state,
            tls_session_cache,
            request,
            &mut *response_buffer_ptr,
        )
        .await
    };
    match first_attempt {
        Err(err) if is_transient_transport_error(err) && TRANSPORT_RETRY_ATTEMPTS > 1 => {
            warn!(
                "backend request retry path={} attempt=1 err={:?}",
                request.path, err
            );
            Timer::after(Duration::from_millis(TRANSPORT_RETRY_BACKOFF_MS)).await;
            unsafe {
                send_https_request_once(
                    stack,
                    tls,
                    ca_chain,
                    tcp_state,
                    tls_session_cache,
                    request,
                    &mut *response_buffer_ptr,
                )
                .await
            }
        }
        other => other,
    }
}

async fn send_https_request_once<'a>(
    stack: Stack<'static>,
    tls: TlsReference<'a>,
    ca_chain: &Certificate<'static>,
    tcp_state: &'a BackendTcpClientState,
    tls_session_cache: &mut Option<SerializedClientSession>,
    request: HttpRequest<'_>,
    response_buffer: &'a mut [u8],
) -> Result<HttpResponse<'a>, BackendError> {
    let mut tcp_client = TcpClient::new(stack, tcp_state);
    tcp_client.set_timeout(Some(Duration::from_secs(
        request.class.socket_timeout_secs(),
    )));
    let ConnectedBackendSession {
        mut session,
        metrics,
        ..
    } = open_backend_session(
        stack,
        tls,
        ca_chain,
        &tcp_client,
        tls_session_cache,
        request.trace,
        request.path,
        request.class,
    )
    .await?;
    let response = send_https_request_over_session_with_metrics(
        &mut session,
        request,
        response_buffer,
        metrics,
    )
    .await;
    close_backend_tls_session(&mut session, "request").await;

    response
}

#[allow(clippy::too_many_arguments)]
async fn send_https_request_reusing_session<'a, 'b>(
    stack: Stack<'static>,
    tls: TlsReference<'a>,
    ca_chain: &Certificate<'static>,
    tcp_client: &'a BackendTcpClient<'a>,
    reusable_session: &mut Option<ReusableBackendSession<'a>>,
    tls_session_cache: &mut Option<SerializedClientSession>,
    request: HttpRequest<'_>,
    response_buffer: &'b mut [u8],
) -> Result<HttpResponse<'b>, BackendError> {
    let response_buffer_ptr: *mut [u8] = response_buffer;
    let first_attempt = unsafe {
        send_https_request_reusing_session_once(
            stack,
            tls,
            ca_chain,
            tcp_client,
            reusable_session,
            tls_session_cache,
            request,
            &mut *response_buffer_ptr,
        )
        .await
    };
    match first_attempt {
        Err(err) if is_transient_transport_error(err) && TRANSPORT_RETRY_ATTEMPTS > 1 => {
            close_reusable_session(reusable_session, "retry").await;
            warn!(
                "backend request retry path={} attempt=1 err={:?}",
                request.path, err
            );
            Timer::after(Duration::from_millis(TRANSPORT_RETRY_BACKOFF_MS)).await;
            unsafe {
                send_https_request_reusing_session_once(
                    stack,
                    tls,
                    ca_chain,
                    tcp_client,
                    reusable_session,
                    tls_session_cache,
                    request,
                    &mut *response_buffer_ptr,
                )
                .await
            }
        }
        other => other,
    }
}

#[allow(clippy::too_many_arguments)]
async fn send_https_request_reusing_session_once<'a, 'b>(
    stack: Stack<'static>,
    tls: TlsReference<'a>,
    ca_chain: &Certificate<'static>,
    tcp_client: &'a BackendTcpClient<'a>,
    reusable_session: &mut Option<ReusableBackendSession<'a>>,
    tls_session_cache: &mut Option<SerializedClientSession>,
    request: HttpRequest<'_>,
    response_buffer: &'b mut [u8],
) -> Result<HttpResponse<'b>, BackendError> {
    let metrics = ensure_reusable_session(
        stack,
        tls,
        ca_chain,
        tcp_client,
        reusable_session,
        tls_session_cache,
        request.trace,
        request.path,
        request.class,
    )
    .await?;
    let response = match reusable_session {
        Some(reusable) => {
            send_https_request_over_session_with_metrics(
                &mut reusable.session,
                request,
                response_buffer,
                metrics,
            )
            .await
        }
        None => unreachable!(),
    };

    update_reusable_session_after_buffered_request(reusable_session, request, &response).await;
    response
}

async fn send_https_request_over_session<'a, T>(
    session: &mut Session<'_, T>,
    request: HttpRequest<'_>,
    response_buffer: &'a mut [u8],
) -> Result<HttpResponse<'a>, BackendError>
where
    T: AsyncRead07 + AsyncWrite07,
{
    send_https_request_over_session_with_metrics(
        session,
        request,
        response_buffer,
        RequestMetrics::new(request.trace, true, request.class),
    )
    .await
}

async fn send_https_request_over_session_with_metrics<'a, T>(
    session: &mut Session<'_, T>,
    request: HttpRequest<'_>,
    response_buffer: &'a mut [u8],
    mut metrics: RequestMetrics,
) -> Result<HttpResponse<'a>, BackendError>
where
    T: AsyncRead07 + AsyncWrite07,
{
    write_http_request(
        session,
        request.class,
        request.path,
        request.method,
        request.content_type,
        request.bearer_token,
        request.body,
        request.connection_close,
    )
    .await?;
    log_request_phase(
        metrics.trace,
        request.path,
        request.class,
        "request_sent",
        metrics.elapsed_ms(),
    );
    let response = read_http_response(
        session,
        request.path,
        response_buffer,
        request.connection_close,
        &mut metrics,
    )
    .await;
    match &response {
        Ok(parsed) => {
            metrics.finish();
            log_request_timing(request, parsed.status, &metrics);
        }
        Err(_) => log_request_heap(request.path, "request failed"),
    }
    response
}

fn open_tls_session<'a, T>(
    tls: TlsReference<'a>,
    ca_chain: &Certificate<'static>,
    stream: T,
) -> Result<Session<'a, T>, BackendError>
where
    T: AsyncRead07 + AsyncWrite07,
{
    let mut config = ClientSessionConfig::new();
    config.ca_chain = Some(ca_chain.clone());
    config.server_name = Some(backend_host_cstr());
    // TLS 1.3 client hello key exchange is currently the failing allocation path on device
    // (`psa_export_public_key() -> MBEDTLS_ERR_SSL_ALLOC_FAILED`), so pin the device client
    // to TLS 1.2 until we can ship a lower-memory custom mbedTLS build.
    config.min_version = TlsVersion::Tls1_2;

    Session::new(tls, stream, &SessionConfig::Client(config)).map_err(|_| BackendError::Tls)
}

fn prepare_tls_session_resume<T>(
    session: &mut Session<'_, T>,
    tls_session_cache: &mut Option<SerializedClientSession>,
    path: &str,
    streaming: bool,
    metrics: &mut RequestMetrics,
) where
    T: AsyncRead07 + AsyncWrite07,
{
    let Some(cached_session_len) = tls_session_cache.as_ref().map(SerializedClientSession::len)
    else {
        return;
    };
    let resume_result = tls_session_cache
        .as_ref()
        .map(|cached_session| session.set_serialized_session(cached_session));

    match resume_result {
        Some(Ok(())) => {
            metrics.tls_resume_offered = true;
            info!(
                "backend tls session resume prepared path={} streaming={} bytes={}",
                path, streaming, cached_session_len,
            );
        }
        Some(Err(err)) => {
            info!(
                "backend tls session resume discarded path={} streaming={} err={:?}",
                path, streaming, err
            );
            *tls_session_cache = None;
        }
        None => {}
    }
}

fn discard_failed_tls_session_resume(
    tls_session_cache: &mut Option<SerializedClientSession>,
    path: &str,
    streaming: bool,
    metrics: &RequestMetrics,
) {
    if metrics.tls_resume_offered && tls_session_cache.take().is_some() {
        info!(
            "backend tls session resume discarded path={} streaming={} reason=handshake_failed",
            path, streaming
        );
    }
}

fn cache_negotiated_tls_session<T>(
    session: &Session<'_, T>,
    tls_session_cache: &mut Option<SerializedClientSession>,
    path: &str,
    streaming: bool,
) where
    T: AsyncRead07 + AsyncWrite07,
{
    match session.export_serialized_session() {
        Ok(cached_session) => {
            info!(
                "backend tls session cached path={} streaming={} bytes={}",
                path,
                streaming,
                cached_session.len(),
            );
            *tls_session_cache = Some(cached_session);
        }
        Err(err) => {
            info!(
                "backend tls session cache save failed path={} streaming={} err={:?}",
                path, streaming, err
            );
        }
    }
}

fn current_network_address(stack: Stack<'static>) -> Option<embassy_net::Ipv4Cidr> {
    stack.config_v4().map(|config| config.address)
}

fn is_backend_request_path_ready(stack: Stack<'static>) -> bool {
    stack.is_link_up()
        && current_network_address(stack).is_some()
        && crate::internet::backend_path_ready()
}

async fn wait_for_backend_request_path_ready(
    stack: Stack<'static>,
    trace: TraceContext,
    path: &str,
    class: RequestClass,
    reason: &'static str,
) -> Result<(), BackendError> {
    if is_backend_request_path_ready(stack) {
        return Ok(());
    }

    let started_ms = now_ms();
    let timeout_secs = class.network_ready_timeout_secs();
    let package_retry_wait =
        matches!(class, RequestClass::StreamingPackage) && reason == "package_retry";
    let timeout_ms = Duration::from_secs(timeout_secs).as_millis();
    let max_timeout_ms =
        Duration::from_secs(class.network_ready_max_timeout_secs(reason)).as_millis();
    let mut logged_wait = false;
    let mut last_progress_ms = started_ms;
    let mut previous_link_up = stack.is_link_up();
    let mut previous_has_ip = current_network_address(stack).is_some();
    let mut previous_backend_path_ready = crate::internet::backend_path_ready();

    loop {
        let link_up = stack.is_link_up();
        let network_address = current_network_address(stack);
        let backend_path_ready = crate::internet::backend_path_ready();

        if link_up && network_address.is_some() && backend_path_ready {
            let waited_ms = elapsed_since_ms(started_ms);
            crate::verbose_diag!(
                "backend request network ready path={} reason={} wait_ms={} network_ip={:?} backend_path_ready={}",
                path,
                reason,
                waited_ms,
                network_address,
                backend_path_ready,
            );
            log_request_phase(trace, path, class, "network_ready", waited_ms);
            return Ok(());
        }

        let waited_ms = elapsed_since_ms(started_ms);
        if package_retry_wait {
            let has_ip = network_address.is_some();
            let link_restored = !previous_link_up && link_up;
            let ip_restored = !previous_has_ip && has_ip;
            let path_restored = !previous_backend_path_ready && backend_path_ready;
            if link_restored || ip_restored || path_restored {
                last_progress_ms = now_ms();
                crate::verbose_diag!(
                    "backend request network progress path={} reason={} wait_ms={} link_up={} network_ip={:?} backend_path_ready={}",
                    path,
                    reason,
                    waited_ms,
                    link_up,
                    network_address,
                    backend_path_ready,
                );
                log_request_phase(trace, path, class, "network_wait_progress", waited_ms);
            }
            previous_link_up = link_up;
            previous_has_ip = has_ip;
            previous_backend_path_ready = backend_path_ready;

            let stalled_ms = elapsed_since_ms(last_progress_ms);
            if waited_ms >= max_timeout_ms || stalled_ms >= timeout_ms {
                warn!(
                    "backend request network wait timed out path={} reason={} wait_ms={} stalled_ms={} link_up={} network_ip={:?} backend_path_ready={}",
                    path,
                    reason,
                    waited_ms,
                    stalled_ms,
                    link_up,
                    network_address,
                    backend_path_ready,
                );
                log_request_phase(trace, path, class, "network_wait_timeout", waited_ms);
                return Err(BackendError::Connect);
            }
        } else if waited_ms >= timeout_ms {
            warn!(
                "backend request network wait timed out path={} reason={} wait_ms={} link_up={} network_ip={:?} backend_path_ready={}",
                path, reason, waited_ms, link_up, network_address, backend_path_ready,
            );
            log_request_phase(trace, path, class, "network_wait_timeout", waited_ms);
            return Err(BackendError::Connect);
        }

        if !logged_wait {
            crate::verbose_diag!(
                "backend request waiting for network path={} reason={} link_up={} network_ip={:?} backend_path_ready={}",
                path,
                reason,
                link_up,
                network_address,
                backend_path_ready,
            );
            log_request_phase(trace, path, class, "network_wait_start", waited_ms);
            logged_wait = true;
        }

        Timer::after(Duration::from_millis(REQUEST_NETWORK_READY_POLL_MS)).await;
    }
}

#[allow(clippy::too_many_arguments)]
async fn open_backend_session<'a>(
    stack: Stack<'static>,
    tls: TlsReference<'a>,
    ca_chain: &Certificate<'static>,
    tcp_client: &'a BackendTcpClient<'a>,
    tls_session_cache: &mut Option<SerializedClientSession>,
    trace: TraceContext,
    path: &str,
    class: RequestClass,
) -> Result<ConnectedBackendSession<'a>, BackendError> {
    let streaming = class.is_streaming();
    let network_address = current_network_address(stack).ok_or_else(|| {
        crate::internet::invalidate_backend_path("missing_ip");
        warn!("backend request network unavailable path={}", path);
        log_request_heap(path, "network unavailable");
        BackendError::Connect
    })?;
    crate::verbose_diag!(
        "backend request open path={} streaming={} network_ip={:?}",
        path,
        streaming,
        network_address
    );
    let mut metrics = RequestMetrics::new(trace, false, class);
    log_request_phase(trace, path, class, "open", 0);
    let resolved_remote = resolve_backend_remote(stack, trace, path, class, &mut metrics).await?;
    let connect_started_ms = now_ms();
    let connection = with_timeout(
        Duration::from_secs(class.connect_timeout_secs()),
        tcp_client.connect(SocketAddr::new(
            IpAddr::V4(resolved_remote.addr),
            BACKEND_PORT,
        )),
    )
    .await
    .map_err(|_| {
        crate::internet::invalidate_backend_path("connect_timeout");
        warn!("backend request connect timed out path={}", path);
        log_request_heap(path, "connect timeout");
        BackendError::Connect
    })?
    .map_err(|_| {
        crate::internet::invalidate_backend_path("connect_failed");
        warn!("backend request connect failed path={}", path);
        log_request_heap(path, "connect failed");
        BackendError::Connect
    })?;
    crate::internet::record_backend_endpoint(resolved_remote.addr, "request_connect_ok");
    metrics.connect_ms = elapsed_since_ms(connect_started_ms);
    crate::verbose_diag!(
        "backend request connect ok path={} remote_ip={} connect_ms={}",
        path,
        resolved_remote.addr,
        metrics.connect_ms
    );
    if matches!(resolved_remote.source, BackendEndpointSource::Cached) {
        info!(
            "backend request dns fallback succeeded path={} remote_ip={} cache_age_ms={} session_epoch={} connect_ms={}",
            path,
            resolved_remote.addr,
            resolved_remote.cache_age_ms.unwrap_or_default(),
            resolved_remote.session_epoch.unwrap_or_default(),
            metrics.connect_ms
        );
    }
    log_request_phase(trace, path, class, "connect_ok", metrics.elapsed_ms());
    log_request_heap(path, "tls setup start");
    log_request_phase(trace, path, class, "tls_setup_start", metrics.elapsed_ms());
    let mut session = open_tls_session(tls, ca_chain, CompatConnection::new(connection))
        .inspect_err(|_err| {
            warn!("backend request tls setup failed path={}", path);
            log_request_heap(path, "tls setup failed");
        })?;
    prepare_tls_session_resume(
        &mut session,
        tls_session_cache,
        path,
        streaming,
        &mut metrics,
    );
    log_request_heap(path, "tls setup ok");
    log_request_phase(trace, path, class, "tls_setup_ok", metrics.elapsed_ms());
    let tls_started_ms = now_ms();
    log_request_heap(path, "tls handshake start");
    log_request_phase(
        trace,
        path,
        class,
        "tls_handshake_start",
        metrics.elapsed_ms(),
    );
    if let Err(err) = await_tls_handshake(&mut session, path, class).await {
        discard_failed_tls_session_resume(tls_session_cache, path, streaming, &metrics);
        return Err(err);
    }
    metrics.tls_ms = elapsed_since_ms(tls_started_ms);
    log_request_heap(path, "tls handshake ok");
    log_request_phase(trace, path, class, "tls_handshake_ok", metrics.elapsed_ms());
    let verification_flags = session.tls_verification_details();
    crate::verbose_diag!(
        "backend request tls ok path={} tls_ms={} verification_flags=0x{:08x}",
        path,
        metrics.tls_ms,
        verification_flags
    );
    if verification_flags != 0 {
        warn!(
            "backend request tls verification flags path={} flags=0x{:08x}",
            path, verification_flags
        );
    }
    cache_negotiated_tls_session(&session, tls_session_cache, path, streaming);

    Ok(ConnectedBackendSession {
        session,
        network_address,
        metrics,
    })
}

async fn resolve_backend_remote(
    stack: Stack<'static>,
    trace: TraceContext,
    path: &str,
    class: RequestClass,
    metrics: &mut RequestMetrics,
) -> Result<ResolvedBackendRemote, BackendError> {
    let dns = DnsSocket::new(stack);
    let dns_started_ms = now_ms();
    let remote = dns.get_host_by_name(BACKEND_HOST, AddrType::IPv4).await;
    metrics.dns_ms = elapsed_since_ms(dns_started_ms);

    match remote {
        Ok(IpAddr::V4(addr)) => {
            crate::verbose_diag!(
                "backend request dns ok path={} remote_ip={} dns_ms={}",
                path,
                addr,
                metrics.dns_ms
            );
            log_request_phase(trace, path, class, "dns_ok", metrics.dns_ms);
            Ok(ResolvedBackendRemote {
                addr,
                source: BackendEndpointSource::Dns,
                cache_age_ms: None,
                session_epoch: None,
            })
        }
        Ok(IpAddr::V6(_)) => {
            resolve_backend_remote_from_cache(trace, path, class, metrics, "dns_invalid_family")
        }
        Err(_) => resolve_backend_remote_from_cache(trace, path, class, metrics, "dns_failed"),
    }
}

fn resolve_backend_remote_from_cache(
    trace: TraceContext,
    path: &str,
    class: RequestClass,
    metrics: &RequestMetrics,
    reason: &'static str,
) -> Result<ResolvedBackendRemote, BackendError> {
    if let Some(cached) = crate::internet::cached_backend_endpoint() {
        info!(
            "backend request dns fallback path={} remote_ip={} cache_age_ms={} session_epoch={} reason={}",
            path, cached.addr, cached.age_ms, cached.session_epoch, reason
        );
        log_request_phase(trace, path, class, "dns_fallback", metrics.dns_ms);
        return Ok(ResolvedBackendRemote {
            addr: cached.addr,
            source: BackendEndpointSource::Cached,
            cache_age_ms: Some(cached.age_ms),
            session_epoch: Some(cached.session_epoch),
        });
    }

    crate::internet::invalidate_backend_path(reason);
    info!(
        "backend request dns failed path={} reason={} cache=miss",
        path, reason
    );
    log_request_heap(path, "dns failed");
    Err(BackendError::Dns)
}

async fn close_backend_tls_session<T>(session: &mut Session<'_, T>, reason: &str)
where
    T: AsyncRead07 + AsyncWrite07,
{
    info!("backend tls close start reason={}", reason);
    match with_timeout(Duration::from_millis(250), session.close()).await {
        Ok(Ok(())) => {}
        Ok(Err(err)) => warn!("backend tls close failed reason={} err={:?}", reason, err),
        Err(_) => warn!("backend tls close timed out reason={}", reason),
    }
}

async fn close_reusable_session(
    reusable_session: &mut Option<ReusableBackendSession<'_>>,
    reason: &str,
) {
    if let Some(mut reusable) = reusable_session.take() {
        close_backend_tls_session(&mut reusable.session, reason).await;
    }
}

fn discard_reusable_session(
    reusable_session: &mut Option<ReusableBackendSession<'_>>,
    reason: &str,
) {
    if reusable_session.take().is_some() {
        info!("backend reusable session discarded reason={}", reason);
    }
}

async fn invalidate_access_state(
    access_session: &mut Option<ActiveAccessSession>,
    reusable_session: &mut Option<ReusableBackendSession<'_>>,
) {
    *access_session = None;
    close_reusable_session(reusable_session, "auth invalid").await;
}

#[allow(clippy::too_many_arguments)]
async fn ensure_reusable_session<'a>(
    stack: Stack<'static>,
    tls: TlsReference<'a>,
    ca_chain: &Certificate<'static>,
    tcp_client: &'a BackendTcpClient<'a>,
    reusable_session: &mut Option<ReusableBackendSession<'a>>,
    tls_session_cache: &mut Option<SerializedClientSession>,
    trace: TraceContext,
    path: &str,
    class: RequestClass,
) -> Result<RequestMetrics, BackendError> {
    let streaming = class.is_streaming();
    let now_ms = now_ms();
    match reusable_session.as_ref() {
        Some(session) if session.is_usable_on(stack, now_ms, streaming) => {
            info!(
                "backend reusable session reuse path={} streaming={} age_ms={} session_ip={:?}",
                path,
                streaming,
                now_ms.saturating_sub(session.last_used_ms),
                session.network_address,
            );
            return Ok(RequestMetrics::new(trace, true, class));
        }
        Some(session) => {
            let link_up = stack.is_link_up();
            let current_network = current_network_address(stack);
            info!(
                "backend reusable session discard path={} streaming={} reason={} age_ms={} idle_limit_ms={} link_up={} current_network={:?} session_network={:?}",
                path,
                streaming,
                reusable_session_discard_reason(stack, session, now_ms, streaming),
                now_ms.saturating_sub(session.last_used_ms),
                reusable_session_idle_timeout_ms(streaming),
                link_up,
                current_network,
                session.network_address,
            );
        }
        None => {
            info!(
                "backend reusable session missing path={} streaming={}",
                path, streaming
            );
        }
    }

    discard_reusable_session(reusable_session, "stale");
    let ConnectedBackendSession {
        session,
        network_address,
        metrics,
    } = open_backend_session(
        stack,
        tls,
        ca_chain,
        tcp_client,
        tls_session_cache,
        trace,
        path,
        class,
    )
    .await?;
    *reusable_session = Some(ReusableBackendSession {
        session,
        network_address,
        last_used_ms: now_ms,
    });
    Ok(metrics)
}

fn should_keep_connection_alive(request: HttpRequest<'_>, response_reusable: bool) -> bool {
    !request.connection_close && response_reusable
}

async fn update_reusable_session_after_buffered_request(
    reusable_session: &mut Option<ReusableBackendSession<'_>>,
    request: HttpRequest<'_>,
    response: &Result<HttpResponse<'_>, BackendError>,
) {
    match response {
        Ok(response) if should_keep_connection_alive(request, response.connection_reusable) => {
            info!(
                "backend reusable session keep path={} status={} response_reusable={}",
                request.path, response.status, response.connection_reusable
            );
            if let Some(reusable) = reusable_session.as_mut() {
                reusable.mark_used(now_ms());
            }
        }
        Ok(response) => {
            info!(
                "backend reusable session close path={} status={} reason=response_not_reusable request_close={} response_reusable={}",
                request.path,
                response.status,
                request.connection_close,
                response.connection_reusable,
            );
            close_reusable_session(reusable_session, "buffered done").await;
        }
        Err(err) => {
            info!(
                "backend reusable session close path={} reason=request_failed err={:?}",
                request.path, err
            );
            close_reusable_session(reusable_session, "buffered done").await;
        }
    }
}

async fn update_reusable_session_after_streaming_request(
    reusable_session: &mut Option<ReusableBackendSession<'_>>,
    request: HttpRequest<'_>,
    response: &Result<StreamingHttpResponse, BackendError>,
) {
    match response {
        Ok(response) if should_keep_connection_alive(request, response.connection_reusable) => {
            info!(
                "backend reusable session keep path={} status={} response_reusable={}",
                request.path, response.status, response.connection_reusable
            );
            if let Some(reusable) = reusable_session.as_mut() {
                reusable.mark_used(now_ms());
            }
        }
        Ok(response) => {
            info!(
                "backend reusable session close path={} status={} reason=response_not_reusable request_close={} response_reusable={}",
                request.path,
                response.status,
                request.connection_close,
                response.connection_reusable,
            );
            close_reusable_session(reusable_session, "stream done").await;
        }
        Err(err) => {
            info!(
                "backend reusable session close path={} reason=request_failed err={:?}",
                request.path, err
            );
            close_reusable_session(reusable_session, "stream done").await;
        }
    }
}

enum NetworkByteBuffer<const N: usize> {
    External(crate::memory_policy::ExternalBox<[u8; N]>),
    Internal(crate::memory_policy::InternalBox<[u8; N]>),
}

impl<const N: usize> NetworkByteBuffer<N> {
    fn as_mut_slice(&mut self) -> &mut [u8] {
        match self {
            Self::External(buffer) => &mut buffer[..],
            Self::Internal(buffer) => &mut buffer[..],
        }
    }
}

fn allocate_zeroed_network_buffer<const N: usize>(
    path: &str,
    kind: &str,
) -> Result<NetworkByteBuffer<N>, BackendError> {
    match crate::memory_policy::try_external_zeroed_array_box::<N>() {
        Ok(buffer) => {
            info!(
                "backend buffer alloc kind={} path={} bytes={} placement=external",
                kind, path, N
            );
            crate::memtrace!(
                "request_buffer_alloc",
                "component" = "backend",
                "at_ms" = now_ms(),
                "path" = path,
                "buffer_kind" = kind,
                "bytes" = N,
                "placement" = "external",
            );
            Ok(NetworkByteBuffer::External(buffer))
        }
        Err(_) => match crate::memory_policy::try_internal_zeroed_array_box::<N>() {
            Ok(buffer) => {
                info!(
                    "backend buffer alloc kind={} path={} bytes={} placement=internal_fallback",
                    kind, path, N
                );
                crate::memtrace!(
                    "request_buffer_alloc",
                    "component" = "backend",
                    "at_ms" = now_ms(),
                    "path" = path,
                    "buffer_kind" = kind,
                    "bytes" = N,
                    "placement" = "internal_fallback",
                );
                Ok(NetworkByteBuffer::Internal(buffer))
            }
            Err(_) => {
                info!(
                    "backend buffer alloc failed kind={} path={} bytes={}",
                    kind, path, N
                );
                crate::memtrace!(
                    "request_buffer_alloc",
                    "component" = "backend",
                    "at_ms" = now_ms(),
                    "path" = path,
                    "buffer_kind" = kind,
                    "bytes" = N,
                    "placement" = "failed",
                );
                log_request_heap(path, "buffer alloc failed");
                Err(BackendError::Alloc)
            }
        },
    }
}

fn allocate_standard_response_buffer(
    path: &str,
) -> Result<NetworkByteBuffer<HTTP_RESPONSE_MAX_LEN>, BackendError> {
    allocate_zeroed_network_buffer::<HTTP_RESPONSE_MAX_LEN>(path, "buffered_response")
}

fn allocate_stream_header_buffer(
    path: &str,
) -> Result<NetworkByteBuffer<HTTP_STREAM_HEADER_MAX_LEN>, BackendError> {
    allocate_zeroed_network_buffer::<HTTP_STREAM_HEADER_MAX_LEN>(path, "stream_header")
}

fn allocate_stream_chunk_buffer(
    path: &str,
) -> Result<NetworkByteBuffer<PACKAGE_DOWNLOAD_CHUNK_LEN>, BackendError> {
    allocate_zeroed_network_buffer::<PACKAGE_DOWNLOAD_CHUNK_LEN>(path, "stream_chunk")
}

fn map_logged_session_error(path: &str, stage: &str, error: SessionError) -> BackendError {
    match error {
        SessionError::MbedTls(err) => {
            crate::internet::invalidate_backend_path("tls_failed");
            info!(
                "backend request tls {} failed path={} err={:?}",
                stage, path, err
            );
            log_request_heap(path, stage);
            BackendError::Tls
        }
        SessionError::Io(err) => {
            crate::internet::invalidate_backend_path("io_failed");
            info!(
                "backend request io {} failed path={} err={:?}",
                stage, path, err
            );
            log_request_heap(path, stage);
            BackendError::Io
        }
    }
}

fn log_request_timeout(
    path: &str,
    class: RequestClass,
    stage: &str,
    error: BackendError,
) -> BackendError {
    if is_transient_transport_error(error) {
        let reason = match error {
            BackendError::Dns => "dns_timeout",
            BackendError::Connect => "connect_timeout",
            BackendError::Tls => "tls_timeout",
            BackendError::Io => "io_timeout",
            _ => unreachable!(),
        };
        crate::internet::invalidate_backend_path(reason);
    }
    info!(
        "backend request {} timed out path={} class={}",
        stage,
        path,
        class.label()
    );
    log_request_heap(path, stage);
    error
}

async fn await_tls_handshake<T>(
    session: &mut Session<'_, T>,
    path: &str,
    class: RequestClass,
) -> Result<(), BackendError>
where
    T: AsyncRead07 + AsyncWrite07,
{
    with_timeout(
        Duration::from_secs(class.tls_handshake_timeout_secs()),
        session.connect(),
    )
    .await
    .map_err(|_| log_request_timeout(path, class, "handshake", BackendError::Tls))?
    .map_err(|err| map_logged_session_error(path, "handshake", err))
}

async fn await_body_io_timeout<T, F>(
    path: &str,
    class: RequestClass,
    stage: &str,
    future: F,
) -> Result<T, BackendError>
where
    F: Future<Output = Result<T, SessionError>>,
{
    await_body_io_timeout_for(
        path,
        class,
        stage,
        Duration::from_secs(class.io_timeout_secs()),
        future,
    )
    .await
}

async fn await_body_io_timeout_for<T, F>(
    path: &str,
    class: RequestClass,
    stage: &str,
    timeout: Duration,
    future: F,
) -> Result<T, BackendError>
where
    F: Future<Output = Result<T, SessionError>>,
{
    with_timeout(timeout, future)
        .await
        .map_err(|_| log_request_timeout(path, class, stage, BackendError::Io))?
        .map_err(|err| map_logged_session_error(path, stage, err))
}

#[allow(clippy::too_many_arguments)]
async fn write_http_request<T>(
    session: &mut Session<'_, T>,
    class: RequestClass,
    path: &str,
    method: &str,
    content_type: Option<&str>,
    bearer_token: Option<&str>,
    body: &[u8],
    connection_close: bool,
) -> Result<(), BackendError>
where
    T: AsyncRead07 + AsyncWrite07,
{
    await_body_io_timeout(
        path,
        class,
        "write method",
        AsyncWrite07::write_all(session, method.as_bytes()),
    )
    .await?;
    await_body_io_timeout(
        path,
        class,
        "write separator",
        AsyncWrite07::write_all(session, b" "),
    )
    .await?;
    await_body_io_timeout(
        path,
        class,
        "write path",
        AsyncWrite07::write_all(session, path.as_bytes()),
    )
    .await?;
    await_body_io_timeout(
        path,
        class,
        "write request line",
        AsyncWrite07::write_all(session, b" HTTP/1.1\r\nHost: "),
    )
    .await?;
    await_body_io_timeout(
        path,
        class,
        "write host",
        AsyncWrite07::write_all(session, BACKEND_HOST.as_bytes()),
    )
    .await?;
    await_body_io_timeout(
        path,
        class,
        "write user agent header",
        AsyncWrite07::write_all(session, b"\r\nUser-Agent: "),
    )
    .await?;
    await_body_io_timeout(
        path,
        class,
        "write user agent",
        AsyncWrite07::write_all(session, USER_AGENT.as_bytes()),
    )
    .await?;
    await_body_io_timeout(
        path,
        class,
        "write connection header",
        AsyncWrite07::write_all(session, b"\r\nAccept: application/json\r\nConnection: "),
    )
    .await?;
    await_body_io_timeout(
        path,
        class,
        "write connection value",
        AsyncWrite07::write_all(
            session,
            if connection_close {
                b"close\r\n"
            } else {
                b"keep-alive\r\n"
            },
        ),
    )
    .await?;

    if let Some(token) = bearer_token {
        await_body_io_timeout(
            path,
            class,
            "write auth header",
            AsyncWrite07::write_all(session, b"Authorization: Bearer "),
        )
        .await?;
        await_body_io_timeout(
            path,
            class,
            "write auth token",
            AsyncWrite07::write_all(session, token.as_bytes()),
        )
        .await?;
        await_body_io_timeout(
            path,
            class,
            "write auth line ending",
            AsyncWrite07::write_all(session, b"\r\n"),
        )
        .await?;
    }

    if !body.is_empty() {
        if let Some(content_type) = content_type {
            await_body_io_timeout(
                path,
                class,
                "write content type header",
                AsyncWrite07::write_all(session, b"Content-Type: "),
            )
            .await?;
            await_body_io_timeout(
                path,
                class,
                "write content type",
                AsyncWrite07::write_all(session, content_type.as_bytes()),
            )
            .await?;
            await_body_io_timeout(
                path,
                class,
                "write content type line ending",
                AsyncWrite07::write_all(session, b"\r\n"),
            )
            .await?;
        }

        let mut content_length = heapless::String::<16>::new();
        write!(&mut content_length, "{}", body.len()).map_err(|_| BackendError::InvalidResponse)?;
        await_body_io_timeout(
            path,
            class,
            "write content length header",
            AsyncWrite07::write_all(session, b"Content-Length: "),
        )
        .await?;
        await_body_io_timeout(
            path,
            class,
            "write content length",
            AsyncWrite07::write_all(session, content_length.as_bytes()),
        )
        .await?;
        await_body_io_timeout(
            path,
            class,
            "write content length line ending",
            AsyncWrite07::write_all(session, b"\r\n"),
        )
        .await?;
    }

    await_body_io_timeout(
        path,
        class,
        "write header terminator",
        AsyncWrite07::write_all(session, b"\r\n"),
    )
    .await?;

    if !body.is_empty() {
        await_body_io_timeout(
            path,
            class,
            "write body",
            AsyncWrite07::write_all(session, body),
        )
        .await?;
    }

    await_body_io_timeout(path, class, "flush", AsyncWrite07::flush(session)).await?;
    Ok(())
}

async fn read_http_response<'a, T>(
    session: &mut Session<'_, T>,
    path: &str,
    response_buffer: &'a mut [u8],
    connection_close: bool,
    metrics: &mut RequestMetrics,
) -> Result<HttpResponse<'a>, BackendError>
where
    T: AsyncRead07 + AsyncWrite07,
{
    let mut total = 0usize;
    let mut expected_total = None;
    let mut saw_headers = false;

    loop {
        if total == response_buffer.len() {
            return Err(BackendError::ResponseTooLarge);
        }

        let read = match await_body_io_timeout(
            path,
            metrics.class,
            "response read",
            AsyncRead07::read(session, &mut response_buffer[total..]),
        )
        .await
        {
            Ok(read) => read,
            Err(backend_error) if total > 0 => {
                info!(
                    "backend request response read interrupted path={} received_bytes={} mapped={:?}",
                    path, total, backend_error
                );
                break;
            }
            Err(err) => return Err(err),
        };
        if read == 0 {
            if let Some(expected_total) = expected_total
                && total < expected_total
            {
                return Err(BackendError::InvalidResponse);
            }
            break;
        }
        metrics.mark_first_byte();
        total += read;

        if !saw_headers
            && let Some(header_end) = find_subslice(&response_buffer[..total], b"\r\n\r\n")
        {
            let metadata = parse_http_response_metadata(&response_buffer[..header_end + 4])?;
            if metadata.chunked {
                return Err(BackendError::InvalidResponse);
            }

            expected_total = match metadata.content_length {
                Some(content_length) => {
                    let total_len = metadata
                        .body_start
                        .checked_add(content_length)
                        .ok_or(BackendError::ResponseTooLarge)?;
                    if total_len > response_buffer.len() {
                        return Err(BackendError::ResponseTooLarge);
                    }
                    Some(total_len)
                }
                None if connection_close => None,
                None => return Err(BackendError::InvalidResponse),
            };
            metrics.header_bytes = metadata.body_start;
            metrics.response_buffer_capacity = response_buffer.len();
            metrics.content_length = metadata.content_length.unwrap_or(0);
            metrics.content_length_known = metadata.content_length.is_some();
            metrics.response_buffer_headroom = expected_total
                .map(|expected| response_buffer.len().saturating_sub(expected))
                .unwrap_or_else(|| response_buffer.len().saturating_sub(total));
            crate::memtrace!(
                "request_headers",
                "component" = "backend",
                "at_ms" = now_ms(),
                "sync_id" = metrics.trace.sync_id,
                "req_id" = metrics.trace.req_id,
                "path" = path,
                "request_class" = metrics.class.label(),
                "streaming" = bool_flag(metrics.streaming),
                "header_bytes" = metadata.body_start,
                "initial_body_bytes" = total.saturating_sub(metadata.body_start),
                "content_length_known" = bool_flag(metadata.content_length.is_some()),
                "content_length" = metadata.content_length.unwrap_or(0),
                "response_buffer_capacity" = response_buffer.len(),
                "response_buffer_headroom" = metrics.response_buffer_headroom,
                "connection_close" = bool_flag(connection_close),
            );
            saw_headers = true;
        }

        if let Some(expected_total) = expected_total
            && total >= expected_total
        {
            total = expected_total;
            break;
        }
    }

    let parsed = parse_http_response(&response_buffer[..total])?;
    metrics.body_bytes = parsed.body.len();
    metrics.response_bytes = total;
    metrics.response_buffer_capacity = response_buffer.len();
    metrics.response_buffer_headroom = response_buffer.len().saturating_sub(total);
    if !metrics.content_length_known {
        metrics.content_length = parsed.body.len();
    }
    Ok(parsed)
}

fn parse_http_response(response: &[u8]) -> Result<HttpResponse<'_>, BackendError> {
    let metadata = parse_http_response_metadata(response)?;
    if metadata.chunked {
        return Err(BackendError::InvalidResponse);
    }

    let body_end = match metadata.content_length {
        Some(content_length) => metadata
            .body_start
            .checked_add(content_length)
            .ok_or(BackendError::ResponseTooLarge)?,
        None => response.len(),
    };
    if body_end > response.len() {
        return Err(BackendError::InvalidResponse);
    }

    let body = str::from_utf8(&response[metadata.body_start..body_end])
        .map_err(|_| BackendError::InvalidUtf8)?;

    Ok(HttpResponse {
        status: metadata.status,
        body,
        connection_reusable: is_response_connection_reusable(metadata),
    })
}

fn parse_http_response_metadata(response: &[u8]) -> Result<HttpResponseMetadata, BackendError> {
    let header_end = find_subslice(response, b"\r\n\r\n").ok_or(BackendError::InvalidResponse)?;
    let status = parse_http_status(response)?;
    let header_text =
        str::from_utf8(&response[..header_end]).map_err(|_| BackendError::InvalidUtf8)?;
    let mut content_length = None;
    let mut chunked = false;
    let mut connection_close = false;

    for line in header_text.split("\r\n").skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-length") {
            content_length = Some(
                value
                    .parse::<usize>()
                    .map_err(|_| BackendError::InvalidResponse)?,
            );
            continue;
        }
        if name.eq_ignore_ascii_case("transfer-encoding")
            && value
                .split(',')
                .any(|encoding| encoding.trim().eq_ignore_ascii_case("chunked"))
        {
            chunked = true;
            continue;
        }
        if name.eq_ignore_ascii_case("connection")
            && value
                .split(',')
                .any(|token| token.trim().eq_ignore_ascii_case("close"))
        {
            connection_close = true;
        }
    }

    Ok(HttpResponseMetadata {
        status,
        body_start: header_end + 4,
        content_length,
        chunked,
        connection_close,
    })
}

fn is_response_connection_reusable(metadata: HttpResponseMetadata) -> bool {
    metadata.content_length.is_some() && !metadata.chunked && !metadata.connection_close
}

fn parse_refresh_session(body: &str) -> Result<RefreshSession, RefreshError> {
    let access_token = extract_json_string(body, "\"access_token\"")
        .ok_or(RefreshError::Other(BackendError::MissingField))?;
    let refresh_token = extract_json_string(body, "\"refresh_token\"")
        .ok_or(RefreshError::Other(BackendError::MissingField))?;
    let expires_in = extract_json_u64(body, "\"expires_in\"")
        .ok_or(RefreshError::Other(BackendError::MissingField))?;

    Ok(RefreshSession {
        access_token: bounded_string(access_token).map_err(RefreshError::Other)?,
        refresh_token: bounded_string(refresh_token).map_err(RefreshError::Other)?,
        expires_in,
    })
}

fn parse_me_profile(body: &str) -> Result<MeProfile, IdentityError> {
    let user_id = extract_json_string(body, "\"user_id\"")
        .ok_or(IdentityError::Other(BackendError::MissingField))?;
    let role = extract_json_string(body, "\"role\"")
        .ok_or(IdentityError::Other(BackendError::MissingField))?;

    Ok(MeProfile {
        user_id: bounded_string(user_id).map_err(IdentityError::Other)?,
        role: bounded_string(role).map_err(IdentityError::Other)?,
    })
}

fn build_collection_page_path(
    endpoint: CollectionEndpoint,
    cursor: Option<&heapless::String<COLLECTION_CURSOR_MAX_LEN>>,
) -> Result<heapless::String<COLLECTION_PAGE_PATH_MAX_LEN>, BackendError> {
    let mut path = heapless::String::<COLLECTION_PAGE_PATH_MAX_LEN>::new();
    write!(
        &mut path,
        "{}?limit={}",
        endpoint.path(),
        COLLECTION_PAGE_LIMIT
    )
    .map_err(|_| BackendError::ResponseTooLarge)?;
    path.push_str(endpoint.extra_query())
        .map_err(|_| BackendError::ResponseTooLarge)?;
    if let Some(cursor) = cursor {
        path.push_str("&cursor=")
            .map_err(|_| BackendError::ResponseTooLarge)?;
        path.push_str(cursor.as_str())
            .map_err(|_| BackendError::ResponseTooLarge)?;
    }
    Ok(path)
}

fn parse_collection_page_cursor(
    endpoint: CollectionEndpoint,
    body: &str,
) -> Result<Option<heapless::String<COLLECTION_CURSOR_MAX_LEN>>, BackendError> {
    match endpoint {
        CollectionEndpoint::Recommendations => {
            extract_json_optional_string(body, "\"next_cursor\"")
                .unwrap_or(None)
                .map(bounded_string)
                .transpose()
        }
        CollectionEndpoint::Inbox | CollectionEndpoint::Saved => {
            extract_json_optional_string(body, "\"next_cursor\"")
                .ok_or(BackendError::MissingField)?
                .map(bounded_string)
                .transpose()
        }
    }
}

fn collection_body_preview(
    body: &str,
    item_count: usize,
) -> Result<(Option<heapless::String<INBOX_LOG_PREVIEW_MAX_LEN>>, bool), BackendError> {
    if item_count == 0 {
        Ok((None, false))
    } else {
        let (preview, truncated) = utf8_log_prefix(body, INBOX_LOG_PREVIEW_MAX_LEN);
        Ok((Some(bounded_string(preview)?), truncated))
    }
}

fn parse_inbox_fetch_page(body: &str) -> Result<CollectionFetchPage, BackendError> {
    let collection = parse_inbox_collection(body)?;
    let next_cursor = parse_collection_page_cursor(CollectionEndpoint::Inbox, body)?;
    let (body_preview, body_preview_truncated) = collection_body_preview(body, collection.len())?;
    Ok(CollectionFetchPage {
        collection,
        next_cursor,
        body_preview,
        body_preview_truncated,
    })
}

fn parse_saved_content_fetch_page(body: &str) -> Result<CollectionFetchPage, BackendError> {
    let collection = parse_saved_content_collection(body)?;
    let next_cursor = parse_collection_page_cursor(CollectionEndpoint::Saved, body)?;
    let (body_preview, body_preview_truncated) = collection_body_preview(body, collection.len())?;
    Ok(CollectionFetchPage {
        collection,
        next_cursor,
        body_preview,
        body_preview_truncated,
    })
}

fn parse_recommendation_fetch_page(body: &str) -> Result<CollectionFetchPage, BackendError> {
    let collection = parse_recommendation_collection(body)?;
    let next_cursor = parse_collection_page_cursor(CollectionEndpoint::Recommendations, body)?;
    let (body_preview, body_preview_truncated) = collection_body_preview(body, collection.len())?;
    Ok(CollectionFetchPage {
        collection,
        next_cursor,
        body_preview,
        body_preview_truncated,
    })
}

fn parse_collection_fetch_page(
    endpoint: CollectionEndpoint,
    body: &str,
) -> Result<CollectionFetchPage, BackendError> {
    match endpoint {
        CollectionEndpoint::Inbox => parse_inbox_fetch_page(body),
        CollectionEndpoint::Saved => parse_saved_content_fetch_page(body),
        CollectionEndpoint::Recommendations => parse_recommendation_fetch_page(body),
    }
}

#[allow(clippy::too_many_arguments)]
fn log_collection_fetch_page_metrics(
    trace: TraceContext,
    kind: CollectionKind,
    path: &str,
    page_index: usize,
    body_bytes: usize,
    item_count: usize,
    accepted_items: usize,
    next_cursor_present: bool,
    response_body_headroom: usize,
    merged_item_count: usize,
    truncated_by_capacity: bool,
) {
    let avg_item_bytes = if item_count == 0 {
        0
    } else {
        body_bytes / item_count
    };
    let estimated_max_items = if item_count == 0 || avg_item_bytes == 0 {
        item_count
    } else {
        item_count.saturating_add(response_body_headroom / avg_item_bytes)
    };

    crate::memtrace!(
        "collection_fetch_page",
        "component" = "backend",
        "at_ms" = now_ms(),
        "sync_id" = trace.sync_id,
        "req_id" = trace.req_id,
        "collection" = collection_label(kind),
        "path" = path,
        "page_index" = page_index,
        "body_bytes" = body_bytes,
        "item_count" = item_count,
        "accepted_items" = accepted_items,
        "merged_item_count" = merged_item_count,
        "avg_item_bytes" = avg_item_bytes,
        "next_cursor_present" = bool_flag(next_cursor_present),
        "response_body_headroom" = response_body_headroom,
        "estimated_max_items" = estimated_max_items,
        "page_limit" = COLLECTION_PAGE_LIMIT,
        "truncated_by_capacity" = bool_flag(truncated_by_capacity),
    );
}

fn log_collection_fetch_total_metrics(
    trace: TraceContext,
    kind: CollectionKind,
    page_count: usize,
    body_bytes_total: usize,
    item_count: usize,
    next_cursor_present: bool,
    truncated_by_capacity: bool,
) {
    crate::memtrace!(
        "collection_fetch_total",
        "component" = "backend",
        "at_ms" = now_ms(),
        "sync_id" = trace.sync_id,
        "req_id" = trace.req_id,
        "collection" = collection_label(kind),
        "page_count" = page_count,
        "page_limit" = COLLECTION_PAGE_LIMIT,
        "body_bytes_total" = body_bytes_total,
        "item_count" = item_count,
        "next_cursor_present" = bool_flag(next_cursor_present),
        "truncated_by_capacity" = bool_flag(truncated_by_capacity),
    );
}

#[allow(clippy::too_many_arguments)]
async fn perform_collection_fetch_paginated(
    stack: Stack<'static>,
    tls: TlsReference<'_>,
    ca_chain: &Certificate<'static>,
    tcp_state: &BackendTcpClientState,
    tls_session_cache: &mut Option<SerializedClientSession>,
    access_token: &str,
    sync_id: u32,
    endpoint: CollectionEndpoint,
) -> Result<CollectionFetchResult, CollectionQueryError> {
    let mut accumulator = CollectionFetchAccumulator::new(next_request_trace(sync_id));
    let mut page_index = 0usize;
    let mut cursor = None::<heapless::String<COLLECTION_CURSOR_MAX_LEN>>;

    loop {
        if page_index >= COLLECTION_FETCH_MAX_PAGES {
            return Err(CollectionQueryError::Other(BackendError::InvalidResponse));
        }

        let path = build_collection_page_path(endpoint, cursor.as_ref())
            .map_err(CollectionQueryError::Other)?;
        let request_trace = if page_index == 0 {
            accumulator.trace
        } else {
            next_request_trace(sync_id)
        };
        let mut response_buffer = allocate_standard_response_buffer(path.as_str())
            .map_err(CollectionQueryError::Other)?;
        let response = send_https_request(
            stack,
            tls,
            ca_chain,
            tcp_state,
            tls_session_cache,
            HttpRequest {
                trace: request_trace,
                class: RequestClass::BufferedMetadata,
                method: "GET",
                path: path.as_str(),
                content_type: Some("application/json"),
                bearer_token: Some(access_token),
                body: b"",
                connection_close: true,
            },
            response_buffer.as_mut_slice(),
        )
        .await
        .map_err(CollectionQueryError::Other)?;

        if (400..500).contains(&response.status) {
            return Err(CollectionQueryError::Rejected(response.status));
        }
        if response.status != 200 {
            return Err(CollectionQueryError::Other(BackendError::InvalidResponse));
        }

        let page = parse_collection_fetch_page(endpoint, response.body)
            .map_err(CollectionQueryError::Other)?;
        if page.collection.is_empty() && page.next_cursor.is_some() {
            return Err(CollectionQueryError::Other(BackendError::InvalidResponse));
        }
        accumulator.absorb_page(
            endpoint,
            path.as_str(),
            response.body.len(),
            page_index,
            page,
        );
        page_index += 1;

        if !accumulator.should_continue() {
            break;
        }
        cursor = accumulator.next_cursor.clone();
    }

    log_collection_fetch_total_metrics(
        accumulator.trace,
        endpoint.kind(),
        accumulator.page_count,
        accumulator.body_bytes_total,
        accumulator.collection.len(),
        accumulator.next_cursor.is_some() || accumulator.truncated_by_capacity,
        accumulator.truncated_by_capacity,
    );
    Ok(accumulator.into_result())
}

async fn perform_saved_content_fetch_paginated_over_session<T>(
    session: &mut Session<'_, T>,
    access_token: &str,
    connection_close: bool,
    sync_id: u32,
) -> Result<CollectionFetchResult, CollectionQueryError>
where
    T: AsyncRead07 + AsyncWrite07,
{
    let mut accumulator = CollectionFetchAccumulator::new(next_request_trace(sync_id));
    let mut page_index = 0usize;
    let mut cursor = None::<heapless::String<COLLECTION_CURSOR_MAX_LEN>>;

    loop {
        if page_index >= COLLECTION_FETCH_MAX_PAGES {
            return Err(CollectionQueryError::Other(BackendError::InvalidResponse));
        }

        let path = build_collection_page_path(CollectionEndpoint::Saved, cursor.as_ref())
            .map_err(CollectionQueryError::Other)?;
        let request_trace = if page_index == 0 {
            accumulator.trace
        } else {
            next_request_trace(sync_id)
        };
        let mut response_buffer = allocate_standard_response_buffer(path.as_str())
            .map_err(CollectionQueryError::Other)?;
        let response = send_https_request_over_session(
            session,
            HttpRequest {
                trace: request_trace,
                class: RequestClass::BufferedMetadata,
                method: "GET",
                path: path.as_str(),
                content_type: Some("application/json"),
                bearer_token: Some(access_token),
                body: b"",
                connection_close: connection_close && page_index == 0,
            },
            response_buffer.as_mut_slice(),
        )
        .await
        .map_err(CollectionQueryError::Other)?;

        if (400..500).contains(&response.status) {
            return Err(CollectionQueryError::Rejected(response.status));
        }
        if response.status != 200 {
            return Err(CollectionQueryError::Other(BackendError::InvalidResponse));
        }

        let page =
            parse_saved_content_fetch_page(response.body).map_err(CollectionQueryError::Other)?;
        if page.collection.is_empty() && page.next_cursor.is_some() {
            return Err(CollectionQueryError::Other(BackendError::InvalidResponse));
        }
        accumulator.absorb_page(
            CollectionEndpoint::Saved,
            path.as_str(),
            response.body.len(),
            page_index,
            page,
        );
        page_index += 1;

        if !accumulator.should_continue() {
            break;
        }
        cursor = accumulator.next_cursor.clone();
    }

    log_collection_fetch_total_metrics(
        accumulator.trace,
        CollectionEndpoint::Saved.kind(),
        accumulator.page_count,
        accumulator.body_bytes_total,
        accumulator.collection.len(),
        accumulator.next_cursor.is_some() || accumulator.truncated_by_capacity,
        accumulator.truncated_by_capacity,
    );
    Ok(accumulator.into_result())
}

async fn perform_inbox_fetch(
    stack: Stack<'static>,
    tls: TlsReference<'_>,
    ca_chain: &Certificate<'static>,
    tcp_state: &BackendTcpClientState,
    tls_session_cache: &mut Option<SerializedClientSession>,
    access_token: &str,
    sync_id: u32,
) -> Result<CollectionFetchResult, CollectionQueryError> {
    perform_collection_fetch_paginated(
        stack,
        tls,
        ca_chain,
        tcp_state,
        tls_session_cache,
        access_token,
        sync_id,
        CollectionEndpoint::Inbox,
    )
    .await
}

async fn perform_saved_content_fetch(
    stack: Stack<'static>,
    tls: TlsReference<'_>,
    ca_chain: &Certificate<'static>,
    tcp_state: &BackendTcpClientState,
    tls_session_cache: &mut Option<SerializedClientSession>,
    access_token: &str,
    sync_id: u32,
) -> Result<CollectionFetchResult, CollectionQueryError> {
    perform_collection_fetch_paginated(
        stack,
        tls,
        ca_chain,
        tcp_state,
        tls_session_cache,
        access_token,
        sync_id,
        CollectionEndpoint::Saved,
    )
    .await
}

async fn perform_recommendation_fetch(
    stack: Stack<'static>,
    tls: TlsReference<'_>,
    ca_chain: &Certificate<'static>,
    tcp_state: &BackendTcpClientState,
    tls_session_cache: &mut Option<SerializedClientSession>,
    access_token: &str,
    sync_id: u32,
) -> Result<CollectionFetchResult, CollectionQueryError> {
    perform_collection_fetch_paginated(
        stack,
        tls,
        ca_chain,
        tcp_state,
        tls_session_cache,
        access_token,
        sync_id,
        CollectionEndpoint::Recommendations,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn fetch_and_stage_package<'a>(
    stack: Stack<'static>,
    tls: TlsReference<'a>,
    ca_chain: &Certificate<'static>,
    tcp_client: &'a BackendTcpClient<'a>,
    access_token: &str,
    reusable_session: &mut Option<ReusableBackendSession<'a>>,
    tls_session_cache: &mut Option<SerializedClientSession>,
    request: PrepareContentRequest,
    sync_id: u32,
) -> Result<content_storage::CommitAndOpenPackageResult, PackagePrepareError> {
    let path = build_package_path(request).map_err(PackagePrepareError::Other)?;
    let mut attempt = 0usize;
    let mut recovery_wait_failures = 0usize;

    loop {
        let request_trace = next_request_trace(sync_id);
        if let Err(err) = wait_for_backend_request_path_ready(
            stack,
            request_trace,
            path.as_str(),
            RequestClass::StreamingPackage,
            if attempt == 0 {
                "package_fetch"
            } else {
                "package_retry"
            },
        )
        .await
        {
            if attempt > 0 && recovery_wait_failures + 1 < PACKAGE_RETRY_RECOVERY_ATTEMPTS {
                recovery_wait_failures += 1;
                info!(
                    "backend package retry recovery content_id={} wait_attempt={} transport_attempt={} err={:?}",
                    request.content_id.as_str(),
                    recovery_wait_failures,
                    attempt,
                    err
                );
                Timer::after(Duration::from_millis(TRANSPORT_RETRY_BACKOFF_MS)).await;
                continue;
            }
            return Err(PackagePrepareError::Other(err));
        }
        recovery_wait_failures = 0;

        info!(
            "backend package fetch begin content_id={} remote_item_id={} revision={} collection={:?} path={} outer_attempt={}/{}",
            request.content_id.as_str(),
            request.remote_item_id.as_str(),
            request.remote_revision,
            request.collection,
            path.as_str(),
            attempt + 1,
            PACKAGE_TRANSPORT_RETRY_ATTEMPTS,
        );
        content_storage::begin_package_stage_traced(
            request_trace,
            request.content_id,
            request.remote_revision,
        )
        .await
        .map_err(map_storage_prepare_error)?;

        let response = match stream_https_response_body_to_storage_reusing_session(
            stack,
            tls,
            ca_chain,
            tcp_client,
            reusable_session,
            tls_session_cache,
            HttpRequest {
                trace: request_trace,
                class: RequestClass::StreamingPackage,
                method: "GET",
                path: path.as_str(),
                content_type: Some("application/json"),
                bearer_token: Some(access_token),
                body: b"",
                connection_close: false,
            },
            Some(request.content_id),
        )
        .await
        {
            Ok(response) => response,
            Err(err)
                if is_transient_transport_error(err)
                    && attempt + 1 < PACKAGE_TRANSPORT_RETRY_ATTEMPTS =>
            {
                let _ = content_storage::abort_package_stage_traced(request_trace).await;
                close_reusable_session(reusable_session, "package retry").await;
                attempt += 1;
                info!(
                    "backend package retry content_id={} attempt={} err={:?}",
                    request.content_id.as_str(),
                    attempt,
                    err
                );
                Timer::after(Duration::from_millis(TRANSPORT_RETRY_BACKOFF_MS)).await;
                continue;
            }
            Err(err) => {
                let _ = content_storage::abort_package_stage_traced(request_trace).await;
                return Err(PackagePrepareError::Other(err));
            }
        };

        if response.status == 409 {
            let _ = content_storage::abort_package_stage_traced(request_trace).await;
            return Err(PackagePrepareError::PendingRemote);
        }
        if (400..500).contains(&response.status) {
            let _ = content_storage::abort_package_stage_traced(request_trace).await;
            return Err(PackagePrepareError::Rejected(response.status));
        }
        if response.status != 200 {
            let _ = content_storage::abort_package_stage_traced(request_trace).await;
            return Err(PackagePrepareError::Other(BackendError::InvalidResponse));
        }

        if let Some(progress_state) = response.prepare_progress {
            publish_event(
                Event::ContentPrepareProgress {
                    content_id: request.content_id,
                    progress: PrepareContentProgress {
                        phase: PrepareContentPhase::Caching,
                        completed_steps: 1 + progress_state.download_steps,
                        total_steps: progress_state.total_steps,
                    },
                },
                now_ms(),
            );
        }
        let result = content_storage::commit_package_stage_and_open_cached_reader_package_traced(
            request_trace,
            request.collection,
            request.remote_item_id,
            request.content_id,
        )
        .await
        .map_err(map_storage_prepare_error)?;
        if let Some(progress_state) = response.prepare_progress {
            publish_event(
                Event::ContentPrepareProgress {
                    content_id: request.content_id,
                    progress: PrepareContentProgress {
                        phase: PrepareContentPhase::Opening,
                        completed_steps: 2 + progress_state.download_steps,
                        total_steps: progress_state.total_steps,
                    },
                },
                now_ms(),
            );
        }

        if manifest_item_state(&result.snapshot, &request.remote_item_id)
            == Some(PackageState::PendingRemote)
        {
            return Err(PackagePrepareError::PendingRemote);
        }

        return Ok(result);
    }
}

async fn fetch_and_stage_package_over_session<T>(
    session: &mut Session<'_, T>,
    access_token: &str,
    request: PrepareContentRequest,
    sync_id: u32,
) -> Result<CollectionManifestState, PackagePrepareError>
where
    T: AsyncRead07 + AsyncWrite07,
{
    let path = build_package_path(request).map_err(PackagePrepareError::Other)?;
    let request_trace = next_request_trace(sync_id);
    content_storage::begin_package_stage_traced(
        request_trace,
        request.content_id,
        request.remote_revision,
    )
    .await
    .map_err(map_storage_prepare_error)?;

    let response = match stream_https_response_body_to_storage_over_session(
        session,
        HttpRequest {
            trace: request_trace,
            class: RequestClass::StreamingPackage,
            method: "GET",
            path: path.as_str(),
            content_type: Some("application/json"),
            bearer_token: Some(access_token),
            body: b"",
            connection_close: false,
        },
        None,
    )
    .await
    {
        Ok(response) => response,
        Err(err) => {
            let _ = content_storage::abort_package_stage_traced(request_trace).await;
            return Err(PackagePrepareError::Other(err));
        }
    };

    if response.status == 409 {
        let _ = content_storage::abort_package_stage_traced(request_trace).await;
        return Err(PackagePrepareError::PendingRemote);
    }
    if (400..500).contains(&response.status) {
        let _ = content_storage::abort_package_stage_traced(request_trace).await;
        return Err(PackagePrepareError::Rejected(response.status));
    }
    if response.status != 200 {
        let _ = content_storage::abort_package_stage_traced(request_trace).await;
        return Err(PackagePrepareError::Other(BackendError::InvalidResponse));
    }

    let snapshot = content_storage::commit_package_stage_traced(
        request_trace,
        request.collection,
        request.remote_item_id,
    )
    .await
    .map_err(map_storage_prepare_error)?;

    if manifest_item_state(&snapshot, &request.remote_item_id) == Some(PackageState::PendingRemote)
    {
        return Err(PackagePrepareError::PendingRemote);
    }

    Ok(snapshot)
}
async fn stream_https_response_body_to_storage(
    stack: Stack<'static>,
    tls: TlsReference<'_>,
    ca_chain: &Certificate<'static>,
    tcp_state: &BackendTcpClientState,
    tls_session_cache: &mut Option<SerializedClientSession>,
    request: HttpRequest<'_>,
    prepare_progress_content_id: Option<InlineText<{ domain::content::CONTENT_ID_MAX_BYTES }>>,
) -> Result<StreamingHttpResponse, BackendError> {
    let mut header_buffer = allocate_stream_header_buffer(request.path)?;
    let mut chunk_buffer = allocate_stream_chunk_buffer(request.path)?;
    let mut tcp_client = TcpClient::new(stack, tcp_state);
    tcp_client.set_timeout(Some(Duration::from_secs(
        request.class.socket_timeout_secs(),
    )));
    let ConnectedBackendSession {
        mut session,
        mut metrics,
        ..
    } = open_backend_session(
        stack,
        tls,
        ca_chain,
        &tcp_client,
        tls_session_cache,
        request.trace,
        request.path,
        request.class,
    )
    .await?;
    write_http_request(
        &mut session,
        request.class,
        request.path,
        request.method,
        request.content_type,
        request.bearer_token,
        request.body,
        request.connection_close,
    )
    .await?;

    let response = read_streaming_http_response_to_storage(
        &mut session,
        request.path,
        request.connection_close,
        &mut metrics,
        header_buffer.as_mut_slice(),
        chunk_buffer.as_mut_slice(),
        prepare_progress_content_id,
    )
    .await;
    close_backend_tls_session(&mut session, "stream request").await;

    match response {
        Ok(response) => {
            metrics.finish();
            log_request_timing(request, response.status, &metrics);
            Ok(response)
        }
        Err(err) => {
            log_request_heap(request.path, "stream failed");
            Err(err)
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn stream_https_response_body_to_storage_reusing_session<'a>(
    stack: Stack<'static>,
    tls: TlsReference<'a>,
    ca_chain: &Certificate<'static>,
    tcp_client: &'a BackendTcpClient<'a>,
    reusable_session: &mut Option<ReusableBackendSession<'a>>,
    tls_session_cache: &mut Option<SerializedClientSession>,
    request: HttpRequest<'_>,
    prepare_progress_content_id: Option<InlineText<{ domain::content::CONTENT_ID_MAX_BYTES }>>,
) -> Result<StreamingHttpResponse, BackendError> {
    let first_attempt = stream_https_response_body_to_storage_reusing_session_once(
        stack,
        tls,
        ca_chain,
        tcp_client,
        reusable_session,
        tls_session_cache,
        request,
        prepare_progress_content_id,
    )
    .await;
    match first_attempt {
        Err(err) if is_transient_transport_error(err) && TRANSPORT_RETRY_ATTEMPTS > 1 => {
            close_reusable_session(reusable_session, "stream retry").await;
            warn!(
                "backend request retry path={} attempt=1 err={:?}",
                request.path, err
            );
            Timer::after(Duration::from_millis(TRANSPORT_RETRY_BACKOFF_MS)).await;
            stream_https_response_body_to_storage_reusing_session_once(
                stack,
                tls,
                ca_chain,
                tcp_client,
                reusable_session,
                tls_session_cache,
                request,
                prepare_progress_content_id,
            )
            .await
        }
        other => other,
    }
}

#[allow(clippy::too_many_arguments)]
async fn stream_https_response_body_to_storage_reusing_session_once<'a>(
    stack: Stack<'static>,
    tls: TlsReference<'a>,
    ca_chain: &Certificate<'static>,
    tcp_client: &'a BackendTcpClient<'a>,
    reusable_session: &mut Option<ReusableBackendSession<'a>>,
    tls_session_cache: &mut Option<SerializedClientSession>,
    request: HttpRequest<'_>,
    prepare_progress_content_id: Option<InlineText<{ domain::content::CONTENT_ID_MAX_BYTES }>>,
) -> Result<StreamingHttpResponse, BackendError> {
    let mut header_buffer = allocate_stream_header_buffer(request.path)?;
    let mut chunk_buffer = allocate_stream_chunk_buffer(request.path)?;
    let metrics = ensure_reusable_session(
        stack,
        tls,
        ca_chain,
        tcp_client,
        reusable_session,
        tls_session_cache,
        request.trace,
        request.path,
        request.class,
    )
    .await?;
    info!(
        "backend request stream attempt path={} reused={} request_close={}",
        request.path, metrics.reused_session, request.connection_close
    );
    let response = match reusable_session {
        Some(reusable) => {
            stream_https_response_body_to_storage_over_session_with_metrics(
                &mut reusable.session,
                request,
                metrics,
                header_buffer.as_mut_slice(),
                chunk_buffer.as_mut_slice(),
                prepare_progress_content_id,
            )
            .await
        }
        None => unreachable!(),
    };

    update_reusable_session_after_streaming_request(reusable_session, request, &response).await;
    response
}

async fn stream_https_response_body_to_storage_over_session<T>(
    session: &mut Session<'_, T>,
    request: HttpRequest<'_>,
    prepare_progress_content_id: Option<InlineText<{ domain::content::CONTENT_ID_MAX_BYTES }>>,
) -> Result<StreamingHttpResponse, BackendError>
where
    T: AsyncRead07 + AsyncWrite07,
{
    let mut header_buffer = allocate_stream_header_buffer(request.path)?;
    let mut chunk_buffer = allocate_stream_chunk_buffer(request.path)?;
    stream_https_response_body_to_storage_over_session_with_metrics(
        session,
        request,
        RequestMetrics::new(request.trace, true, request.class),
        header_buffer.as_mut_slice(),
        chunk_buffer.as_mut_slice(),
        prepare_progress_content_id,
    )
    .await
}

async fn stream_https_response_body_to_storage_over_session_with_metrics<T>(
    session: &mut Session<'_, T>,
    request: HttpRequest<'_>,
    mut metrics: RequestMetrics,
    header: &mut [u8],
    chunk: &mut [u8],
    prepare_progress_content_id: Option<InlineText<{ domain::content::CONTENT_ID_MAX_BYTES }>>,
) -> Result<StreamingHttpResponse, BackendError>
where
    T: AsyncRead07 + AsyncWrite07,
{
    write_http_request(
        session,
        request.class,
        request.path,
        request.method,
        request.content_type,
        request.bearer_token,
        request.body,
        request.connection_close,
    )
    .await?;
    log_request_phase(
        metrics.trace,
        request.path,
        request.class,
        "request_sent",
        metrics.elapsed_ms(),
    );

    let response = read_streaming_http_response_to_storage(
        session,
        request.path,
        request.connection_close,
        &mut metrics,
        header,
        chunk,
        prepare_progress_content_id,
    )
    .await;
    match response {
        Ok(response) => {
            metrics.finish();
            log_request_timing(request, response.status, &metrics);
            Ok(response)
        }
        Err(err) => {
            log_request_heap(request.path, "stream failed");
            Err(err)
        }
    }
}

async fn read_streaming_http_response_to_storage<T>(
    session: &mut Session<'_, T>,
    path: &str,
    connection_close: bool,
    metrics: &mut RequestMetrics,
    header: &mut [u8],
    chunk: &mut [u8],
    prepare_progress_content_id: Option<InlineText<{ domain::content::CONTENT_ID_MAX_BYTES }>>,
) -> Result<StreamingHttpResponse, BackendError>
where
    T: AsyncRead07 + AsyncWrite07,
{
    let mut header_len = 0usize;
    let mut streamed_body_bytes = 0usize;
    let mut buffered_chunk_len = 0usize;
    let mut next_progress_log = STREAM_PROGRESS_LOG_INTERVAL_BYTES;
    let mut prepare_progress = prepare_progress_content_id.map(PrepareProgressReporter::new);
    metrics.stream_header_capacity = header.len();
    metrics.stream_header_headroom = header.len();

    loop {
        if header_len == header.len() {
            return Err(BackendError::ResponseTooLarge);
        }

        let read = await_body_io_timeout(
            path,
            metrics.class,
            "stream response header read",
            AsyncRead07::read(session, &mut header[header_len..]),
        )
        .await?;
        if read == 0 {
            return Err(BackendError::InvalidResponse);
        }
        metrics.mark_first_byte();
        header_len += read;

        let Some(header_end) = find_subslice(&header[..header_len], b"\r\n\r\n") else {
            continue;
        };
        let metadata = parse_http_response_metadata(&header[..header_len])?;
        let body_start = header_end + 4;
        let initial_body_len = header_len.saturating_sub(body_start);
        let response_reusable = is_response_connection_reusable(metadata);
        crate::verbose_diag!(
            "backend request stream headers path={} status={} header_bytes={} initial_body_bytes={} content_length={:?} chunked={} connection_close={} response_reusable={} storage_handoff_chunk_len={} elapsed_ms={}",
            path,
            metadata.status,
            metadata.body_start,
            initial_body_len,
            metadata.content_length,
            metadata.chunked,
            metadata.connection_close,
            response_reusable,
            PACKAGE_STORAGE_HANDOFF_CHUNK_LEN,
            metrics.elapsed_ms(),
        );
        crate::memtrace!(
            "request_headers",
            "component" = "backend",
            "at_ms" = now_ms(),
            "sync_id" = metrics.trace.sync_id,
            "req_id" = metrics.trace.req_id,
            "path" = path,
            "request_class" = metrics.class.label(),
            "streaming" = 1,
            "header_bytes" = metadata.body_start,
            "initial_body_bytes" = initial_body_len,
            "content_length_known" = bool_flag(metadata.content_length.is_some()),
            "content_length" = metadata.content_length.unwrap_or(0),
            "storage_handoff_chunk_len" = PACKAGE_STORAGE_HANDOFF_CHUNK_LEN,
            "response_buffer_capacity" = 0,
            "response_buffer_headroom" = 0,
            "connection_close" = bool_flag(connection_close),
        );
        if metadata.status != 200 {
            metrics.header_bytes = metadata.body_start;
            metrics.body_bytes = initial_body_len;
            metrics.response_bytes = header_len;
            metrics.stream_header_headroom = header.len().saturating_sub(header_len);
            if let Some(content_length) = metadata.content_length {
                metrics.content_length = content_length;
                metrics.content_length_known = true;
            }
            return Ok(StreamingHttpResponse {
                status: metadata.status,
                connection_reusable: false,
                prepare_progress: prepare_progress
                    .as_ref()
                    .map(PrepareProgressReporter::state),
            });
        }

        match metadata.content_length {
            Some(content_length) => {
                if let Some(progress) = prepare_progress.as_mut() {
                    progress.begin_download(Some(content_length));
                }
                if initial_body_len > content_length {
                    return Err(BackendError::InvalidResponse);
                }
                metrics.header_bytes = metadata.body_start;
                metrics.body_bytes = content_length;
                metrics.response_bytes = metadata.body_start.saturating_add(content_length);
                metrics.content_length = content_length;
                metrics.content_length_known = true;
                metrics.stream_header_headroom = header.len().saturating_sub(header_len);
                if initial_body_len > 0 {
                    chunk[..initial_body_len].copy_from_slice(&header[body_start..header_len]);
                    buffered_chunk_len = initial_body_len;
                    drain_buffered_package_chunks(
                        metrics.trace,
                        chunk,
                        &mut buffered_chunk_len,
                        PACKAGE_STORAGE_HANDOFF_CHUNK_LEN,
                    )
                    .await?;
                    streamed_body_bytes = initial_body_len;
                    log_stream_progress_if_needed(
                        metrics,
                        path,
                        &mut next_progress_log,
                        streamed_body_bytes,
                        Some(content_length),
                        metrics.elapsed_ms(),
                    );
                    if let Some(progress) = prepare_progress.as_mut() {
                        progress
                            .publish_download_progress(streamed_body_bytes, Some(content_length));
                    }
                }

                let mut remaining = content_length - initial_body_len;
                while remaining > 0 {
                    let read_len = remaining.min(chunk.len().saturating_sub(buffered_chunk_len));
                    let read = await_body_io_timeout_for(
                        path,
                        metrics.class,
                        "stream response body read",
                        Duration::from_secs(metrics.class.io_timeout_secs()),
                        AsyncRead07::read(
                            session,
                            &mut chunk[buffered_chunk_len..buffered_chunk_len + read_len],
                        ),
                    )
                    .await?;
                    if read == 0 {
                        return Err(BackendError::InvalidResponse);
                    }
                    buffered_chunk_len += read;
                    streamed_body_bytes = streamed_body_bytes.saturating_add(read);
                    remaining -= read;
                    log_stream_progress_if_needed(
                        metrics,
                        path,
                        &mut next_progress_log,
                        streamed_body_bytes,
                        Some(content_length),
                        metrics.elapsed_ms(),
                    );
                    if let Some(progress) = prepare_progress.as_mut() {
                        progress
                            .publish_download_progress(streamed_body_bytes, Some(content_length));
                    }
                    drain_buffered_package_chunks(
                        metrics.trace,
                        chunk,
                        &mut buffered_chunk_len,
                        PACKAGE_STORAGE_HANDOFF_CHUNK_LEN,
                    )
                    .await?;
                }
                flush_buffered_package_chunk(metrics.trace, chunk, &mut buffered_chunk_len).await?;
                log_stream_complete(
                    metrics,
                    path,
                    streamed_body_bytes,
                    Some(content_length),
                    response_reusable,
                    metrics.elapsed_ms(),
                );
                return Ok(StreamingHttpResponse {
                    status: 200,
                    connection_reusable: response_reusable,
                    prepare_progress: prepare_progress
                        .as_ref()
                        .map(PrepareProgressReporter::state),
                });
            }
            None if connection_close => {
                if let Some(progress) = prepare_progress.as_mut() {
                    progress.begin_download(None);
                }
                metrics.header_bytes = metadata.body_start;
                metrics.stream_header_headroom = header.len().saturating_sub(header_len);
                if initial_body_len > 0 {
                    chunk[..initial_body_len].copy_from_slice(&header[body_start..header_len]);
                    buffered_chunk_len = initial_body_len;
                    drain_buffered_package_chunks(
                        metrics.trace,
                        chunk,
                        &mut buffered_chunk_len,
                        PACKAGE_STORAGE_HANDOFF_CHUNK_LEN,
                    )
                    .await?;
                    streamed_body_bytes = initial_body_len;
                    log_stream_progress_if_needed(
                        metrics,
                        path,
                        &mut next_progress_log,
                        streamed_body_bytes,
                        None,
                        metrics.elapsed_ms(),
                    );
                    if let Some(progress) = prepare_progress.as_mut() {
                        progress.publish_download_progress(streamed_body_bytes, None);
                    }
                }
                break;
            }
            None => return Err(BackendError::InvalidResponse),
        }
    }

    loop {
        if buffered_chunk_len == chunk.len() {
            drain_buffered_package_chunks(
                metrics.trace,
                chunk,
                &mut buffered_chunk_len,
                PACKAGE_STORAGE_HANDOFF_CHUNK_LEN,
            )
            .await?;
            if buffered_chunk_len == chunk.len() {
                flush_buffered_package_chunk(metrics.trace, chunk, &mut buffered_chunk_len).await?;
            }
        }
        let read = await_body_io_timeout_for(
            path,
            metrics.class,
            "stream response body read",
            Duration::from_secs(metrics.class.io_timeout_secs()),
            AsyncRead07::read(session, &mut chunk[buffered_chunk_len..]),
        )
        .await?;
        if read == 0 {
            break;
        }
        buffered_chunk_len += read;
        streamed_body_bytes = streamed_body_bytes.saturating_add(read);
        log_stream_progress_if_needed(
            metrics,
            path,
            &mut next_progress_log,
            streamed_body_bytes,
            None,
            metrics.elapsed_ms(),
        );
        if let Some(progress) = prepare_progress.as_mut() {
            progress.publish_download_progress(streamed_body_bytes, None);
        }
        drain_buffered_package_chunks(
            metrics.trace,
            chunk,
            &mut buffered_chunk_len,
            PACKAGE_STORAGE_HANDOFF_CHUNK_LEN,
        )
        .await?;
    }
    flush_buffered_package_chunk(metrics.trace, chunk, &mut buffered_chunk_len).await?;

    metrics.body_bytes = streamed_body_bytes;
    metrics.response_bytes = metrics.header_bytes.saturating_add(streamed_body_bytes);
    log_stream_complete(
        metrics,
        path,
        streamed_body_bytes,
        None,
        false,
        metrics.elapsed_ms(),
    );
    Ok(StreamingHttpResponse {
        status: 200,
        connection_reusable: false,
        prepare_progress: prepare_progress
            .as_ref()
            .map(PrepareProgressReporter::state),
    })
}

async fn drain_buffered_package_chunks(
    trace: TraceContext,
    chunk: &mut [u8],
    buffered_len: &mut usize,
    storage_handoff_chunk_len: usize,
) -> Result<(), BackendError> {
    if storage_handoff_chunk_len == 0 {
        return Ok(());
    }

    while *buffered_len >= storage_handoff_chunk_len {
        write_buffered_package_prefix(trace, chunk, buffered_len, storage_handoff_chunk_len)
            .await?;
    }
    Ok(())
}

async fn flush_buffered_package_chunk(
    trace: TraceContext,
    chunk: &mut [u8],
    buffered_len: &mut usize,
) -> Result<(), BackendError> {
    if *buffered_len == 0 {
        return Ok(());
    }
    content_storage::write_package_chunk_traced(trace, &chunk[..*buffered_len])
        .await
        .map_err(map_storage_backend_error)?;
    *buffered_len = 0;
    Ok(())
}

async fn write_buffered_package_prefix(
    trace: TraceContext,
    chunk: &mut [u8],
    buffered_len: &mut usize,
    write_len: usize,
) -> Result<(), BackendError> {
    if *buffered_len < write_len {
        return Ok(());
    }

    content_storage::write_package_chunk_traced(trace, &chunk[..write_len])
        .await
        .map_err(map_storage_backend_error)?;
    consume_buffered_chunk_prefix(chunk, buffered_len, write_len);
    Ok(())
}

fn consume_buffered_chunk_prefix(chunk: &mut [u8], buffered_len: &mut usize, consumed_len: usize) {
    let consumed_len = consumed_len.min(*buffered_len);
    let remaining_len = (*buffered_len).saturating_sub(consumed_len);
    if remaining_len > 0 {
        chunk.copy_within(consumed_len..*buffered_len, 0);
    }
    *buffered_len = remaining_len;
}

fn parse_http_status(response: &[u8]) -> Result<u16, BackendError> {
    let status_line_end = find_subslice(response, b"\r\n").ok_or(BackendError::InvalidResponse)?;
    let status_line =
        str::from_utf8(&response[..status_line_end]).map_err(|_| BackendError::InvalidUtf8)?;
    let mut parts = status_line.splitn(3, ' ');
    let _http = parts.next().ok_or(BackendError::InvalidResponse)?;
    parts
        .next()
        .ok_or(BackendError::InvalidResponse)?
        .parse::<u16>()
        .map_err(|_| BackendError::InvalidResponse)
}

async fn perform_saved_content_fetch_over_session<T>(
    session: &mut Session<'_, T>,
    access_token: &str,
    connection_close: bool,
    sync_id: u32,
) -> Result<CollectionFetchResult, CollectionQueryError>
where
    T: AsyncRead07 + AsyncWrite07,
{
    perform_saved_content_fetch_paginated_over_session(
        session,
        access_token,
        connection_close,
        sync_id,
    )
    .await
}

async fn publish_package_state(
    trace: TraceContext,
    collection: CollectionKind,
    remote_item_id: InlineText<REMOTE_ITEM_ID_MAX_BYTES>,
    package_state: PackageState,
) -> Result<(), StorageError> {
    match content_storage::update_package_state_traced(
        trace,
        collection,
        remote_item_id,
        package_state,
    )
    .await
    {
        Ok(snapshot) => {
            publish_event(
                Event::CollectionContentUpdated(collection, Box::new(snapshot)),
                now_ms(),
            );
            Ok(())
        }
        Err(err) => {
            publish_event(
                Event::ContentPackageStateChanged {
                    collection,
                    remote_item_id,
                    package_state,
                },
                now_ms(),
            );
            Err(err)
        }
    }
}

fn map_storage_prepare_error(error: StorageError) -> PackagePrepareError {
    PackagePrepareError::Other(map_storage_backend_error(error))
}

fn map_storage_backend_error(_error: StorageError) -> BackendError {
    BackendError::Io
}

fn prepare_error_package_state(error: BackendError) -> PackageState {
    if is_transient_transport_error(error) {
        PackageState::Missing
    } else {
        PackageState::Failed
    }
}

fn build_package_path(
    request: PrepareContentRequest,
) -> Result<heapless::String<128>, BackendError> {
    let mut path = heapless::String::<128>::new();
    match request.detail_locator {
        DetailLocator::Saved => {
            path.push_str("/device/v1/me/saved-content/")
                .map_err(|_| BackendError::ResponseTooLarge)?;
            path.push_str(request.remote_item_id.as_str())
                .map_err(|_| BackendError::ResponseTooLarge)?;
        }
        DetailLocator::Inbox => {
            path.push_str("/device/v1/me/inbox/")
                .map_err(|_| BackendError::ResponseTooLarge)?;
            path.push_str(request.remote_item_id.as_str())
                .map_err(|_| BackendError::ResponseTooLarge)?;
        }
        DetailLocator::Content => {
            path.push_str("/device/v1/me/content/")
                .map_err(|_| BackendError::ResponseTooLarge)?;
            path.push_str(request.content_id.as_str())
                .map_err(|_| BackendError::ResponseTooLarge)?;
        }
    }
    path.push_str("/package")
        .map_err(|_| BackendError::ResponseTooLarge)?;

    Ok(path)
}

fn manifest_item_state(
    snapshot: &CollectionManifestState,
    remote_item_id: &InlineText<REMOTE_ITEM_ID_MAX_BYTES>,
) -> Option<PackageState> {
    let mut index = 0;
    while index < snapshot.len() {
        if snapshot.items[index].remote_item_id == *remote_item_id {
            return Some(snapshot.items[index].package_state);
        }
        index += 1;
    }

    None
}

fn build_refresh_body(
    refresh_token: &str,
) -> Result<heapless::String<REQUEST_BODY_MAX_LEN>, BackendError> {
    let mut body = heapless::String::<REQUEST_BODY_MAX_LEN>::new();
    body.push_str("{\"refresh_token\":\"")
        .map_err(|_| BackendError::ResponseTooLarge)?;
    append_json_escaped(&mut body, refresh_token)?;
    body.push_str("\"}")
        .map_err(|_| BackendError::ResponseTooLarge)?;
    Ok(body)
}

fn append_json_escaped(
    out: &mut heapless::String<REQUEST_BODY_MAX_LEN>,
    value: &str,
) -> Result<(), BackendError> {
    for ch in value.chars() {
        match ch {
            '"' => out
                .push_str("\\\"")
                .map_err(|_| BackendError::ResponseTooLarge)?,
            '\\' => out
                .push_str("\\\\")
                .map_err(|_| BackendError::ResponseTooLarge)?,
            '\n' => out
                .push_str("\\n")
                .map_err(|_| BackendError::ResponseTooLarge)?,
            '\r' => out
                .push_str("\\r")
                .map_err(|_| BackendError::ResponseTooLarge)?,
            '\t' => out
                .push_str("\\t")
                .map_err(|_| BackendError::ResponseTooLarge)?,
            ch if ch.is_control() => return Err(BackendError::InvalidResponse),
            ch => out.push(ch).map_err(|_| BackendError::ResponseTooLarge)?,
        }
    }

    Ok(())
}

fn compile_time_refresh_token() -> Option<BackendCredential> {
    let refresh_token = option_env!("MOTIF_BACKEND_REFRESH_TOKEN")?.trim();
    if refresh_token.is_empty() {
        return None;
    }

    match BackendCredential::from_refresh_token(refresh_token) {
        Ok(credential) => Some(credential),
        Err(_) => {
            info!("backend compile-time token ignored: invalid format");
            None
        }
    }
}

fn select_startup_credential(
    compile_time: Option<BackendCredential>,
    stored: Option<BackendCredential>,
) -> Option<StartupCredential> {
    if let Some(credential) = stored {
        return Some(StartupCredential {
            credential,
            source: CredentialSource::Stored,
        });
    }

    if let Some(credential) = compile_time {
        return Some(StartupCredential {
            credential,
            source: CredentialSource::CompileTime,
        });
    }

    None
}

fn backend_host_cstr() -> &'static CStr {
    unsafe { CStr::from_bytes_with_nul_unchecked(BACKEND_HOST_CSTR_BYTES) }
}

fn backend_ca_chain() -> Result<Certificate<'static>, BackendError> {
    let pem = CStr::from_bytes_with_nul(BACKEND_CA_CHAIN_PEM.as_bytes())
        .map_err(|_| BackendError::InvalidResponse)?;
    Certificate::new(X509::PEM(pem)).map_err(|_| BackendError::InvalidResponse)
}

fn find_json_value_start(json: &str, key: &str) -> Option<usize> {
    let key_pos = json.find(key)?;
    let bytes = json.as_bytes();
    let mut index = key_pos + key.len();

    while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
        index += 1;
    }
    if *bytes.get(index)? != b':' {
        return None;
    }
    index += 1;
    while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
        index += 1;
    }

    Some(index)
}

fn extract_json_string<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let bytes = json.as_bytes();
    let mut index = find_json_value_start(json, key)?;
    if *bytes.get(index)? != b'"' {
        return None;
    }
    index += 1;
    let start = index;

    while let Some(&byte) = bytes.get(index) {
        match byte {
            b'\\' => return None,
            b'"' => return Some(&json[start..index]),
            _ => index += 1,
        }
    }

    None
}

fn extract_json_u64(json: &str, key: &str) -> Option<u64> {
    let bytes = json.as_bytes();
    let mut index = find_json_value_start(json, key)?;

    let start = index;
    while bytes.get(index).is_some_and(u8::is_ascii_digit) {
        index += 1;
    }

    if start == index {
        return None;
    }

    json[start..index].parse().ok()
}

fn extract_json_optional_string<'a>(json: &'a str, key: &str) -> Option<Option<&'a str>> {
    let bytes = json.as_bytes();
    let mut index = find_json_value_start(json, key)?;

    match *bytes.get(index)? {
        b'n' if bytes.get(index..index + 4) == Some(b"null") => Some(None),
        b'"' => {
            index += 1;
            let start = index;

            while let Some(&byte) = bytes.get(index) {
                match byte {
                    b'\\' => return None,
                    b'"' => return Some(Some(&json[start..index])),
                    _ => index += 1,
                }
            }

            None
        }
        _ => None,
    }
}

fn extract_json_array_len(json: &str, key: &str) -> Option<usize> {
    let bytes = json.as_bytes();
    let mut index = find_json_value_start(json, key)?;
    if *bytes.get(index)? != b'[' {
        return None;
    }
    index += 1;

    let mut count = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    let mut object_depth = 0usize;
    let mut array_depth = 0usize;
    let mut saw_top_level_value = false;

    while let Some(&byte) = bytes.get(index) {
        if in_string {
            match byte {
                b'\\' if !escaped => escaped = true,
                b'"' if !escaped => in_string = false,
                _ => escaped = false,
            }
            index += 1;
            continue;
        }

        match byte {
            b'"' => {
                in_string = true;
                escaped = false;
                if object_depth == 0 && array_depth == 0 {
                    saw_top_level_value = true;
                }
            }
            b'{' => {
                if object_depth == 0 && array_depth == 0 {
                    saw_top_level_value = true;
                }
                object_depth += 1;
            }
            b'}' => {
                object_depth = object_depth.checked_sub(1)?;
            }
            b'[' => {
                if object_depth == 0 && array_depth == 0 {
                    saw_top_level_value = true;
                }
                array_depth += 1;
            }
            b']' => {
                if object_depth == 0 && array_depth == 0 {
                    if saw_top_level_value {
                        count += 1;
                    }
                    return Some(count);
                }
                array_depth = array_depth.checked_sub(1)?;
            }
            b',' => {
                if object_depth == 0 && array_depth == 0 && saw_top_level_value {
                    count += 1;
                    saw_top_level_value = false;
                }
            }
            byte if !byte.is_ascii_whitespace() && object_depth == 0 && array_depth == 0 => {
                saw_top_level_value = true;
            }
            _ => {}
        }

        index += 1;
    }

    None
}

fn parse_collection_fetch_summary(
    body: &str,
    array_key: &str,
) -> Result<CollectionFetchSummary, BackendError> {
    let item_count = extract_json_array_len(body, array_key).ok_or(BackendError::MissingField)?;
    let next_cursor_present = extract_json_optional_string(body, "\"next_cursor\"")
        .ok_or(BackendError::MissingField)?
        .is_some();
    let (body_preview, body_preview_truncated) = collection_body_preview(body, item_count)?;

    Ok(CollectionFetchSummary {
        trace: TraceContext::none(),
        item_count,
        next_cursor_present,
        page_count: 1,
        body_bytes_total: body.len(),
        truncated_by_capacity: false,
        body_preview,
        body_preview_truncated,
    })
}

fn parse_inbox_fetch_result(body: &str) -> Result<CollectionFetchResult, BackendError> {
    let page = parse_inbox_fetch_page(body)?;
    let next_cursor_present = page.next_cursor.is_some();
    let item_count = page.collection.len();

    Ok(CollectionFetchResult {
        summary: CollectionFetchSummary {
            trace: TraceContext::none(),
            item_count,
            next_cursor_present,
            page_count: 1,
            body_bytes_total: body.len(),
            truncated_by_capacity: false,
            body_preview: page.body_preview,
            body_preview_truncated: page.body_preview_truncated,
        },
        collection: page.collection,
    })
}

fn parse_saved_content_fetch_result(body: &str) -> Result<CollectionFetchResult, BackendError> {
    let page = parse_saved_content_fetch_page(body)?;
    let next_cursor_present = page.next_cursor.is_some();
    let item_count = page.collection.len();

    Ok(CollectionFetchResult {
        summary: CollectionFetchSummary {
            trace: TraceContext::none(),
            item_count,
            next_cursor_present,
            page_count: 1,
            body_bytes_total: body.len(),
            truncated_by_capacity: false,
            body_preview: page.body_preview,
            body_preview_truncated: page.body_preview_truncated,
        },
        collection: page.collection,
    })
}

fn parse_recommendation_fetch_result(body: &str) -> Result<CollectionFetchResult, BackendError> {
    let page = parse_recommendation_fetch_page(body)?;
    let next_cursor_present = page.next_cursor.is_some();
    let item_count = page.collection.len();

    Ok(CollectionFetchResult {
        summary: CollectionFetchSummary {
            trace: TraceContext::none(),
            item_count,
            next_cursor_present,
            page_count: 1,
            body_bytes_total: body.len(),
            truncated_by_capacity: false,
            body_preview: page.body_preview,
            body_preview_truncated: page.body_preview_truncated,
        },
        collection: page.collection,
    })
}

fn parse_saved_content_collection(body: &str) -> Result<CollectionManifestState, BackendError> {
    let items = extract_json_top_level_array_items::<MANIFEST_ITEM_CAPACITY>(body, "\"content\"")?;
    let mut collection = CollectionManifestState::empty();

    let mut index = 0;
    while index < items.len() {
        if let Some(item_json) = items[index] {
            let _ = collection.try_push(parse_saved_article_manifest(item_json)?);
        }
        index += 1;
    }

    Ok(collection)
}

fn parse_inbox_collection(body: &str) -> Result<CollectionManifestState, BackendError> {
    let items = extract_json_top_level_array_items::<MANIFEST_ITEM_CAPACITY>(body, "\"inbox\"")?;
    let mut collection = CollectionManifestState::empty();

    let mut index = 0;
    while index < items.len() {
        if let Some(item_json) = items[index] {
            let _ = collection.try_push(parse_inbox_article_manifest(item_json)?);
        }
        index += 1;
    }

    Ok(collection)
}

fn parse_recommendation_collection(body: &str) -> Result<CollectionManifestState, BackendError> {
    let items = extract_json_top_level_array_items::<MANIFEST_ITEM_CAPACITY>(body, "\"content\"")?;
    let mut collection = CollectionManifestState::empty();
    if let Some(serve_id) = extract_json_optional_inline_text::<RECOMMENDATION_SERVE_ID_MAX_BYTES>(
        body,
        "\"serve_id\"",
    )? {
        collection.serve_id = serve_id;
    }

    let mut index = 0;
    while index < items.len() {
        if let Some(item_json) = items[index] {
            let _ = collection.try_push(parse_recommendation_manifest(item_json)?);
        }
        index += 1;
    }

    Ok(collection)
}

fn parse_saved_article_manifest(item_json: &str) -> Result<CollectionManifestItem, BackendError> {
    let backend_id = extract_json_string(item_json, "\"id\"").ok_or(BackendError::MissingField)?;
    let submitted_url =
        extract_json_string(item_json, "\"submitted_url\"").ok_or(BackendError::MissingField)?;
    let content_json =
        extract_json_object_slice(item_json, "\"content\"").ok_or(BackendError::MissingField)?;
    let content_id =
        extract_json_string(content_json, "\"id\"").ok_or(BackendError::MissingField)?;
    let host = extract_json_string(content_json, "\"host\"").ok_or(BackendError::MissingField)?;
    let site_name =
        extract_json_optional_inline_text::<CONTENT_META_MAX_BYTES>(content_json, "\"site_name\"")?
            .filter(|value| !value.is_empty());
    let title =
        extract_json_optional_inline_text::<CONTENT_TITLE_MAX_BYTES>(content_json, "\"title\"")?
            .filter(|value| !value.is_empty());
    let remote_status = extract_remote_status(content_json)?;
    let remote_revision = extract_remote_revision(content_json);

    let mut manifest = CollectionManifestItem::empty();
    manifest.remote_item_id.set_truncated(backend_id);
    manifest.content_id.set_truncated(content_id);
    manifest.detail_locator = DetailLocator::Saved;
    manifest.source = domain::source::SourceKind::PersonalQueue;
    manifest.remote_status = remote_status;
    manifest.remote_revision = remote_revision;

    if let Some(site_name) = site_name {
        set_collection_meta(&mut manifest.meta, site_name.as_str(), "SAVED");
    } else {
        set_collection_meta(&mut manifest.meta, host, "SAVED");
    }

    if let Some(title) = title {
        manifest.title = title;
    } else {
        manifest.title.set_truncated(submitted_url);
    }

    Ok(manifest)
}

fn parse_inbox_article_manifest(item_json: &str) -> Result<CollectionManifestItem, BackendError> {
    let inbox_id = extract_json_string(item_json, "\"id\"").ok_or(BackendError::MissingField)?;
    let content_json =
        extract_json_object_slice(item_json, "\"content\"").ok_or(BackendError::MissingField)?;
    let content_id =
        extract_json_string(content_json, "\"id\"").ok_or(BackendError::MissingField)?;
    let source_json =
        extract_json_object_slice(item_json, "\"source\"").ok_or(BackendError::MissingField)?;
    let host = extract_json_string(content_json, "\"host\"").ok_or(BackendError::MissingField)?;
    let source_title =
        extract_json_optional_inline_text::<CONTENT_META_MAX_BYTES>(source_json, "\"title\"")?
            .filter(|value| !value.is_empty());
    let title =
        extract_json_optional_inline_text::<CONTENT_TITLE_MAX_BYTES>(content_json, "\"title\"")?
            .filter(|value| !value.is_empty());
    let remote_status = extract_remote_status(content_json)?;
    let remote_revision =
        extract_remote_revision(item_json).max(extract_remote_revision(content_json));

    let mut manifest = CollectionManifestItem::empty();
    manifest.remote_item_id.set_truncated(inbox_id);
    manifest.content_id.set_truncated(content_id);
    manifest.detail_locator = DetailLocator::Inbox;
    manifest.source = domain::source::SourceKind::EditorialFeed;
    manifest.remote_status = remote_status;
    manifest.remote_revision = remote_revision;

    if let Some(source_title) = source_title {
        set_collection_meta(&mut manifest.meta, source_title.as_str(), "INBOX");
    } else {
        set_collection_meta(&mut manifest.meta, host, "INBOX");
    }

    if let Some(title) = title {
        manifest.title = title;
    } else {
        manifest.title.set_truncated(host);
    }

    Ok(manifest)
}

fn parse_recommendation_manifest(item_json: &str) -> Result<CollectionManifestItem, BackendError> {
    let content_json =
        extract_json_object_slice(item_json, "\"content\"").ok_or(BackendError::MissingField)?;
    let content_id =
        extract_json_string(content_json, "\"id\"").ok_or(BackendError::MissingField)?;
    let source_json =
        extract_json_object_slice(item_json, "\"source\"").ok_or(BackendError::MissingField)?;
    let host = extract_json_string(content_json, "\"host\"").ok_or(BackendError::MissingField)?;
    let source_title =
        extract_json_optional_inline_text::<CONTENT_META_MAX_BYTES>(source_json, "\"title\"")?
            .filter(|value| !value.is_empty());
    let title =
        extract_json_optional_inline_text::<CONTENT_TITLE_MAX_BYTES>(content_json, "\"title\"")?
            .filter(|value| !value.is_empty());
    let remote_status = extract_remote_status(content_json)?;
    let remote_revision = extract_remote_revision(content_json);

    let mut manifest = CollectionManifestItem::empty();
    manifest.remote_item_id.set_truncated(content_id);
    manifest.content_id.set_truncated(content_id);
    manifest.detail_locator = DetailLocator::Content;
    manifest.source = domain::source::SourceKind::EditorialFeed;
    manifest.remote_status = remote_status;
    manifest.remote_revision = remote_revision;

    if let Some(source_title) = source_title {
        set_collection_meta(&mut manifest.meta, source_title.as_str(), "FOR YOU");
    } else {
        set_collection_meta(&mut manifest.meta, host, "FOR YOU");
    }

    if let Some(title) = title {
        manifest.title = title;
    } else {
        manifest.title.set_truncated(host);
    }

    Ok(manifest)
}

fn extract_json_top_level_array_items<'a, const N: usize>(
    json: &'a str,
    key: &str,
) -> Result<[Option<&'a str>; N], BackendError> {
    let bytes = json.as_bytes();
    let mut index = find_json_value_start(json, key).ok_or(BackendError::MissingField)?;
    if *bytes.get(index).ok_or(BackendError::MissingField)? != b'[' {
        return Err(BackendError::MissingField);
    }
    index += 1;

    let mut items = [None; N];
    let mut count = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    let mut object_depth = 0usize;
    let mut array_depth = 0usize;
    let mut item_start: Option<usize> = None;

    while let Some(&byte) = bytes.get(index) {
        if in_string {
            match byte {
                b'\\' if !escaped => escaped = true,
                b'"' if !escaped => in_string = false,
                _ => escaped = false,
            }
            index += 1;
            continue;
        }

        match byte {
            b'"' => {
                in_string = true;
                escaped = false;
            }
            b'{' => {
                if object_depth == 0 && array_depth == 0 {
                    item_start = Some(index);
                }
                object_depth += 1;
            }
            b'}' => {
                object_depth = object_depth
                    .checked_sub(1)
                    .ok_or(BackendError::InvalidResponse)?;
                if object_depth == 0 && array_depth == 0 {
                    let start = item_start.take().ok_or(BackendError::InvalidResponse)?;
                    if count < N {
                        items[count] = Some(&json[start..=index]);
                    }
                    count = count.saturating_add(1);
                }
            }
            b'[' => {
                if object_depth > 0 || array_depth > 0 {
                    array_depth += 1;
                }
            }
            b']' => {
                if object_depth == 0 && array_depth == 0 {
                    return Ok(items);
                }
                array_depth = array_depth
                    .checked_sub(1)
                    .ok_or(BackendError::InvalidResponse)?;
            }
            _ => {}
        }

        index += 1;
    }

    Err(BackendError::InvalidResponse)
}

fn extract_json_object_slice<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let bytes = json.as_bytes();
    let mut index = find_json_value_start(json, key)?;
    if *bytes.get(index)? != b'{' {
        return None;
    }

    let start = index;
    let mut in_string = false;
    let mut escaped = false;
    let mut depth = 0usize;

    while let Some(&byte) = bytes.get(index) {
        if in_string {
            match byte {
                b'\\' if !escaped => escaped = true,
                b'"' if !escaped => in_string = false,
                _ => escaped = false,
            }
            index += 1;
            continue;
        }

        match byte {
            b'"' => {
                in_string = true;
                escaped = false;
            }
            b'{' => depth += 1,
            b'}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(&json[start..=index]);
                }
            }
            _ => {}
        }

        index += 1;
    }

    None
}

fn extract_json_string_raw<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let bytes = json.as_bytes();
    let mut index = find_json_value_start(json, key)?;
    if *bytes.get(index)? != b'"' {
        return None;
    }
    index += 1;
    let start = index;
    let mut escaped = false;

    while let Some(&byte) = bytes.get(index) {
        match byte {
            b'\\' if !escaped => escaped = true,
            b'"' if !escaped => return Some(&json[start..index]),
            _ => escaped = false,
        }
        index += 1;
    }

    None
}

fn extract_json_optional_string_raw<'a>(json: &'a str, key: &str) -> Option<Option<&'a str>> {
    let bytes = json.as_bytes();
    let index = find_json_value_start(json, key)?;

    match *bytes.get(index)? {
        b'n' if bytes.get(index..index + 4) == Some(b"null") => Some(None),
        b'"' => extract_json_string_raw(json, key).map(Some),
        _ => None,
    }
}

fn extract_json_optional_inline_text<const N: usize>(
    json: &str,
    key: &str,
) -> Result<Option<domain::text::InlineText<N>>, BackendError> {
    let Some(raw) = extract_json_optional_string_raw(json, key) else {
        return Ok(None);
    };

    raw.map(decode_json_string::<N>).transpose()
}

fn decode_json_string<const N: usize>(
    raw: &str,
) -> Result<domain::text::InlineText<N>, BackendError> {
    let mut output = domain::text::InlineText::<N>::new();
    let mut chars = raw.chars();

    while let Some(ch) = chars.next() {
        if ch != '\\' {
            if !output.try_push_char(ch) {
                break;
            }
            continue;
        }

        let decoded = match chars.next().ok_or(BackendError::InvalidResponse)? {
            '"' => '"',
            '\\' => '\\',
            '/' => '/',
            'b' => '\u{0008}',
            'f' => '\u{000C}',
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            'u' => return Err(BackendError::InvalidResponse),
            _ => return Err(BackendError::InvalidResponse),
        };

        if !output.try_push_char(decoded) {
            break;
        }
    }

    Ok(output)
}

fn extract_remote_status(json: &str) -> Result<RemoteContentStatus, BackendError> {
    let fetch_status =
        extract_json_string(json, "\"fetch_status\"").ok_or(BackendError::MissingField)?;
    let parse_status =
        extract_json_string(json, "\"parse_status\"").ok_or(BackendError::MissingField)?;

    if matches!(fetch_status, "failed") || matches!(parse_status, "failed") {
        return Ok(RemoteContentStatus::Failed);
    }
    if matches!(fetch_status, "succeeded") && matches!(parse_status, "succeeded") {
        return Ok(RemoteContentStatus::Ready);
    }

    Ok(RemoteContentStatus::Pending)
}

fn extract_remote_revision(json: &str) -> u64 {
    extract_json_u64(json, "\"parsed_at\"")
        .or_else(|| extract_json_u64(json, "\"updated_at\""))
        .unwrap_or(0)
}

fn set_collection_meta<const N: usize>(
    target: &mut domain::text::InlineText<N>,
    source: &str,
    suffix: &str,
) {
    target.clear();

    for ch in source.chars() {
        if !target.try_push_char(ch.to_ascii_uppercase()) {
            return;
        }
    }

    let _ = target.try_push_str(" / ");
    let _ = target.try_push_str(suffix);
}

fn utf8_log_prefix(value: &str, max_len: usize) -> (&str, bool) {
    if value.len() <= max_len {
        return (value, false);
    }

    let mut boundary = max_len;
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }

    (&value[..boundary], true)
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn bounded_string<const N: usize>(value: &str) -> Result<heapless::String<N>, BackendError> {
    let mut output = heapless::String::<N>::new();
    output
        .push_str(value)
        .map_err(|_| BackendError::ResponseTooLarge)?;
    Ok(output)
}

fn log_status(status: SyncStatus) {
    info!("backend status={:?}", status);
    publish_event(Event::BackendSyncStatusChanged(status), now_ms());
}

fn log_heap(label: &str) {
    let stats = crate::telemetry::capture_heap();
    info!(
        "heap label={} size={} used={} free={} internal_size={} internal_used={} internal_free={} internal_peak_used={} internal_min_free={} external_size={} external_used={} external_free={} external_peak_used={} external_min_free={}",
        label,
        stats.size,
        stats.used,
        stats.free,
        stats.internal_size,
        stats.internal_used,
        stats.internal_free,
        stats.internal_peak_used,
        stats.internal_min_free,
        stats.external_size,
        stats.external_used,
        stats.external_free,
        stats.external_peak_used,
        stats.external_min_free,
    );
    info!(
        "heap regions label={} region0_kind={} region0_used={} region0_free={} region0_peak_used={} region0_min_free={} region1_kind={} region1_used={} region1_free={} region1_peak_used={} region1_min_free={} region2_kind={} region2_used={} region2_free={} region2_peak_used={} region2_min_free={}",
        label,
        stats.regions[0].kind,
        stats.regions[0].used,
        stats.regions[0].free,
        stats.regions[0].peak_used,
        stats.regions[0].min_free,
        stats.regions[1].kind,
        stats.regions[1].used,
        stats.regions[1].free,
        stats.regions[1].peak_used,
        stats.regions[1].min_free,
        stats.regions[2].kind,
        stats.regions[2].used,
        stats.regions[2].free,
        stats.regions[2].peak_used,
        stats.regions[2].min_free,
    );
}

fn log_request_heap(_path: &str, _stage: &str) {
    #[cfg(not(feature = "telemetry-verbose-diagnostics"))]
    let _ = (_path, _stage);

    #[cfg(feature = "telemetry-verbose-diagnostics")]
    {
        let stats = crate::telemetry::capture_heap();
        info!(
            "backend heap path={} stage={} size={} used={} free={} internal_size={} internal_used={} internal_free={} internal_peak_used={} internal_min_free={} external_size={} external_used={} external_free={} external_peak_used={} external_min_free={}",
            _path,
            _stage,
            stats.size,
            stats.used,
            stats.free,
            stats.internal_size,
            stats.internal_used,
            stats.internal_free,
            stats.internal_peak_used,
            stats.internal_min_free,
            stats.external_size,
            stats.external_used,
            stats.external_free,
            stats.external_peak_used,
            stats.external_min_free,
        );
        info!(
            "backend heap regions path={} stage={} region0_kind={} region0_used={} region0_free={} region0_peak_used={} region0_min_free={} region1_kind={} region1_used={} region1_free={} region1_peak_used={} region1_min_free={} region2_kind={} region2_used={} region2_free={} region2_peak_used={} region2_min_free={}",
            _path,
            _stage,
            stats.regions[0].kind,
            stats.regions[0].used,
            stats.regions[0].free,
            stats.regions[0].peak_used,
            stats.regions[0].min_free,
            stats.regions[1].kind,
            stats.regions[1].used,
            stats.regions[1].free,
            stats.regions[1].peak_used,
            stats.regions[1].min_free,
            stats.regions[2].kind,
            stats.regions[2].used,
            stats.regions[2].free,
            stats.regions[2].peak_used,
            stats.regions[2].min_free,
        );
    }
}

fn now_ms() -> u64 {
    Instant::now().as_millis()
}

fn elapsed_since_ms(started_ms: u64) -> u64 {
    now_ms().saturating_sub(started_ms)
}

fn log_request_phase(
    trace: TraceContext,
    path: &str,
    class: RequestClass,
    phase: &str,
    elapsed_ms: u64,
) {
    crate::memtrace!(
        "request_phase",
        "component" = "backend",
        "at_ms" = now_ms(),
        "sync_id" = trace.sync_id,
        "req_id" = trace.req_id,
        "path" = path,
        "request_class" = class.label(),
        "phase" = phase,
        "streaming" = bool_flag(class.is_streaming()),
        "elapsed_ms" = elapsed_ms,
    );
}

fn reusable_session_discard_reason(
    stack: Stack<'static>,
    session: &ReusableBackendSession<'_>,
    now_ms: u64,
    streaming: bool,
) -> &'static str {
    if !stack.is_link_up() {
        return "link_down";
    }

    match current_network_address(stack) {
        None => "missing_ip",
        Some(current) if current != session.network_address => "network_changed",
        Some(_) => {
            if !is_reusable_session_age_usable(session.last_used_ms, now_ms, streaming) {
                "idle_timeout"
            } else {
                "unknown"
            }
        }
    }
}

fn reusable_session_idle_reap_timeout_ms() -> u64 {
    Duration::from_secs(REUSABLE_BUFFERED_SESSION_IDLE_TIMEOUT_SECS).as_millis()
}

fn reusable_session_idle_timeout_ms(streaming: bool) -> u64 {
    let seconds = if streaming {
        REUSABLE_STREAMING_SESSION_IDLE_TIMEOUT_SECS
    } else {
        REUSABLE_BUFFERED_SESSION_IDLE_TIMEOUT_SECS
    };
    Duration::from_secs(seconds).as_millis()
}

fn is_reusable_session_age_usable(last_used_ms: u64, now_ms: u64, streaming: bool) -> bool {
    now_ms.saturating_sub(last_used_ms) <= reusable_session_idle_timeout_ms(streaming)
}

fn log_stream_progress_if_needed(
    metrics: &RequestMetrics,
    path: &str,
    next_progress_log: &mut usize,
    received_bytes: usize,
    total_bytes: Option<usize>,
    elapsed_ms: u64,
) {
    if received_bytes < *next_progress_log {
        return;
    }

    let remaining_bytes = total_bytes.map(|total| total.saturating_sub(received_bytes));

    #[cfg(feature = "telemetry-verbose-diagnostics")]
    {
        let stats = esp_alloc::HEAP.stats();
        info!(
            "backend request stream progress path={} received_bytes={} total_bytes={:?} remaining_bytes={:?} elapsed_ms={} heap_used={} heap_free={}",
            path,
            received_bytes,
            total_bytes,
            remaining_bytes,
            elapsed_ms,
            stats.current_usage,
            stats.size.saturating_sub(stats.current_usage),
        );
    }
    crate::memtrace!(
        "request_stream_progress",
        "component" = "backend",
        "at_ms" = now_ms(),
        "sync_id" = metrics.trace.sync_id,
        "req_id" = metrics.trace.req_id,
        "path" = path,
        "request_class" = metrics.class.label(),
        "received_bytes" = received_bytes,
        "total_bytes_known" = bool_flag(total_bytes.is_some()),
        "total_bytes" = total_bytes.unwrap_or(0),
        "remaining_bytes_known" = bool_flag(remaining_bytes.is_some()),
        "remaining_bytes" = remaining_bytes.unwrap_or(0),
        "elapsed_ms" = elapsed_ms,
    );

    while received_bytes >= *next_progress_log {
        *next_progress_log = next_progress_log.saturating_add(STREAM_PROGRESS_LOG_INTERVAL_BYTES);
    }
}

const fn prepare_download_step_count(total_bytes: usize) -> u16 {
    let raw_steps = total_bytes.div_ceil(PREPARE_PROGRESS_DOWNLOAD_STEP_BYTES) as u16;
    if raw_steps < PREPARE_PROGRESS_MIN_DOWNLOAD_STEPS {
        PREPARE_PROGRESS_MIN_DOWNLOAD_STEPS
    } else if raw_steps > PREPARE_PROGRESS_MAX_DOWNLOAD_STEPS {
        PREPARE_PROGRESS_MAX_DOWNLOAD_STEPS
    } else {
        raw_steps
    }
}

fn log_stream_complete(
    metrics: &RequestMetrics,
    path: &str,
    received_bytes: usize,
    total_bytes: Option<usize>,
    response_reusable: bool,
    elapsed_ms: u64,
) {
    #[cfg(feature = "telemetry-verbose-diagnostics")]
    {
        let stats = esp_alloc::HEAP.stats();
        info!(
            "backend request stream complete path={} received_bytes={} total_bytes={:?} response_reusable={} elapsed_ms={} heap_used={} heap_free={}",
            path,
            received_bytes,
            total_bytes,
            response_reusable,
            elapsed_ms,
            stats.current_usage,
            stats.size.saturating_sub(stats.current_usage),
        );
    }
    crate::memtrace!(
        "request_stream_complete",
        "component" = "backend",
        "at_ms" = now_ms(),
        "sync_id" = metrics.trace.sync_id,
        "req_id" = metrics.trace.req_id,
        "path" = path,
        "request_class" = metrics.class.label(),
        "received_bytes" = received_bytes,
        "total_bytes_known" = bool_flag(total_bytes.is_some()),
        "total_bytes" = total_bytes.unwrap_or(0),
        "response_reusable" = bool_flag(response_reusable),
        "elapsed_ms" = elapsed_ms,
    );
}

fn log_request_timing(request: HttpRequest<'_>, status: u16, metrics: &RequestMetrics) {
    crate::internet::mark_backend_path_ready("backend_request");
    info!(
        "backend request timing class={} method={} path={} status={} reused={} resumed={} streaming={} dns_ms={} connect_ms={} tls_ms={} first_byte_ms={} total_ms={}",
        metrics.class.label(),
        request.method,
        request.path,
        status,
        metrics.reused_session,
        metrics.tls_resume_offered,
        metrics.streaming,
        metrics.dns_ms,
        metrics.connect_ms,
        metrics.tls_ms,
        metrics.first_byte_ms.unwrap_or(metrics.total_ms),
        metrics.total_ms,
    );
    crate::memtrace!(
        "request_complete",
        "component" = "backend",
        "at_ms" = now_ms(),
        "sync_id" = request.trace.sync_id,
        "req_id" = request.trace.req_id,
        "method" = request.method,
        "path" = request.path,
        "request_class" = metrics.class.label(),
        "status" = status,
        "reused" = bool_flag(metrics.reused_session),
        "tls_resume_offered" = bool_flag(metrics.tls_resume_offered),
        "streaming" = bool_flag(metrics.streaming),
        "request_body_bytes" = request.body.len(),
        "dns_ms" = metrics.dns_ms,
        "connect_ms" = metrics.connect_ms,
        "tls_ms" = metrics.tls_ms,
        "first_byte_ms" = metrics.first_byte_ms.unwrap_or(metrics.total_ms),
        "total_ms" = metrics.total_ms,
        "header_bytes" = metrics.header_bytes,
        "body_bytes" = metrics.body_bytes,
        "response_bytes" = metrics.response_bytes,
        "content_length_known" = bool_flag(metrics.content_length_known),
        "content_length" = metrics.content_length,
        "response_buffer_capacity" = metrics.response_buffer_capacity,
        "response_buffer_headroom" = metrics.response_buffer_headroom,
        "stream_header_capacity" = metrics.stream_header_capacity,
        "stream_header_headroom" = metrics.stream_header_headroom,
    );
}

const fn is_transient_transport_error(error: BackendError) -> bool {
    matches!(
        error,
        BackendError::Dns | BackendError::Connect | BackendError::Tls | BackendError::Io
    )
}

const fn backend_error_label(error: BackendError) -> &'static str {
    match error {
        BackendError::Alloc => "alloc",
        BackendError::Dns => "dns",
        BackendError::Connect => "connect",
        BackendError::Tls => "tls",
        BackendError::Io => "io",
        BackendError::InvalidResponse => "invalid_response",
        BackendError::InvalidUtf8 => "invalid_utf8",
        BackendError::ResponseTooLarge => "response_too_large",
        BackendError::MissingField => "missing_field",
    }
}

const fn is_auth_status(status: u16) -> bool {
    matches!(status, 401 | 403)
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct CompatError(eio07::ErrorKind);

impl core::fmt::Display for CompatError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

impl core::error::Error for CompatError {}

impl eio07::Error for CompatError {
    fn kind(&self) -> eio07::ErrorKind {
        self.0
    }
}

struct CompatConnection<T> {
    inner: T,
}

impl<T> CompatConnection<T> {
    const fn new(inner: T) -> Self {
        Self { inner }
    }
}

impl<T> eio07::ErrorType for CompatConnection<T>
where
    T: eio06::ErrorType,
{
    type Error = CompatError;
}

impl<T> AsyncRead07 for CompatConnection<T>
where
    T: AsyncRead06 + AsyncWrite06,
{
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        AsyncRead06::read(&mut self.inner, buf)
            .await
            .map_err(map_compat_error)
    }
}

impl<T> AsyncWrite07 for CompatConnection<T>
where
    T: AsyncRead06 + AsyncWrite06,
{
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        AsyncWrite06::write(&mut self.inner, buf)
            .await
            .map_err(map_compat_error)
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        AsyncWrite06::flush(&mut self.inner)
            .await
            .map_err(map_compat_error)
    }
}

fn map_compat_error<E>(error: E) -> CompatError
where
    E: eio06::Error,
{
    CompatError(match error.kind() {
        eio06::ErrorKind::Other => eio07::ErrorKind::Other,
        eio06::ErrorKind::NotFound => eio07::ErrorKind::NotFound,
        eio06::ErrorKind::PermissionDenied => eio07::ErrorKind::PermissionDenied,
        eio06::ErrorKind::ConnectionRefused => eio07::ErrorKind::ConnectionRefused,
        eio06::ErrorKind::ConnectionReset => eio07::ErrorKind::ConnectionReset,
        eio06::ErrorKind::ConnectionAborted => eio07::ErrorKind::ConnectionAborted,
        eio06::ErrorKind::NotConnected => eio07::ErrorKind::NotConnected,
        eio06::ErrorKind::AddrInUse => eio07::ErrorKind::AddrInUse,
        eio06::ErrorKind::AddrNotAvailable => eio07::ErrorKind::AddrNotAvailable,
        eio06::ErrorKind::BrokenPipe => eio07::ErrorKind::BrokenPipe,
        eio06::ErrorKind::AlreadyExists => eio07::ErrorKind::AlreadyExists,
        eio06::ErrorKind::InvalidInput => eio07::ErrorKind::InvalidInput,
        eio06::ErrorKind::InvalidData => eio07::ErrorKind::InvalidData,
        eio06::ErrorKind::TimedOut => eio07::ErrorKind::TimedOut,
        eio06::ErrorKind::Interrupted => eio07::ErrorKind::Interrupted,
        eio06::ErrorKind::Unsupported => eio07::ErrorKind::Unsupported,
        eio06::ErrorKind::OutOfMemory => eio07::ErrorKind::OutOfMemory,
        eio06::ErrorKind::WriteZero => eio07::ErrorKind::WriteZero,
        _ => eio07::ErrorKind::Other,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_credential_wins_over_compile_time() {
        let compile = BackendCredential::from_refresh_token("compile-token").unwrap();
        let stored = BackendCredential::from_refresh_token("stored-token").unwrap();

        let selected = select_startup_credential(Some(compile), Some(stored)).unwrap();
        assert_eq!(selected.source, CredentialSource::Stored);
        assert_eq!(selected.credential.refresh_token().unwrap(), "stored-token");
    }

    #[test]
    fn compile_time_credential_is_used_when_storage_is_missing() {
        let compile = BackendCredential::from_refresh_token("compile-token").unwrap();

        let selected = select_startup_credential(Some(compile), None).unwrap();
        assert_eq!(selected.source, CredentialSource::CompileTime);
        assert_eq!(
            selected.credential.refresh_token().unwrap(),
            "compile-token"
        );
    }

    #[test]
    fn stored_credential_is_used_when_compile_time_is_missing() {
        let stored = BackendCredential::from_refresh_token("stored-token").unwrap();

        let selected = select_startup_credential(None, Some(stored)).unwrap();
        assert_eq!(selected.source, CredentialSource::Stored);
        assert_eq!(selected.credential.refresh_token().unwrap(), "stored-token");
    }

    #[test]
    fn parses_refresh_session_payload() {
        let json = r#"{"session":{"access_token":"access-123","refresh_token":"refresh-456","token_type":"bearer","expires_in":3600}}"#;
        let session = parse_refresh_session(json).unwrap();

        assert_eq!(session.access_token, "access-123");
        assert_eq!(session.refresh_token, "refresh-456");
        assert_eq!(session.expires_in, 3600);
    }

    #[test]
    fn parses_me_profile_payload() {
        let json = r#"{"user_id":"user-123","email":"reader@example.com","role":"authenticated","aal":"aal1"}"#;
        let profile = parse_me_profile(json).unwrap();

        assert_eq!(profile.user_id, "user-123");
        assert_eq!(profile.role, "authenticated");
    }

    #[test]
    fn builds_refresh_body_with_json_escaping() {
        let body = build_refresh_body("hello\"\\world").unwrap();
        assert_eq!(body.as_str(), "{\"refresh_token\":\"hello\\\"\\\\world\"}");
    }

    #[test]
    fn active_access_session_stays_valid_before_refresh_margin() {
        let refresh_session = RefreshSession {
            access_token: bounded_string("access-token").unwrap(),
            refresh_token: bounded_string("refresh-token").unwrap(),
            expires_in: 3600,
        };

        let active = ActiveAccessSession::from_refresh_session(&refresh_session, 1_000);

        assert!(active.is_valid_at(1_001));
        assert!(active.is_valid_at(active.valid_until_ms.saturating_sub(1)));
        assert!(!active.is_valid_at(active.valid_until_ms));
    }

    #[test]
    fn parses_http_response_with_content_length() {
        let response =
            b"HTTP/1.1 200 OK\r\nContent-Length: 15\r\nConnection: keep-alive\r\n\r\n{\"status\":\"ok\"}";
        let parsed = parse_http_response(response).unwrap();

        assert_eq!(parsed.status, 200);
        assert_eq!(parsed.body, "{\"status\":\"ok\"}");
        assert!(parsed.connection_reusable);
    }

    #[test]
    fn parses_http_response_metadata_with_connection_close() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 15\r\nConnection: close\r\n\r\n";
        let metadata = parse_http_response_metadata(response).unwrap();

        assert_eq!(metadata.status, 200);
        assert_eq!(metadata.content_length, Some(15));
        assert!(metadata.connection_close);
        assert!(!is_response_connection_reusable(metadata));
    }

    #[test]
    fn parses_http_response_metadata_with_chunked_transfer_encoding() {
        let response =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: keep-alive\r\n\r\n";
        let metadata = parse_http_response_metadata(response).unwrap();

        assert_eq!(metadata.status, 200);
        assert!(metadata.chunked);
        assert_eq!(metadata.content_length, None);
        assert!(!metadata.connection_close);
        assert!(!is_response_connection_reusable(metadata));
    }

    #[test]
    fn reusable_session_age_window_allows_recent_use_and_rejects_stale_use() {
        assert!(is_reusable_session_age_usable(1_000, 1_000, true));
        assert!(is_reusable_session_age_usable(
            1_000,
            1_000 + reusable_session_idle_timeout_ms(true),
            true,
        ));
        assert!(!is_reusable_session_age_usable(
            1_000,
            1_001 + reusable_session_idle_timeout_ms(true),
            true,
        ));
        assert!(is_reusable_session_age_usable(
            1_000,
            1_000 + reusable_session_idle_timeout_ms(false),
            false,
        ));
    }

    #[test]
    fn content_length_keep_alive_response_is_reusable() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: keep-alive\r\n\r\n{}";
        let metadata = parse_http_response_metadata(response).unwrap();

        assert!(is_response_connection_reusable(metadata));
    }

    #[test]
    fn startup_prefetch_only_marks_item_cached_without_replacing_collection() {
        let mut collection = CollectionManifestState::empty();

        let mut first = CollectionManifestItem::empty();
        first.remote_item_id.set_truncated("saved-1");
        first.content_id.set_truncated("content-1");
        first.package_state = PackageState::Missing;
        assert!(collection.try_push(first));

        let mut second = CollectionManifestItem::empty();
        second.remote_item_id.set_truncated("saved-2");
        second.content_id.set_truncated("content-2");
        second.package_state = PackageState::Missing;
        assert!(collection.try_push(second));

        let request = PrepareContentRequest::from_manifest(CollectionKind::Saved, first);
        mark_prefetched_item_cached(&mut collection, &request);

        assert_eq!(collection.len(), 2);
        assert_eq!(
            collection.item_at(0).unwrap().package_state,
            PackageState::Cached
        );
        assert_eq!(
            collection.item_at(1).unwrap().package_state,
            PackageState::Missing
        );
    }

    #[test]
    fn parses_empty_inbox_response() {
        let summary =
            parse_collection_fetch_summary(r#"{"inbox":[],"next_cursor":null}"#, "\"inbox\"")
                .unwrap();

        assert_eq!(summary.item_count, 0);
        assert!(!summary.next_cursor_present);
        assert!(summary.body_preview.is_none());
        assert!(!summary.body_preview_truncated);
    }

    #[test]
    fn parses_non_empty_inbox_response() {
        let summary = parse_collection_fetch_summary(
            r#"{"inbox":[{"id":"inbox-item-1","content":{"title":"Example Article"},"source":{"title":"Example Source"}}],"next_cursor":"cursor-1"}"#,
            "\"inbox\"",
        )
        .unwrap();

        assert_eq!(summary.item_count, 1);
        assert!(summary.next_cursor_present);
        assert!(summary.body_preview.is_some());
    }

    #[test]
    fn parses_empty_saved_content_response() {
        let result =
            parse_saved_content_fetch_result(r#"{"content":[],"next_cursor":null}"#).unwrap();

        assert_eq!(result.summary.item_count, 0);
        assert!(!result.summary.next_cursor_present);
        assert!(result.summary.body_preview.is_none());
        assert!(result.collection.is_empty());
    }

    #[test]
    fn parses_non_empty_saved_content_response() {
        let result = parse_saved_content_fetch_result(
            r#"{"content":[{"id":"80ac9044-964c-4067-9de3-0d2476cd7d4a","submitted_url":"https://cra.mr/article","read_state":"unread","is_favorited":false,"created_at":1,"updated_at":2,"tags":[],"content":{"id":"c8e17b7a-95e9-4d3b-93da-5d8dca584e4a","canonical_url":"https://cra.mr/article","host":"cra.mr","site_name":"CRA","title":"Optimizing content for agents"}}],"next_cursor":null}"#,
        )
        .unwrap();

        assert_eq!(result.summary.item_count, 1);
        assert!(!result.summary.next_cursor_present);
        assert_eq!(result.collection.len(), 1);
        let item = result.collection.item_at(0).unwrap();
        assert_eq!(
            item.backend_id.as_str(),
            "80ac9044-964c-4067-9de3-0d2476cd7d4a"
        );
        assert_eq!(item.meta.as_str(), "CRA / SAVED");
        assert_eq!(item.title.as_str(), "Optimizing content for agents");
    }

    #[test]
    fn builds_saved_content_page_path_with_cursor() {
        let mut cursor = heapless::String::<COLLECTION_CURSOR_MAX_LEN>::new();
        cursor.push_str("cursor-123").unwrap();

        let path = build_collection_page_path(CollectionEndpoint::Saved, Some(&cursor)).unwrap();

        assert_eq!(
            path.as_str(),
            "/device/v1/me/saved-content?limit=4&archived=false&cursor=cursor-123"
        );
    }

    #[test]
    fn parses_saved_content_page_cursor() {
        let page = parse_saved_content_fetch_page(
            r#"{"content":[{"id":"80ac9044-964c-4067-9de3-0d2476cd7d4a","submitted_url":"https://cra.mr/article","read_state":"unread","is_favorited":false,"created_at":1,"updated_at":2,"tags":[],"content":{"id":"c8e17b7a-95e9-4d3b-93da-5d8dca584e4a","canonical_url":"https://cra.mr/article","host":"cra.mr","site_name":"CRA","title":"Optimizing content for agents"}}],"next_cursor":"cursor-2"}"#,
        )
        .unwrap();

        assert_eq!(page.collection.len(), 1);
        assert_eq!(page.next_cursor.as_ref().unwrap().as_str(), "cursor-2");
        assert!(page.body_preview.is_some());
    }

    #[test]
    fn consume_buffered_chunk_prefix_compacts_remaining_bytes() {
        let mut chunk = [0u8; 8];
        chunk[..6].copy_from_slice(b"ABCDEF");
        let mut buffered_len = 6usize;

        consume_buffered_chunk_prefix(&mut chunk, &mut buffered_len, 4);

        assert_eq!(buffered_len, 2);
        assert_eq!(&chunk[..buffered_len], b"EF");
    }

    #[test]
    fn consume_buffered_chunk_prefix_saturates_to_empty() {
        let mut chunk = [0u8; 8];
        chunk[..3].copy_from_slice(b"XYZ");
        let mut buffered_len = 3usize;

        consume_buffered_chunk_prefix(&mut chunk, &mut buffered_len, 8);

        assert_eq!(buffered_len, 0);
    }
}
