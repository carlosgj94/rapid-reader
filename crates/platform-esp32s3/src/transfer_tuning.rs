use log::info;

pub const PACKAGE_TRANSFER_CHUNK_LEN_OVERRIDE_ENV: &str = "MOTIF_PACKAGE_TRANSFER_CHUNK_LEN";

const PACKAGE_TRANSFER_PRODUCT_CHUNK_LEN: usize = 64 * 1024;
const PACKAGE_TRANSFER_MIN_CHUNK_LEN: usize = 8 * 1024;
const PACKAGE_TRANSFER_MAX_CHUNK_LEN: usize = 64 * 1024;
const PACKAGE_TRANSFER_FLUSH_MULTIPLIER: usize = 2;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PackageTransferConfig {
    pub chunk_len: usize,
    pub flush_interval_bytes: u32,
    pub source: &'static str,
}

pub const PACKAGE_TRANSFER_CONFIG: PackageTransferConfig = resolve_package_transfer_config();
pub const PACKAGE_TRANSFER_CHUNK_LEN: usize = PACKAGE_TRANSFER_CONFIG.chunk_len;
pub const PACKAGE_TRANSFER_FLUSH_INTERVAL_BYTES: u32 = PACKAGE_TRANSFER_CONFIG.flush_interval_bytes;
pub const PACKAGE_TRANSFER_SOURCE: &str = PACKAGE_TRANSFER_CONFIG.source;

const fn default_package_transfer_config() -> PackageTransferConfig {
    PackageTransferConfig {
        chunk_len: PACKAGE_TRANSFER_PRODUCT_CHUNK_LEN,
        flush_interval_bytes: flush_interval_bytes_for_chunk(PACKAGE_TRANSFER_PRODUCT_CHUNK_LEN),
        source: "product_default",
    }
}

const fn resolve_package_transfer_config() -> PackageTransferConfig {
    let (config, _) =
        resolve_package_transfer_config_from(option_env!("MOTIF_PACKAGE_TRANSFER_CHUNK_LEN"));
    config
}

pub const fn resolve_package_transfer_config_from(
    override_raw: Option<&str>,
) -> (PackageTransferConfig, Option<&str>) {
    match override_raw {
        Some(raw) => match parse_chunk_len(raw) {
            Some(chunk_len) => (
                PackageTransferConfig {
                    chunk_len,
                    flush_interval_bytes: flush_interval_bytes_for_chunk(chunk_len),
                    source: "build_override",
                },
                None,
            ),
            None => (default_package_transfer_config(), Some(raw)),
        },
        None => (default_package_transfer_config(), None),
    }
}

pub fn log_runtime_config() {
    let (config, invalid_raw) =
        resolve_package_transfer_config_from(option_env!("MOTIF_PACKAGE_TRANSFER_CHUNK_LEN"));
    if let Some(raw) = invalid_raw {
        info!(
            "package transfer override invalid env={} raw={} defaulting_to_chunk_len={} flush_interval_bytes={}",
            PACKAGE_TRANSFER_CHUNK_LEN_OVERRIDE_ENV,
            raw,
            config.chunk_len,
            config.flush_interval_bytes,
        );
    } else if let Some(raw) = option_env!("MOTIF_PACKAGE_TRANSFER_CHUNK_LEN") {
        info!(
            "package transfer override accepted env={} raw={} chunk_len={} flush_interval_bytes={}",
            PACKAGE_TRANSFER_CHUNK_LEN_OVERRIDE_ENV,
            raw,
            config.chunk_len,
            config.flush_interval_bytes,
        );
    }

    info!(
        "package transfer config chunk_len={} flush_interval_bytes={} source={} min_chunk_len={} max_chunk_len={}",
        config.chunk_len,
        config.flush_interval_bytes,
        config.source,
        PACKAGE_TRANSFER_MIN_CHUNK_LEN,
        PACKAGE_TRANSFER_MAX_CHUNK_LEN,
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

const fn flush_interval_bytes_for_chunk(chunk_len: usize) -> u32 {
    (chunk_len * PACKAGE_TRANSFER_FLUSH_MULTIPLIER) as u32
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
    fn default_transfer_config_is_aggressive_but_bounded() {
        let (config, invalid_raw) = resolve_package_transfer_config_from(None);
        assert_eq!(config.chunk_len, 64 * 1024);
        assert_eq!(config.flush_interval_bytes, 128 * 1024);
        assert_eq!(config.source, "product_default");
        assert_eq!(invalid_raw, None);
    }

    #[test]
    fn valid_override_uses_requested_chunk_len() {
        let (config, invalid_raw) = resolve_package_transfer_config_from(Some("65536"));
        assert_eq!(config.chunk_len, 64 * 1024);
        assert_eq!(config.flush_interval_bytes, 128 * 1024);
        assert_eq!(config.source, "build_override");
        assert_eq!(invalid_raw, None);
    }

    #[test]
    fn invalid_override_falls_back_to_default() {
        let (config, invalid_raw) = resolve_package_transfer_config_from(Some("12345"));
        assert_eq!(config.chunk_len, 64 * 1024);
        assert_eq!(config.flush_interval_bytes, 128 * 1024);
        assert_eq!(config.source, "product_default");
        assert_eq!(invalid_raw, Some("12345"));
    }

    #[test]
    fn oversized_override_is_rejected() {
        let (config, invalid_raw) = resolve_package_transfer_config_from(Some("131072"));
        assert_eq!(config.chunk_len, 64 * 1024);
        assert_eq!(config.flush_interval_bytes, 128 * 1024);
        assert_eq!(config.source, "product_default");
        assert_eq!(invalid_raw, Some("131072"));
    }
}
