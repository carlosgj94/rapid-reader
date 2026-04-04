extern crate alloc;

use alloc::boxed::Box;
use core::net::IpAddr;
use core::{ffi::CStr, fmt::Write as _, future::Future, net::SocketAddr, ptr::addr_of_mut, str};

use embassy_executor::Spawner;
use embassy_net::{
    Stack,
    dns::DnsSocket,
    tcp::client::{TcpClient, TcpClientState},
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
use log::info;
use mbedtls_rs::{
    Certificate, ClientSessionConfig, Session, SessionConfig, SessionError, Tls, TlsReference,
    TlsVersion, X509,
};
use services::backend_sync::SyncStatus;
use services::storage::StorageError;

use crate::{
    bootstrap::{persist_backend_credential, publish_event},
    content_storage,
    storage::{BACKEND_REFRESH_TOKEN_MAX_LEN, BackendCredential},
};
use domain::{
    content::{
        CONTENT_META_MAX_BYTES, CONTENT_TITLE_MAX_BYTES, CollectionKind, CollectionManifestItem,
        CollectionManifestState, DetailLocator, MANIFEST_ITEM_CAPACITY, PackageState,
        PrepareContentRequest, RECOMMENDATION_SERVE_ID_MAX_BYTES, REMOTE_ITEM_ID_MAX_BYTES,
        RemoteContentStatus,
    },
    runtime::Event,
    text::InlineText,
};

pub(crate) const BACKEND_HOST: &str = "motif-backend-production-a143.up.railway.app";
const BACKEND_HOST_CSTR_BYTES: &[u8] = b"motif-backend-production-a143.up.railway.app\0";
const BACKEND_BASE_URL: &str = "https://motif-backend-production-a143.up.railway.app";
const HEALTH_PATH: &str = "/health";
const REFRESH_PATH: &str = "/auth/session/refresh";
const ME_PATH: &str = "/me";
const INBOX_PATH: &str = "/me/inbox?limit=16";
const SAVED_CONTENT_PATH: &str = "/me/saved-content?limit=16&archived=false";
const RECOMMENDATIONS_PATH: &str = "/me/recommendations/content?limit=16";
pub(crate) const BACKEND_PORT: u16 = 443;
const NETWORK_POLL_MS: u64 = 500;
const RETRY_BACKOFF_MS: u64 = 10_000;
const TRANSPORT_RETRY_ATTEMPTS: usize = 2;
const TRANSPORT_RETRY_BACKOFF_MS: u64 = 750;
const CONNECT_TIMEOUT_SECS: u64 = 5;
const TLS_HANDSHAKE_TIMEOUT_SECS: u64 = 8;
const HTTP_BODY_TIMEOUT_SECS: u64 = 15;
const HTTP_RESPONSE_MAX_LEN: usize = 8 * 1024;
const DIRECT_OPEN_RESPONSE_MAX_LEN: usize = 64 * 1024;
const HTTP_STREAM_HEADER_MAX_LEN: usize = 2048;
const REFRESH_BODY_OVERHEAD_LEN: usize = "{\"refresh_token\":\"\"}".len();
const REQUEST_BODY_MAX_LEN: usize = REFRESH_BODY_OVERHEAD_LEN + (BACKEND_REFRESH_TOKEN_MAX_LEN * 2);
const INBOX_LOG_PREVIEW_MAX_LEN: usize = 256;
// 1 KiB halves package read/write round-trips while adding only ~512 B to the
// backend streaming scratch buffer. A 2 KiB jump would cost noticeably more
// RAM once the storage queue and sender-side scratch buffer are included.
const PACKAGE_DOWNLOAD_CHUNK_LEN: usize = 1024;
const STARTUP_SAVED_PREFETCH_LIMIT: usize = 4;
const USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));
const BACKEND_CA_CHAIN_PEM: &str =
    concat!(include_str!("../certs/letsencrypt_isrg_root_x1.pem"), "\0");
const BACKEND_CMD_QUEUE_CAPACITY: usize = 4;
type BackendTcpClientState = TcpClientState<1, 1024, 1024>;
type BackendTcpClient<'a> = TcpClient<'a, 1, 1024, 1024>;

static BACKEND_CMD_CH: Channel<
    CriticalSectionRawMutex,
    BackendCommand,
    BACKEND_CMD_QUEUE_CAPACITY,
> = Channel::new();
static mut HTTP_RESPONSE_BUFFER: [u8; HTTP_RESPONSE_MAX_LEN] = [0; HTTP_RESPONSE_MAX_LEN];
static mut DIRECT_OPEN_RESPONSE_BUFFER: [u8; DIRECT_OPEN_RESPONSE_MAX_LEN] =
    [0; DIRECT_OPEN_RESPONSE_MAX_LEN];

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
    item_count: usize,
    next_cursor_present: bool,
    body_preview: Option<heapless::String<INBOX_LOG_PREVIEW_MAX_LEN>>,
    body_preview_truncated: bool,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct CollectionFetchResult {
    summary: CollectionFetchSummary,
    collection: CollectionManifestState,
}

