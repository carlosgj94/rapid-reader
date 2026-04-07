use log::info;

pub const PACKAGE_TRANSFER_CHUNK_LEN_OVERRIDE_ENV: &str = "MOTIF_PACKAGE_TRANSFER_CHUNK_LEN";

const PACKAGE_TRANSFER_PRODUCT_CHUNK_LEN: usize = 128 * 1024;
const PACKAGE_TRANSFER_PRODUCT_STORAGE_HANDOFF_CHUNK_LEN: usize = 64 * 1024;
const PACKAGE_TRANSFER_MIN_CHUNK_LEN: usize = 8 * 1024;
const PACKAGE_TRANSFER_MAX_CHUNK_LEN: usize = 128 * 1024;
const PACKAGE_TRANSFER_FLUSH_MULTIPLIER: usize = 2;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PackageTransferConfig {
    pub chunk_len: usize,
    pub storage_handoff_chunk_len: usize,
    pub flush_interval_bytes: u32,
    pub source: &'static str,
}

pub const PACKAGE_TRANSFER_CONFIG: PackageTransferConfig = resolve_package_transfer_config();
pub const PACKAGE_TRANSFER_CHUNK_LEN: usize = PACKAGE_TRANSFER_CONFIG.chunk_len;
pub const PACKAGE_TRANSFER_STORAGE_HANDOFF_CHUNK_LEN: usize =
    PACKAGE_TRANSFER_CONFIG.storage_handoff_chunk_len;
pub const PACKAGE_TRANSFER_FLUSH_INTERVAL_BYTES: u32 = PACKAGE_TRANSFER_CONFIG.flush_interval_bytes;
pub const PACKAGE_TRANSFER_SOURCE: &str = PACKAGE_TRANSFER_CONFIG.source;

const fn default_package_transfer_config() -> PackageTransferConfig {
    let storage_handoff_chunk_len =
        storage_handoff_chunk_len_for_receive(PACKAGE_TRANSFER_PRODUCT_CHUNK_LEN);
    PackageTransferConfig {
        chunk_len: PACKAGE_TRANSFER_PRODUCT_CHUNK_LEN,
        storage_handoff_chunk_len,
        flush_interval_bytes: flush_interval_bytes_for_handoff(storage_handoff_chunk_len),
        source: "product_default",
    }
}

const fn resolve_package_transfer_config() -> PackageTransferConfig {
    let (config, _) =
        resolve_package_transfer_config_from(option_env!("MOTIF_PACKAGE_TRANSFER_CHUNK_LEN"));
    config
}

pub const fn resolve_package_transfer_config_from(
    chunk_override_raw: Option<&str>,
) -> (PackageTransferConfig, Option<&str>) {
    let default = default_package_transfer_config();
    let (chunk_len, invalid_chunk_raw, source) = match chunk_override_raw {
        Some(raw) => match parse_chunk_len(raw) {
            Some(chunk_len) => (chunk_len, None, "build_override"),
            None => (default.chunk_len, Some(raw), default.source),
        },
        None => (default.chunk_len, None, default.source),
    };
    let storage_handoff_chunk_len = storage_handoff_chunk_len_for_receive(chunk_len);
    (
        PackageTransferConfig {
            chunk_len,
            storage_handoff_chunk_len,
            flush_interval_bytes: flush_interval_bytes_for_handoff(storage_handoff_chunk_len),
            source,
        },
        invalid_chunk_raw,
    )
}

pub fn log_runtime_config() {
    let (config, invalid_chunk_raw) =
        resolve_package_transfer_config_from(option_env!("MOTIF_PACKAGE_TRANSFER_CHUNK_LEN"));
    if let Some(raw) = invalid_chunk_raw {
        info!(
            "package transfer override invalid env={} raw={} defaulting_to_chunk_len={} defaulting_to_storage_handoff_chunk_len={} flush_interval_bytes={}",
            PACKAGE_TRANSFER_CHUNK_LEN_OVERRIDE_ENV,
            raw,
            config.chunk_len,
            config.storage_handoff_chunk_len,
            config.flush_interval_bytes,
        );
    } else if let Some(raw) = option_env!("MOTIF_PACKAGE_TRANSFER_CHUNK_LEN") {
        info!(
            "package transfer override accepted env={} raw={} chunk_len={} storage_handoff_chunk_len={} flush_interval_bytes={}",
            PACKAGE_TRANSFER_CHUNK_LEN_OVERRIDE_ENV,
            raw,
            config.chunk_len,
            config.storage_handoff_chunk_len,
            config.flush_interval_bytes,
        );
    }

    info!(
        "package transfer config chunk_len={} storage_handoff_chunk_len={} flush_interval_bytes={} source={} min_chunk_len={} max_chunk_len={} product_storage_handoff_chunk_len={}",
        config.chunk_len,
        config.storage_handoff_chunk_len,
        config.flush_interval_bytes,
        config.source,
        PACKAGE_TRANSFER_MIN_CHUNK_LEN,
        PACKAGE_TRANSFER_MAX_CHUNK_LEN,
        PACKAGE_TRANSFER_PRODUCT_STORAGE_HANDOFF_CHUNK_LEN,
    );
}

const fn parse_chunk_len(raw: &str) -> Option<usize> {
    match parse_positive_usize(raw) {
        Some(value) if is_valid_chunk_len(value) => Some(value),
        _ => None,
    }
}

const fn is_valid_chunk_len(value: usize) -> bool {
    value >= PACKAGE_TRANSFER_MIN_CHUNK_LEN
        && value <= PACKAGE_TRANSFER_MAX_CHUNK_LEN
        && (value & 1023) == 0
}

const fn storage_handoff_chunk_len_for_receive(chunk_len: usize) -> usize {
    if chunk_len > PACKAGE_TRANSFER_PRODUCT_STORAGE_HANDOFF_CHUNK_LEN {
        PACKAGE_TRANSFER_PRODUCT_STORAGE_HANDOFF_CHUNK_LEN
    } else {
        chunk_len
    }
}

const fn flush_interval_bytes_for_handoff(storage_handoff_chunk_len: usize) -> u32 {
    (storage_handoff_chunk_len * PACKAGE_TRANSFER_FLUSH_MULTIPLIER) as u32
}

const fn parse_positive_usize(raw: &str) -> Option<usize> {
    let bytes = raw.as_bytes();
    if bytes.is_empty() {
        return None;
    }

    let mut index = 0usize;
    let mut value = 0usize;
    while index < bytes.len() {
        let byte = bytes[index];
        if !(byte >= b'0' && byte <= b'9') {
            return None;
        }
        let digit = (byte - b'0') as usize;
        if value > (usize::MAX - digit) / 10 {
            return None;
        }
        value = (value * 10) + digit;
        index += 1;
    }

    if value == 0 { None } else { Some(value) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_transfer_config_is_large_and_fixed() {
        let (config, invalid_chunk_raw) = resolve_package_transfer_config_from(None);
        assert_eq!(config.chunk_len, 128 * 1024);
        assert_eq!(config.storage_handoff_chunk_len, 64 * 1024);
        assert_eq!(config.flush_interval_bytes, 128 * 1024);
        assert_eq!(config.source, "product_default");
        assert_eq!(invalid_chunk_raw, None);
    }

    #[test]
    fn valid_override_uses_requested_chunk_len() {
        let (config, invalid_chunk_raw) = resolve_package_transfer_config_from(Some("32768"));
        assert_eq!(config.chunk_len, 32 * 1024);
        assert_eq!(config.storage_handoff_chunk_len, 32 * 1024);
        assert_eq!(config.flush_interval_bytes, 64 * 1024);
        assert_eq!(config.source, "build_override");
        assert_eq!(invalid_chunk_raw, None);
    }

    #[test]
    fn max_override_keeps_fixed_storage_handoff() {
        let (config, invalid_chunk_raw) = resolve_package_transfer_config_from(Some("131072"));
        assert_eq!(config.chunk_len, 128 * 1024);
        assert_eq!(config.storage_handoff_chunk_len, 64 * 1024);
        assert_eq!(config.flush_interval_bytes, 128 * 1024);
        assert_eq!(config.source, "build_override");
        assert_eq!(invalid_chunk_raw, None);
    }

    #[test]
    fn invalid_override_falls_back_to_default() {
        let (config, invalid_chunk_raw) = resolve_package_transfer_config_from(Some("12345"));
        assert_eq!(config.chunk_len, 128 * 1024);
        assert_eq!(config.storage_handoff_chunk_len, 64 * 1024);
        assert_eq!(config.flush_interval_bytes, 128 * 1024);
        assert_eq!(config.source, "product_default");
        assert_eq!(invalid_chunk_raw, Some("12345"));
    }

    #[test]
    fn oversized_override_is_rejected() {
        let (config, invalid_chunk_raw) = resolve_package_transfer_config_from(Some("262144"));
        assert_eq!(config.chunk_len, 128 * 1024);
        assert_eq!(config.storage_handoff_chunk_len, 64 * 1024);
        assert_eq!(config.flush_interval_bytes, 128 * 1024);
        assert_eq!(config.source, "product_default");
        assert_eq!(invalid_chunk_raw, Some("262144"));
    }
}