struct StartupSyncResult {
    refresh_session: RefreshSession,
    saved_result: Result<CollectionFetchResult, CollectionQueryError>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum BackendCommand {
    PrepareContent(PrepareContentRequest),
    OpenRemoteContent(PrepareContentRequest),
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct HttpResponse<'a> {
    status: u16,
    body: &'a str,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct HttpRequest<'a> {
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
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct RequestMetrics {
    started_ms: u64,
    dns_ms: u64,
    connect_ms: u64,
    tls_ms: u64,
    first_byte_ms: Option<u64>,
    total_ms: u64,
    reused_session: bool,
    streaming: bool,
}

impl RequestMetrics {
    fn new(reused_session: bool, streaming: bool) -> Self {
        Self {
            started_ms: now_ms(),
            dns_ms: 0,
            connect_ms: 0,
            tls_ms: 0,
            first_byte_ms: None,
            total_ms: 0,
            reused_session,
            streaming,
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

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum BackendError {
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
        info!("backend failed to spawn auth task");
    }
}

pub async fn request_prepare_content(request: PrepareContentRequest) {
    BACKEND_CMD_CH
        .send(BackendCommand::PrepareContent(request))
        .await;
}

pub async fn request_open_remote_content(request: PrepareContentRequest) {
    BACKEND_CMD_CH
        .send(BackendCommand::OpenRemoteContent(request))
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
            info!("backend tls init failed: TRNG unavailable");
            return;
        }
    };

    let tls = match Tls::new(&mut trng) {
        Ok(tls) => tls,
        Err(err) => {
            log_status(SyncStatus::TransportFailed);
            info!("backend tls init failed: {:?}", err);
            return;
        }
    };

    let ca_chain = match backend_ca_chain() {
        Ok(ca_chain) => ca_chain,
        Err(err) => {
            log_status(SyncStatus::TransportFailed);
            info!("backend ca chain init failed: {:?}", err);
            return;
        }
    };

    let mut current = startup;
    let tcp_state = Box::new(BackendTcpClientState::new());

    loop {
        log_status(SyncStatus::WaitingForNetwork);
        let network = wait_for_network(stack).await;
        info!("backend network ready ip={:?}", network.address);
        log_heap("backend network ready");
        let mut tcp_client = BackendTcpClient::new(stack, tcp_state.as_ref());
        tcp_client.set_timeout(Some(Duration::from_secs(HTTP_BODY_TIMEOUT_SECS)));

        log_status(SyncStatus::RefreshingSession);
        crate::internet::set_probe_suspended(true);
        let startup_sync = perform_startup_refresh_and_saved_sync(
            stack,
            tls.reference(),
            &ca_chain,
            &tcp_client,
            &current.credential,
        )
        .await;
        crate::internet::set_probe_suspended(false);

        let startup_sync = match startup_sync {
            Ok(result) => result,
            Err(RefreshError::Rejected(status)) => {
                log_status(SyncStatus::AuthFailed);
                info!(
                    "backend refresh rejected status={} source={}",
                    status,
                    current.source.label(),
                );
                return;
            }
            Err(RefreshError::Other(err)) => {
                log_status(SyncStatus::TransportFailed);
                info!("backend refresh failed: {:?}", err);
                Timer::after(Duration::from_millis(RETRY_BACKOFF_MS)).await;
                continue;
            }
        };

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

        sync_one_collection(
            CollectionKind::Saved,
            startup_sync.saved_result,
            "backend saved",
        )
        .await;
        log_status(SyncStatus::Ready);

        loop {
            match BACKEND_CMD_CH.receive().await {
                BackendCommand::PrepareContent(request) => {
                    handle_prepare_content_request(
                        stack,
                        tls.reference(),
                        &ca_chain,
                        tcp_state.as_ref(),
                        &mut current,
                        &mut access_session,
                        request,
                    )
                    .await;
                    log_status(SyncStatus::Ready);
                }
                BackendCommand::OpenRemoteContent(request) => {
                    handle_open_remote_content_request(
                        stack,
                        tls.reference(),
                        &ca_chain,
                        tcp_state.as_ref(),
                        &mut current,
                        &mut access_session,
                        request,
                    )
                    .await;
                    log_status(SyncStatus::Ready);
                }
            }
        }
    }
}

async fn wait_for_network(stack: Stack<'static>) -> NetworkReady {
    loop {
        if stack.is_link_up()
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
    let mut last_error = None;
    let mut attempt = 0;
    while attempt < 3 {
        let response_buffer = standard_response_buffer();
        let response = send_https_request(
            stack,
            tls,
            ca_chain,
            tcp_state,
            HttpRequest {
                method: "GET",
                path: HEALTH_PATH,
                content_type: None,
                bearer_token: None,
                body: b"",
                connection_close: true,
            },
            response_buffer,
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
                    info!("backend health retry attempt={} err={:?}", attempt, err);
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
    credential: &BackendCredential,
) -> Result<RefreshSession, RefreshError> {
    let refresh_token = credential
        .refresh_token()
        .map_err(|_| RefreshError::Other(BackendError::InvalidResponse))?;
    info!(
        "backend refresh building request token_len={}",
        refresh_token.len()
    );

    let response = {
        let body = Box::new(
            build_refresh_body(refresh_token)
                .map_err(|_| RefreshError::Other(BackendError::InvalidResponse))?,
        );
        info!("backend refresh request ready body_len={}", body.len());
        let response_buffer = standard_response_buffer();
        send_https_request(
            stack,
            tls,
            ca_chain,
            tcp_state,
            HttpRequest {
                method: "POST",
                path: REFRESH_PATH,
                content_type: Some("application/json"),
                bearer_token: None,
                body: body.as_bytes(),
                connection_close: true,
            },
            response_buffer,
        )
        .await
        .map_err(RefreshError::Other)?
    };

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
    access_token: &str,
) -> Result<MeProfile, IdentityError> {
    let response_buffer = standard_response_buffer();
    let response = send_https_request(
        stack,
        tls,
        ca_chain,
        tcp_state,
        HttpRequest {
            method: "GET",
            path: ME_PATH,
            content_type: Some("application/json"),
            bearer_token: Some(access_token),
            body: b"",
            connection_close: true,
        },
        response_buffer,
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
    access_token: &str,
) {
    info!("backend startup sync mode=saved-only");
    sync_one_collection(
        CollectionKind::Saved,
        perform_saved_content_fetch(stack, tls, ca_chain, tcp_state, access_token).await,
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
) -> Result<StartupSyncResult, RefreshError> {
    let mut attempt = 0usize;

    loop {
        let result = perform_startup_refresh_and_saved_sync_once(
            stack, tls, ca_chain, tcp_client, credential,
        )
        .await;
        match result {
            Err(RefreshError::Other(err))
                if is_transient_transport_error(err) && attempt + 1 < TRANSPORT_RETRY_ATTEMPTS =>
            {
                attempt += 1;
                info!("backend startup retry attempt={} err={:?}", attempt, err);
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
    credential: &BackendCredential,
) -> Result<StartupSyncResult, RefreshError> {
    let refresh_token = credential
        .refresh_token()
        .map_err(|_| RefreshError::Other(BackendError::InvalidResponse))?;
    info!(
        "backend refresh building request token_len={}",
        refresh_token.len()
    );

    let mut refresh_metrics = RequestMetrics::new(false, false);
    let dns = DnsSocket::new(stack);
    let dns_started_ms = now_ms();
    let remote = dns
        .get_host_by_name(BACKEND_HOST, AddrType::IPv4)
        .await
        .map_err(|_| {
            info!("backend request dns failed path={}", REFRESH_PATH);
            log_request_heap(REFRESH_PATH, "dns failed");
            RefreshError::Other(BackendError::Dns)
        })?;
    refresh_metrics.dns_ms = elapsed_since_ms(dns_started_ms);
    let remote = match remote {
        IpAddr::V4(addr) => addr,
        IpAddr::V6(_) => {
            info!("backend request dns returned ipv6 path={}", REFRESH_PATH);
            log_request_heap(REFRESH_PATH, "dns ipv6");
            return Err(RefreshError::Other(BackendError::Dns));
        }
    };
    let connect_started_ms = now_ms();
    let connection = with_timeout(
        Duration::from_secs(CONNECT_TIMEOUT_SECS),
        tcp_client.connect(SocketAddr::new(IpAddr::V4(remote), BACKEND_PORT)),
    )
    .await
    .map_err(|_| {
        info!("backend request connect timed out path={}", REFRESH_PATH);
        log_request_heap(REFRESH_PATH, "connect timeout");
        RefreshError::Other(BackendError::Connect)
    })?
    .map_err(|_| {
        info!("backend request connect failed path={}", REFRESH_PATH);
        log_request_heap(REFRESH_PATH, "connect failed");
        RefreshError::Other(BackendError::Connect)
    })?;
    refresh_metrics.connect_ms = elapsed_since_ms(connect_started_ms);
    let mut session = open_tls_session(tls, ca_chain, CompatConnection::new(connection))
        .inspect_err(|_err| {
            info!("backend request tls setup failed path={}", REFRESH_PATH);
            log_request_heap(REFRESH_PATH, "tls setup failed");
        })
        .map_err(RefreshError::Other)?;
    let tls_started_ms = now_ms();
    await_tls_handshake(&mut session, REFRESH_PATH)
        .await
        .map_err(RefreshError::Other)?;
    refresh_metrics.tls_ms = elapsed_since_ms(tls_started_ms);
    let verification_flags = session.tls_verification_details();
    if verification_flags != 0 {
        info!(
            "backend request tls verification flags path={} flags=0x{:08x}",
            REFRESH_PATH, verification_flags
        );
    }

    let refresh_response = {
        let body = Box::new(
            build_refresh_body(refresh_token)
                .map_err(|_| RefreshError::Other(BackendError::InvalidResponse))?,
        );
        info!("backend refresh request ready body_len={}", body.len());
        let response_buffer = standard_response_buffer();
        send_https_request_over_session_with_metrics(
            &mut session,
            HttpRequest {
                method: "POST",
                path: REFRESH_PATH,
                content_type: Some("application/json"),
                bearer_token: None,
                body: body.as_bytes(),
                connection_close: false,
            },
            response_buffer,
            refresh_metrics,
        )
        .await
        .map_err(RefreshError::Other)?
    };

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
        prefetch_startup_saved_content(&mut session, refresh_session.access_token.as_str(), result)
            .await;
    }

    if let Err(err) = session.close().await {
        info!("backend tls close failed: {:?}", err);
    }

    Ok(StartupSyncResult {
        refresh_session,
        saved_result,
    })
}

async fn prefetch_startup_saved_content<T>(
    session: &mut Session<'_, T>,
    access_token: &str,
    result: &mut CollectionFetchResult,
) where
    T: AsyncRead07 + AsyncWrite07,
{
    let mut prefetched = 0usize;
    let mut index = 0usize;
    while index < result.collection.len() && prefetched < STARTUP_SAVED_PREFETCH_LIMIT {
        let item = result.collection.items[index];
        index += 1;

        if !item.can_prepare() {
            continue;
        }
        let request = PrepareContentRequest::from_manifest(CollectionKind::Saved, item);
        match fetch_and_stage_package_over_session(session, access_token, request).await {
            Ok(snapshot) => {
                result.collection = snapshot;
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

async fn ensure_access_session(
    stack: Stack<'static>,
    tls: TlsReference<'_>,
    ca_chain: &Certificate<'static>,
    tcp_state: &BackendTcpClientState,
    current: &mut StartupCredential,
    access_session: &mut Option<ActiveAccessSession>,
) -> Result<(), RefreshError> {
    let now_ms = now_ms();
    if access_session
        .as_ref()
        .is_some_and(|session| session.is_valid_at(now_ms))
    {
        info!("backend access session reuse");
        return Ok(());
    }

    log_status(SyncStatus::RefreshingSession);
    let refresh_session =
        perform_refresh(stack, tls, ca_chain, tcp_state, &current.credential).await?;
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
            let collection = match content_storage::persist_snapshot(kind, result.collection).await
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
                "{} ok item_count={} next_cursor={}",
                label,
                result.summary.item_count,
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

async fn handle_prepare_content_request(
    stack: Stack<'static>,
    tls: TlsReference<'_>,
    ca_chain: &Certificate<'static>,
    tcp_state: &BackendTcpClientState,
    current: &mut StartupCredential,
    access_session: &mut Option<ActiveAccessSession>,
    request: PrepareContentRequest,
) {
    if request.remote_item_id.is_empty() || request.content_id.is_empty() {
        return;
    }

    if !stack.is_link_up() {
        let _ = publish_package_state(
            request.collection,
            request.remote_item_id,
            PackageState::Failed,
        )
        .await;
        info!(
            "backend content prepare skipped: network unavailable collection={:?}",
            request.collection
        );
        return;
    }

    if let Err(err) =
        ensure_access_session(stack, tls, ca_chain, tcp_state, current, access_session).await
    {
        match err {
            RefreshError::Rejected(status) => {
                log_status(SyncStatus::AuthFailed);
                info!(
                    "backend content prepare refresh rejected status={} source={}",
                    status,
                    current.source.label(),
                );
            }
            RefreshError::Other(err) => {
                log_status(SyncStatus::TransportFailed);
                info!("backend content prepare refresh failed: {:?}", err);
            }
        }
        let _ = publish_package_state(
            request.collection,
            request.remote_item_id,
            PackageState::Failed,
        )
        .await;
        return;
    }

    log_status(SyncStatus::SyncingContent);
    let access_token = access_session
        .as_ref()
        .map(|session| session.access_token.as_str())
        .unwrap_or("");

    match fetch_and_stage_package(stack, tls, ca_chain, tcp_state, access_token, request).await {
        Ok(snapshot) => {
            publish_event(
                Event::CollectionContentUpdated(request.collection, Box::new(snapshot)),
                now_ms(),
            );
            info!(
                "backend content cached collection={:?} content_id={}",
                request.collection,
                request.content_id.as_str(),
            );
            match content_storage::open_cached_reader_content(request.content_id).await {
                Ok(opened) => {
                    publish_event(
                        Event::ReaderContentOpened {
                            collection: request.collection,
                            title: opened.title,
                            document: opened.document,
                        },
                        now_ms(),
                    );
                    info!(
                        "backend content opened after prepare collection={:?} content_id={} truncated={}",
                        request.collection,
                        request.content_id.as_str(),
                        opened.truncated,
                    );
                }
                Err(err) => {
                    let _ = publish_package_state(
                        request.collection,
                        request.remote_item_id,
                        PackageState::Failed,
                    )
                    .await;
                    info!(
                        "backend content open after prepare failed collection={:?} content_id={} err={:?}",
                        request.collection,
                        request.content_id.as_str(),
                        err,
                    );
                }
            }
        }
        Err(PackagePrepareError::PendingRemote) => {
            let _ = publish_package_state(
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
            if is_auth_status(status) {
                log_status(SyncStatus::AuthFailed);
            }
            let _ = publish_package_state(
                request.collection,
                request.remote_item_id,
                PackageState::Failed,
            )
            .await;
            info!("backend content fetch rejected status={}", status);
        }
        Err(PackagePrepareError::Other(err)) => {
            let _ = publish_package_state(
                request.collection,
                request.remote_item_id,
                PackageState::Failed,
            )
            .await;
            info!("backend content fetch failed: {:?}", err);
        }
    }
}

async fn handle_open_remote_content_request(
    stack: Stack<'static>,
    tls: TlsReference<'_>,
    ca_chain: &Certificate<'static>,
    tcp_state: &BackendTcpClientState,
    current: &mut StartupCredential,
    access_session: &mut Option<ActiveAccessSession>,
    request: PrepareContentRequest,
) {
    if request.remote_item_id.is_empty() || request.content_id.is_empty() {
        return;
    }

    if !stack.is_link_up() {
        info!(
            "backend content open skipped: network unavailable collection={:?}",
            request.collection
        );
        return;
    }

    if let Err(err) =
        ensure_access_session(stack, tls, ca_chain, tcp_state, current, access_session).await
    {
        match err {
            RefreshError::Rejected(status) => {
                log_status(SyncStatus::AuthFailed);
                info!(
                    "backend content open refresh rejected status={} source={}",
                    status,
                    current.source.label(),
                );
            }
            RefreshError::Other(err) => {
                log_status(SyncStatus::TransportFailed);
                info!("backend content open refresh failed: {:?}", err);
            }
        }
        return;
    }

    log_status(SyncStatus::SyncingContent);
    let access_token = access_session
        .as_ref()
        .map(|session| session.access_token.as_str())
        .unwrap_or("");

    match fetch_opened_reader_content(stack, tls, ca_chain, tcp_state, access_token, request).await
    {
        Ok(opened) => {
            publish_event(
                Event::ReaderContentOpened {
                    collection: request.collection,
                    title: opened.title,
                    document: opened.document,
                },
                now_ms(),
            );
            info!(
                "backend content opened remote collection={:?} content_id={} truncated={}",
                request.collection,
                request.content_id.as_str(),
                opened.truncated,
            );
        }
        Err(PackagePrepareError::PendingRemote) => {
            info!(
                "backend content open pending remote collection={:?} content_id={}",
                request.collection,
                request.content_id.as_str(),
            );
        }
        Err(PackagePrepareError::Rejected(status)) => {
            if is_auth_status(status) {
                log_status(SyncStatus::AuthFailed);
            }
            info!("backend content open rejected status={}", status);
        }
        Err(PackagePrepareError::Other(err)) => {
            info!("backend content open failed: {:?}", err);
        }
    }
}

async fn send_https_request<'a>(
    stack: Stack<'static>,
    tls: TlsReference<'_>,
    ca_chain: &Certificate<'static>,
    tcp_state: &BackendTcpClientState,
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
            request,
            &mut *response_buffer_ptr,
        )
        .await
    };
    match first_attempt {
        Err(err) if is_transient_transport_error(err) && TRANSPORT_RETRY_ATTEMPTS > 1 => {
            info!(
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
    tls: TlsReference<'_>,
    ca_chain: &Certificate<'static>,
    tcp_state: &BackendTcpClientState,
    request: HttpRequest<'_>,
    response_buffer: &'a mut [u8],
) -> Result<HttpResponse<'a>, BackendError> {
    let mut metrics = RequestMetrics::new(false, false);
    let mut tcp_client = TcpClient::new(stack, tcp_state);
    tcp_client.set_timeout(Some(Duration::from_secs(HTTP_BODY_TIMEOUT_SECS)));

    let dns = DnsSocket::new(stack);
    let dns_started_ms = now_ms();
    let remote = dns
        .get_host_by_name(BACKEND_HOST, AddrType::IPv4)
        .await
        .map_err(|_| {
            info!("backend request dns failed path={}", request.path);
            log_request_heap(request.path, "dns failed");
            BackendError::Dns
        })?;
    metrics.dns_ms = elapsed_since_ms(dns_started_ms);
    let remote = match remote {
        IpAddr::V4(addr) => addr,
        IpAddr::V6(_) => {
            info!("backend request dns returned ipv6 path={}", request.path);
            log_request_heap(request.path, "dns ipv6");
            return Err(BackendError::Dns);
        }
    };
    let connect_started_ms = now_ms();
    let connection = with_timeout(
        Duration::from_secs(CONNECT_TIMEOUT_SECS),
        tcp_client.connect(SocketAddr::new(IpAddr::V4(remote), BACKEND_PORT)),
    )
    .await
    .map_err(|_| {
        info!("backend request connect timed out path={}", request.path);
        log_request_heap(request.path, "connect timeout");
        BackendError::Connect
    })?
    .map_err(|_| {
        info!("backend request connect failed path={}", request.path);
        log_request_heap(request.path, "connect failed");
        BackendError::Connect
    })?;
    metrics.connect_ms = elapsed_since_ms(connect_started_ms);
    let mut session = open_tls_session(tls, ca_chain, CompatConnection::new(connection))
        .inspect_err(|_err| {
            info!("backend request tls setup failed path={}", request.path);
            log_request_heap(request.path, "tls setup failed");
        })?;
    let tls_started_ms = now_ms();
    await_tls_handshake(&mut session, request.path).await?;
    metrics.tls_ms = elapsed_since_ms(tls_started_ms);
    let verification_flags = session.tls_verification_details();
    if verification_flags != 0 {
        info!(
            "backend request tls verification flags path={} flags=0x{:08x}",
            request.path, verification_flags
        );
    }
    let response = send_https_request_over_session_with_metrics(
        &mut session,
        request,
        response_buffer,
        metrics,
    )
    .await;
    if let Err(err) = session.close().await {
        info!("backend tls close failed: {:?}", err);
    }

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
        RequestMetrics::new(true, false),
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
        request.path,
        request.method,
        request.content_type,
        request.bearer_token,
        request.body,
        request.connection_close,
    )
    .await?;
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
    config.min_version = TlsVersion::Tls1_2;

    Session::new(tls, stream, &SessionConfig::Client(config)).map_err(|_| BackendError::Tls)
}

fn standard_response_buffer() -> &'static mut [u8] {
    // The backend task processes request/response flows serially, so one shared
    // fixed buffer avoids repeated 8 KiB stack frames during startup sync.
    unsafe {
        core::slice::from_raw_parts_mut(
            addr_of_mut!(HTTP_RESPONSE_BUFFER).cast::<u8>(),
            HTTP_RESPONSE_MAX_LEN,
        )
    }
}

fn map_logged_session_error(path: &str, stage: &str, error: SessionError) -> BackendError {
    match error {
        SessionError::MbedTls(err) => {
            info!(
                "backend request tls {} failed path={} err={:?}",
                stage, path, err
            );
            log_request_heap(path, stage);
            BackendError::Tls
        }
        SessionError::Io(err) => {
            info!(
                "backend request io {} failed path={} err={:?}",
                stage, path, err
            );
            log_request_heap(path, stage);
            BackendError::Io
        }
    }
}

fn log_request_timeout(path: &str, stage: &str, error: BackendError) -> BackendError {
    info!("backend request {} timed out path={}", stage, path);
    log_request_heap(path, stage);
    error
}

async fn await_tls_handshake<T>(
    session: &mut Session<'_, T>,
    path: &str,
) -> Result<(), BackendError>
where
    T: AsyncRead07 + AsyncWrite07,
{
    with_timeout(
        Duration::from_secs(TLS_HANDSHAKE_TIMEOUT_SECS),
        session.connect(),
    )
    .await
    .map_err(|_| log_request_timeout(path, "handshake", BackendError::Tls))?
    .map_err(|err| map_logged_session_error(path, "handshake", err))
}

async fn await_body_io_timeout<T, F>(path: &str, stage: &str, future: F) -> Result<T, BackendError>
where
    F: Future<Output = Result<T, SessionError>>,
{
    with_timeout(Duration::from_secs(HTTP_BODY_TIMEOUT_SECS), future)
        .await
        .map_err(|_| log_request_timeout(path, stage, BackendError::Io))?
        .map_err(|err| map_logged_session_error(path, stage, err))
}

async fn write_http_request<T>(
    session: &mut Session<'_, T>,
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
        "write method",
        AsyncWrite07::write_all(session, method.as_bytes()),
    )
    .await?;
    await_body_io_timeout(
        path,
        "write separator",
        AsyncWrite07::write_all(session, b" "),
    )
    .await?;
    await_body_io_timeout(
        path,
        "write path",
        AsyncWrite07::write_all(session, path.as_bytes()),
    )
    .await?;
    await_body_io_timeout(
        path,
        "write request line",
        AsyncWrite07::write_all(session, b" HTTP/1.1\r\nHost: "),
    )
    .await?;
    await_body_io_timeout(
        path,
        "write host",
        AsyncWrite07::write_all(session, BACKEND_HOST.as_bytes()),
    )
    .await?;
    await_body_io_timeout(
        path,
        "write user agent header",
        AsyncWrite07::write_all(session, b"\r\nUser-Agent: "),
    )
    .await?;
    await_body_io_timeout(
        path,
        "write user agent",
        AsyncWrite07::write_all(session, USER_AGENT.as_bytes()),
    )
    .await?;
    await_body_io_timeout(
        path,
        "write connection header",
        AsyncWrite07::write_all(session, b"\r\nAccept: application/json\r\nConnection: "),
    )
    .await?;
    await_body_io_timeout(
        path,
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
            "write auth header",
            AsyncWrite07::write_all(session, b"Authorization: Bearer "),
        )
        .await?;
        await_body_io_timeout(
            path,
            "write auth token",
            AsyncWrite07::write_all(session, token.as_bytes()),
        )
        .await?;
        await_body_io_timeout(
            path,
            "write auth line ending",
            AsyncWrite07::write_all(session, b"\r\n"),
        )
        .await?;
    }

    if !body.is_empty() {
        if let Some(content_type) = content_type {
            await_body_io_timeout(
                path,
                "write content type header",
                AsyncWrite07::write_all(session, b"Content-Type: "),
            )
            .await?;
            await_body_io_timeout(
                path,
                "write content type",
                AsyncWrite07::write_all(session, content_type.as_bytes()),
            )
            .await?;
            await_body_io_timeout(
                path,
                "write content type line ending",
                AsyncWrite07::write_all(session, b"\r\n"),
            )
            .await?;
        }

        let mut content_length = heapless::String::<16>::new();
        write!(&mut content_length, "{}", body.len()).map_err(|_| BackendError::InvalidResponse)?;
        await_body_io_timeout(
            path,
            "write content length header",
            AsyncWrite07::write_all(session, b"Content-Length: "),
        )
        .await?;
        await_body_io_timeout(
            path,
            "write content length",
            AsyncWrite07::write_all(session, content_length.as_bytes()),
        )
        .await?;
        await_body_io_timeout(
            path,
            "write content length line ending",
            AsyncWrite07::write_all(session, b"\r\n"),
        )
        .await?;
    }

    await_body_io_timeout(
        path,
        "write header terminator",
        AsyncWrite07::write_all(session, b"\r\n"),
    )
    .await?;

    if !body.is_empty() {
        await_body_io_timeout(path, "write body", AsyncWrite07::write_all(session, body)).await?;
    }

    await_body_io_timeout(path, "flush", AsyncWrite07::flush(session)).await?;
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
            saw_headers = true;
        }

        if let Some(expected_total) = expected_total
            && total >= expected_total
        {
            total = expected_total;
            break;
        }
    }

    parse_http_response(&response_buffer[..total])
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
    })
}

fn parse_http_response_metadata(response: &[u8]) -> Result<HttpResponseMetadata, BackendError> {
    let header_end = find_subslice(response, b"\r\n\r\n").ok_or(BackendError::InvalidResponse)?;
    let status = parse_http_status(response)?;
    let header_text =
        str::from_utf8(&response[..header_end]).map_err(|_| BackendError::InvalidUtf8)?;
    let mut content_length = None;
    let mut chunked = false;

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
        }
    }

    Ok(HttpResponseMetadata {
        status,
        body_start: header_end + 4,
        content_length,
        chunked,
    })
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

async fn perform_inbox_fetch(
    stack: Stack<'static>,
    tls: TlsReference<'_>,
    ca_chain: &Certificate<'static>,
    tcp_state: &BackendTcpClientState,
    access_token: &str,
) -> Result<CollectionFetchResult, CollectionQueryError> {
    let response_buffer = standard_response_buffer();
    let response = send_https_request(
        stack,
        tls,
        ca_chain,
        tcp_state,
        HttpRequest {
            method: "GET",
            path: INBOX_PATH,
            content_type: Some("application/json"),
            bearer_token: Some(access_token),
            body: b"",
            connection_close: true,
        },
        response_buffer,
    )
    .await
    .map_err(CollectionQueryError::Other)?;

    if (400..500).contains(&response.status) {
        return Err(CollectionQueryError::Rejected(response.status));
    }
    if response.status != 200 {
        return Err(CollectionQueryError::Other(BackendError::InvalidResponse));
    }

    parse_inbox_fetch_result(response.body).map_err(CollectionQueryError::Other)
}

async fn perform_saved_content_fetch(
    stack: Stack<'static>,
    tls: TlsReference<'_>,
    ca_chain: &Certificate<'static>,
    tcp_state: &BackendTcpClientState,
    access_token: &str,
) -> Result<CollectionFetchResult, CollectionQueryError> {
    let response_buffer = standard_response_buffer();
    let response = send_https_request(
        stack,
        tls,
        ca_chain,
        tcp_state,
        HttpRequest {
            method: "GET",
            path: SAVED_CONTENT_PATH,
            content_type: Some("application/json"),
            bearer_token: Some(access_token),
            body: b"",
            connection_close: true,
        },
        response_buffer,
    )
    .await
    .map_err(CollectionQueryError::Other)?;

    if (400..500).contains(&response.status) {
        return Err(CollectionQueryError::Rejected(response.status));
    }
    if response.status != 200 {
        return Err(CollectionQueryError::Other(BackendError::InvalidResponse));
    }

    parse_saved_content_fetch_result(response.body).map_err(CollectionQueryError::Other)
}

async fn perform_recommendation_fetch(
    stack: Stack<'static>,
    tls: TlsReference<'_>,
    ca_chain: &Certificate<'static>,
    tcp_state: &BackendTcpClientState,
    access_token: &str,
) -> Result<CollectionFetchResult, CollectionQueryError> {
    let response_buffer = standard_response_buffer();
    let response = send_https_request(
        stack,
        tls,
        ca_chain,
        tcp_state,
        HttpRequest {
            method: "GET",
            path: RECOMMENDATIONS_PATH,
            content_type: Some("application/json"),
            bearer_token: Some(access_token),
            body: b"",
            connection_close: true,
        },
        response_buffer,
    )
    .await
    .map_err(CollectionQueryError::Other)?;

    if (400..500).contains(&response.status) {
        return Err(CollectionQueryError::Rejected(response.status));
    }
    if response.status != 200 {
        return Err(CollectionQueryError::Other(BackendError::InvalidResponse));
    }

    parse_recommendation_fetch_result(response.body).map_err(CollectionQueryError::Other)
}

async fn fetch_and_stage_package(
    stack: Stack<'static>,
    tls: TlsReference<'_>,
    ca_chain: &Certificate<'static>,
    tcp_state: &BackendTcpClientState,
    access_token: &str,
    request: PrepareContentRequest,
) -> Result<CollectionManifestState, PackagePrepareError> {
    let path = build_detail_path(request).map_err(PackagePrepareError::Other)?;
    let mut attempt = 0usize;

    loop {
        content_storage::begin_package_stage(request.content_id, request.remote_revision)
            .await
            .map_err(map_storage_prepare_error)?;

        let status = match stream_https_response_body_to_storage(
            stack,
            tls,
            ca_chain,
            tcp_state,
            HttpRequest {
                method: "GET",
                path: path.as_str(),
                content_type: Some("application/json"),
                bearer_token: Some(access_token),
                body: b"",
                connection_close: true,
            },
        )
        .await
        {
            Ok(status) => status,
            Err(err)
                if is_transient_transport_error(err) && attempt + 1 < TRANSPORT_RETRY_ATTEMPTS =>
            {
                let _ = content_storage::abort_package_stage().await;
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
                let _ = content_storage::abort_package_stage().await;
                return Err(PackagePrepareError::Other(err));
            }
        };

        if (400..500).contains(&status) {
            let _ = content_storage::abort_package_stage().await;
            return Err(PackagePrepareError::Rejected(status));
        }
        if status != 200 {
            let _ = content_storage::abort_package_stage().await;
            return Err(PackagePrepareError::Other(BackendError::InvalidResponse));
        }

        let snapshot =
            content_storage::commit_package_stage(request.collection, request.remote_item_id)
                .await
                .map_err(map_storage_prepare_error)?;

        if manifest_item_state(&snapshot, &request.remote_item_id)
            == Some(PackageState::PendingRemote)
        {
            return Err(PackagePrepareError::PendingRemote);
        }

        return Ok(snapshot);
    }
}

async fn fetch_and_stage_package_over_session<T>(
    session: &mut Session<'_, T>,
    access_token: &str,
    request: PrepareContentRequest,
) -> Result<CollectionManifestState, PackagePrepareError>
where
    T: AsyncRead07 + AsyncWrite07,
{
    let path = build_detail_path(request).map_err(PackagePrepareError::Other)?;
    content_storage::begin_package_stage(request.content_id, request.remote_revision)
        .await
        .map_err(map_storage_prepare_error)?;

    let status = match stream_https_response_body_to_storage_over_session(
        session,
        HttpRequest {
            method: "GET",
            path: path.as_str(),
            content_type: Some("application/json"),
            bearer_token: Some(access_token),
            body: b"",
            connection_close: false,
        },
    )
    .await
    {
        Ok(status) => status,
        Err(err) => {
            let _ = content_storage::abort_package_stage().await;
            return Err(PackagePrepareError::Other(err));
        }
    };

    if (400..500).contains(&status) {
        let _ = content_storage::abort_package_stage().await;
        return Err(PackagePrepareError::Rejected(status));
    }
    if status != 200 {
        let _ = content_storage::abort_package_stage().await;
        return Err(PackagePrepareError::Other(BackendError::InvalidResponse));
    }

    let snapshot =
        content_storage::commit_package_stage(request.collection, request.remote_item_id)
            .await
            .map_err(map_storage_prepare_error)?;

    if manifest_item_state(&snapshot, &request.remote_item_id) == Some(PackageState::PendingRemote)
    {
        return Err(PackagePrepareError::PendingRemote);
    }

    Ok(snapshot)
}

async fn fetch_opened_reader_content(
    stack: Stack<'static>,
    tls: TlsReference<'_>,
    ca_chain: &Certificate<'static>,
    tcp_state: &BackendTcpClientState,
    access_token: &str,
    request: PrepareContentRequest,
) -> Result<content_storage::OpenedReaderContent, PackagePrepareError> {
    let path = build_detail_path(request).map_err(PackagePrepareError::Other)?;
    // The backend task is single-consumer, so a shared fixed buffer avoids heap pressure
    // when opening a remote article directly during development fallback.
    let response_buffer = unsafe {
        core::slice::from_raw_parts_mut(
            addr_of_mut!(DIRECT_OPEN_RESPONSE_BUFFER).cast::<u8>(),
            DIRECT_OPEN_RESPONSE_MAX_LEN,
        )
    };
    let response = send_https_request(
        stack,
        tls,
        ca_chain,
        tcp_state,
        HttpRequest {
            method: "GET",
            path: path.as_str(),
            content_type: Some("application/json"),
            bearer_token: Some(access_token),
            body: b"",
            connection_close: true,
        },
        response_buffer,
    )
    .await
    .map_err(PackagePrepareError::Other)?;

    if (400..500).contains(&response.status) {
        return Err(PackagePrepareError::Rejected(response.status));
    }
    if response.status != 200 {
        return Err(PackagePrepareError::Other(BackendError::InvalidResponse));
    }

    content_storage::parse_reader_content_bytes(response.body.as_bytes())
        .map_err(map_storage_prepare_error)
}

async fn stream_https_response_body_to_storage(
    stack: Stack<'static>,
    tls: TlsReference<'_>,
    ca_chain: &Certificate<'static>,
    tcp_state: &BackendTcpClientState,
    request: HttpRequest<'_>,
) -> Result<u16, BackendError> {
    let mut metrics = RequestMetrics::new(false, true);
    let mut tcp_client = TcpClient::new(stack, tcp_state);
    tcp_client.set_timeout(Some(Duration::from_secs(HTTP_BODY_TIMEOUT_SECS)));

    let dns = DnsSocket::new(stack);
    let dns_started_ms = now_ms();
    let remote_addr = dns
        .get_host_by_name(BACKEND_HOST, AddrType::IPv4)
        .await
        .map_err(|_| {
            info!("backend request dns failed path={}", request.path);
            log_request_heap(request.path, "stream dns failed");
            BackendError::Dns
        })?;
    metrics.dns_ms = elapsed_since_ms(dns_started_ms);
    let remote_addr = match remote_addr {
        IpAddr::V4(addr) => addr,
        IpAddr::V6(_) => {
            info!("backend request dns returned ipv6 path={}", request.path);
            log_request_heap(request.path, "stream dns ipv6");
            return Err(BackendError::Dns);
        }
    };
    let connect_started_ms = now_ms();
    let connection = with_timeout(
        Duration::from_secs(CONNECT_TIMEOUT_SECS),
        tcp_client.connect(SocketAddr::new(IpAddr::V4(remote_addr), BACKEND_PORT)),
    )
    .await
    .map_err(|_| {
        info!("backend request connect timed out path={}", request.path);
        log_request_heap(request.path, "stream connect timeout");
        BackendError::Connect
    })?
    .map_err(|_| {
        info!("backend request connect failed path={}", request.path);
        log_request_heap(request.path, "stream connect failed");
        BackendError::Connect
    })?;
    metrics.connect_ms = elapsed_since_ms(connect_started_ms);
    let mut session = open_tls_session(tls, ca_chain, CompatConnection::new(connection))
        .inspect_err(|_err| {
            info!("backend request tls setup failed path={}", request.path);
            log_request_heap(request.path, "stream tls setup failed");
        })?;
    let tls_started_ms = now_ms();
    await_tls_handshake(&mut session, request.path).await?;
    metrics.tls_ms = elapsed_since_ms(tls_started_ms);
    let verification_flags = session.tls_verification_details();
    if verification_flags != 0 {
        info!(
            "backend request tls verification flags path={} flags=0x{:08x}",
            request.path, verification_flags
        );
    }
    write_http_request(
        &mut session,
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
    )
    .await;
    if let Err(err) = session.close().await {
        info!("backend tls close failed: {:?}", err);
    }

    match response {
        Ok(status) => {
            metrics.finish();
            log_request_timing(request, status, &metrics);
            Ok(status)
        }
        Err(err) => {
            log_request_heap(request.path, "stream failed");
            Err(err)
        }
    }
}

async fn stream_https_response_body_to_storage_over_session<T>(
    session: &mut Session<'_, T>,
    request: HttpRequest<'_>,
) -> Result<u16, BackendError>
where
    T: AsyncRead07 + AsyncWrite07,
{
    stream_https_response_body_to_storage_over_session_with_metrics(
        session,
        request,
        RequestMetrics::new(true, true),
    )
    .await
}

async fn stream_https_response_body_to_storage_over_session_with_metrics<T>(
    session: &mut Session<'_, T>,
    request: HttpRequest<'_>,
    mut metrics: RequestMetrics,
) -> Result<u16, BackendError>
where
    T: AsyncRead07 + AsyncWrite07,
{
    write_http_request(
        session,
        request.path,
        request.method,
        request.content_type,
        request.bearer_token,
        request.body,
        request.connection_close,
    )
    .await?;

    let response = read_streaming_http_response_to_storage(
        session,
        request.path,
        request.connection_close,
        &mut metrics,
    )
    .await;
    match response {
        Ok(status) => {
            metrics.finish();
            log_request_timing(request, status, &metrics);
            Ok(status)
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
) -> Result<u16, BackendError>
where
    T: AsyncRead07 + AsyncWrite07,
{
    let mut header = [0u8; HTTP_STREAM_HEADER_MAX_LEN];
    let mut header_len = 0usize;
    let mut chunk = [0u8; PACKAGE_DOWNLOAD_CHUNK_LEN];

    loop {
        if header_len == header.len() {
            return Err(BackendError::ResponseTooLarge);
        }

        let read = await_body_io_timeout(
            path,
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
        if metadata.status != 200 {
            return Ok(metadata.status);
        }

        let initial_body_len = header_len.saturating_sub(body_start);
        match metadata.content_length {
            Some(content_length) => {
                if initial_body_len > content_length {
                    return Err(BackendError::InvalidResponse);
                }
                if initial_body_len > 0 {
                    content_storage::write_package_chunk(&header[body_start..header_len])
                        .await
                        .map_err(map_storage_backend_error)?;
                }

                let mut remaining = content_length - initial_body_len;
                while remaining > 0 {
                    let read_len = remaining.min(chunk.len());
                    let read = await_body_io_timeout(
                        path,
                        "stream response body read",
                        AsyncRead07::read(session, &mut chunk[..read_len]),
                    )
                    .await?;
                    if read == 0 {
                        return Err(BackendError::InvalidResponse);
                    }
                    content_storage::write_package_chunk(&chunk[..read])
                        .await
                        .map_err(map_storage_backend_error)?;
                    remaining -= read;
                }
                return Ok(200);
            }
            None if connection_close => {
                if initial_body_len > 0 {
                    content_storage::write_package_chunk(&header[body_start..header_len])
                        .await
                        .map_err(map_storage_backend_error)?;
                }
                break;
            }
            None => return Err(BackendError::InvalidResponse),
        }
    }

    loop {
        let read = await_body_io_timeout(
            path,
            "stream response body read",
            AsyncRead07::read(session, &mut chunk),
        )
        .await?;
        if read == 0 {
            break;
        }
        content_storage::write_package_chunk(&chunk[..read])
            .await
            .map_err(map_storage_backend_error)?;
    }

    Ok(200)
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
) -> Result<CollectionFetchResult, CollectionQueryError>
where
    T: AsyncRead07 + AsyncWrite07,
{
    let response_buffer = standard_response_buffer();
    let response = send_https_request_over_session(
        session,
        HttpRequest {
            method: "GET",
            path: SAVED_CONTENT_PATH,
            content_type: Some("application/json"),
            bearer_token: Some(access_token),
            body: b"",
            connection_close,
        },
        response_buffer,
    )
    .await
    .map_err(CollectionQueryError::Other)?;

    if (400..500).contains(&response.status) {
        return Err(CollectionQueryError::Rejected(response.status));
    }
    if response.status != 200 {
        return Err(CollectionQueryError::Other(BackendError::InvalidResponse));
    }

    parse_saved_content_fetch_result(response.body).map_err(CollectionQueryError::Other)
}

async fn publish_package_state(
    collection: CollectionKind,
    remote_item_id: InlineText<REMOTE_ITEM_ID_MAX_BYTES>,
    package_state: PackageState,
) -> Result<(), StorageError> {
    match content_storage::update_package_state(collection, remote_item_id, package_state).await {
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

fn build_detail_path(
    request: PrepareContentRequest,
) -> Result<heapless::String<128>, BackendError> {
    let mut path = heapless::String::<128>::new();
    match request.detail_locator {
        DetailLocator::Saved => {
            path.push_str("/me/saved-content/")
                .map_err(|_| BackendError::ResponseTooLarge)?;
            path.push_str(request.remote_item_id.as_str())
                .map_err(|_| BackendError::ResponseTooLarge)?;
        }
        DetailLocator::Inbox => {
            path.push_str("/me/inbox/")
                .map_err(|_| BackendError::ResponseTooLarge)?;
            path.push_str(request.remote_item_id.as_str())
                .map_err(|_| BackendError::ResponseTooLarge)?;
        }
        DetailLocator::Content => {
            path.push_str("/me/content/")
                .map_err(|_| BackendError::ResponseTooLarge)?;
            path.push_str(request.content_id.as_str())
                .map_err(|_| BackendError::ResponseTooLarge)?;
        }
    }

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
    let (body_preview, body_preview_truncated) = if item_count > 0 {
        let (preview, truncated) = utf8_log_prefix(body, INBOX_LOG_PREVIEW_MAX_LEN);
        (Some(bounded_string(preview)?), truncated)
    } else {
        (None, false)
    };

    Ok(CollectionFetchSummary {
        item_count,
        next_cursor_present,
        body_preview,
        body_preview_truncated,
    })
}

fn parse_inbox_fetch_result(body: &str) -> Result<CollectionFetchResult, BackendError> {
    let collection = parse_inbox_collection(body)?;
    let next_cursor_present = extract_json_optional_string(body, "\"next_cursor\"")
        .ok_or(BackendError::MissingField)?
        .is_some();
    let (body_preview, body_preview_truncated) = if collection.is_empty() {
        (None, false)
    } else {
        let (preview, truncated) = utf8_log_prefix(body, INBOX_LOG_PREVIEW_MAX_LEN);
        (Some(bounded_string(preview)?), truncated)
    };

    Ok(CollectionFetchResult {
        summary: CollectionFetchSummary {
            item_count: collection.len(),
            next_cursor_present,
            body_preview,
            body_preview_truncated,
        },
        collection,
    })
}

fn parse_saved_content_fetch_result(body: &str) -> Result<CollectionFetchResult, BackendError> {
    let collection = parse_saved_content_collection(body)?;
    let next_cursor_present = extract_json_optional_string(body, "\"next_cursor\"")
        .ok_or(BackendError::MissingField)?
        .is_some();
    let (body_preview, body_preview_truncated) = if collection.is_empty() {
        (None, false)
    } else {
        let (preview, truncated) = utf8_log_prefix(body, INBOX_LOG_PREVIEW_MAX_LEN);
        (Some(bounded_string(preview)?), truncated)
    };

    Ok(CollectionFetchResult {
        summary: CollectionFetchSummary {
            item_count: collection.len(),
            next_cursor_present,
            body_preview,
            body_preview_truncated,
        },
        collection,
    })
}

fn parse_recommendation_fetch_result(body: &str) -> Result<CollectionFetchResult, BackendError> {
    let collection = parse_recommendation_collection(body)?;
    let next_cursor_present = extract_json_optional_string(body, "\"next_cursor\"")
        .unwrap_or(None)
        .is_some();
    let (body_preview, body_preview_truncated) = if collection.is_empty() {
        (None, false)
    } else {
        let (preview, truncated) = utf8_log_prefix(body, INBOX_LOG_PREVIEW_MAX_LEN);
        (Some(bounded_string(preview)?), truncated)
    };

    Ok(CollectionFetchResult {
        summary: CollectionFetchSummary {
            item_count: collection.len(),
            next_cursor_present,
            body_preview,
            body_preview_truncated,
        },
        collection,
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
    let stats = esp_alloc::HEAP.stats();
    info!(
        "heap label={} size={} used={} free={}",
        label,
        stats.size,
        stats.current_usage,
        stats.size.saturating_sub(stats.current_usage),
    );
}

fn log_request_heap(path: &str, stage: &str) {
    let stats = esp_alloc::HEAP.stats();
    info!(
        "backend heap path={} stage={} size={} used={} free={}",
        path,
        stage,
        stats.size,
        stats.current_usage,
        stats.size.saturating_sub(stats.current_usage),
    );
}

fn now_ms() -> u64 {
    Instant::now().as_millis()
}

fn elapsed_since_ms(started_ms: u64) -> u64 {
    now_ms().saturating_sub(started_ms)
}

fn log_request_timing(request: HttpRequest<'_>, status: u16, metrics: &RequestMetrics) {
    info!(
        "backend request timing method={} path={} status={} reused={} streaming={} dns_ms={} connect_ms={} tls_ms={} first_byte_ms={} total_ms={}",
        request.method,
        request.path,
        status,
        metrics.reused_session,
        metrics.streaming,
        metrics.dns_ms,
        metrics.connect_ms,
        metrics.tls_ms,
        metrics.first_byte_ms.unwrap_or(metrics.total_ms),
        metrics.total_ms,
    );
}

const fn is_transient_transport_error(error: BackendError) -> bool {
    matches!(
        error,
        BackendError::Dns | BackendError::Connect | BackendError::Tls | BackendError::Io
    )
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
    }

    #[test]
    fn parses_http_response_metadata_with_chunked_transfer_encoding() {
        let response =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: keep-alive\r\n\r\n";
        let metadata = parse_http_response_metadata(response).unwrap();

        assert_eq!(metadata.status, 200);
        assert!(metadata.chunked);
        assert_eq!(metadata.content_length, None);
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
}
