use allocator_api2::{
    alloc::AllocError, boxed::Box as AllocBox, collections::TryReserveError, vec::Vec as AllocVec,
};
use core::pin::Pin;
use embassy_time::Instant;
use log::info;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum MemoryPlacement {
    InternalOnly,
    ExternalPreferred,
    LegacyGlobalDefault,
}

impl MemoryPlacement {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InternalOnly => "internal_only",
            Self::ExternalPreferred => "external_preferred",
            Self::LegacyGlobalDefault => "legacy_global_default",
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PolicyRule {
    pub class: &'static str,
    pub placement: MemoryPlacement,
    pub rationale: &'static str,
}

pub const LEGACY_GLOBAL_DEFAULT: PolicyRule = PolicyRule {
    class: "legacy_unreviewed_allocations",
    placement: MemoryPlacement::LegacyGlobalDefault,
    rationale: "keep behavior unchanged until each path is audited and migrated intentionally",
};

pub const INTERNAL_ONLY_RULES: &[PolicyRule] = &[
    PolicyRule {
        class: "wifi_radio_critical_paths",
        placement: MemoryPlacement::InternalOnly,
        rationale: "latency-sensitive vendor and radio state must stay in internal RAM",
    },
    PolicyRule {
        class: "tls_session_internals",
        placement: MemoryPlacement::InternalOnly,
        rationale: "transport-state placement remains internal until explicitly audited",
    },
    PolicyRule {
        class: "atomic_heavy_structures",
        placement: MemoryPlacement::InternalOnly,
        rationale: "heap-allocating atomics in PSRAM is not considered safe on esp32s3",
    },
    PolicyRule {
        class: "dma_sensitive_buffers",
        placement: MemoryPlacement::InternalOnly,
        rationale: "DMA-capable buffers require internal-capable memory",
    },
];

pub const EXTERNAL_PREFERRED_RULES: &[PolicyRule] = &[
    PolicyRule {
        class: "article_package_buffers",
        placement: MemoryPlacement::ExternalPreferred,
        rationale: "large sequential content buffers are good PSRAM candidates",
    },
    PolicyRule {
        class: "article_parse_scratch",
        placement: MemoryPlacement::ExternalPreferred,
        rationale: "bulk parsing scratch is large and not transport-critical",
    },
    PolicyRule {
        class: "cache_working_sets",
        placement: MemoryPlacement::ExternalPreferred,
        rationale: "read-mostly cache state can move out of scarce internal RAM",
    },
    PolicyRule {
        class: "bounded_response_buffers",
        placement: MemoryPlacement::ExternalPreferred,
        rationale: "application-layer bounded buffers need not compete with TLS internals",
    },
];

pub type ExternalVec<T> = AllocVec<T, esp_alloc::ExternalMemory>;
pub type InternalVec<T> = AllocVec<T, esp_alloc::InternalMemory>;
pub type ExternalBox<T> = AllocBox<T, esp_alloc::ExternalMemory>;
pub type InternalBox<T> = AllocBox<T, esp_alloc::InternalMemory>;
pub type PinnedExternalBox<T> = Pin<ExternalBox<T>>;
pub type PinnedInternalBox<T> = Pin<InternalBox<T>>;

pub fn new_external_vec<T>() -> ExternalVec<T> {
    AllocVec::new_in(esp_alloc::ExternalMemory)
}

pub fn try_external_vec_with_capacity<T>(
    capacity: usize,
) -> Result<ExternalVec<T>, TryReserveError> {
    let mut vec = new_external_vec();
    vec.try_reserve(capacity)?;
    Ok(vec)
}

pub fn new_internal_vec<T>() -> InternalVec<T> {
    AllocVec::new_in(esp_alloc::InternalMemory)
}

pub fn try_internal_vec_with_capacity<T>(
    capacity: usize,
) -> Result<InternalVec<T>, TryReserveError> {
    let mut vec = new_internal_vec();
    vec.try_reserve(capacity)?;
    Ok(vec)
}

pub fn try_external_box<T>(value: T) -> Result<ExternalBox<T>, AllocError> {
    AllocBox::try_new_in(value, esp_alloc::ExternalMemory)
}

pub fn try_internal_box<T>(value: T) -> Result<InternalBox<T>, AllocError> {
    AllocBox::try_new_in(value, esp_alloc::InternalMemory)
}

pub fn try_external_pinned_box<T>(value: T) -> Result<PinnedExternalBox<T>, AllocError> {
    Ok(AllocBox::into_pin(try_external_box(value)?))
}

pub fn try_internal_pinned_box<T>(value: T) -> Result<PinnedInternalBox<T>, AllocError> {
    Ok(AllocBox::into_pin(try_internal_box(value)?))
}

pub fn try_external_zeroed_array_box<const N: usize>() -> Result<ExternalBox<[u8; N]>, AllocError> {
    let boxed = AllocBox::<[u8; N], _>::try_new_zeroed_in(esp_alloc::ExternalMemory)?;
    // SAFETY: `[u8; N]` is valid for an all-zero byte pattern.
    Ok(unsafe { boxed.assume_init() })
}

pub fn try_internal_zeroed_array_box<const N: usize>() -> Result<InternalBox<[u8; N]>, AllocError> {
    let boxed = AllocBox::<[u8; N], _>::try_new_zeroed_in(esp_alloc::InternalMemory)?;
    // SAFETY: `[u8; N]` is valid for an all-zero byte pattern.
    Ok(unsafe { boxed.assume_init() })
}

pub(crate) fn log_policy_inventory() {
    info!(
        "memory policy default={} internal_only_rules={} external_preferred_rules={} helper_external_vec=1 helper_internal_vec=1 helper_external_box=1 helper_internal_box=1 helper_external_pinned_box=1 helper_internal_pinned_box=1 helper_external_zeroed_array_box=1 helper_internal_zeroed_array_box=1",
        LEGACY_GLOBAL_DEFAULT.placement.as_str(),
        INTERNAL_ONLY_RULES.len(),
        EXTERNAL_PREFERRED_RULES.len(),
    );
    info!(
        "memory policy default_rule class={} rationale={}",
        LEGACY_GLOBAL_DEFAULT.class, LEGACY_GLOBAL_DEFAULT.rationale,
    );
    for rule in INTERNAL_ONLY_RULES {
        info!(
            "memory policy rule class={} placement={} rationale={}",
            rule.class,
            rule.placement.as_str(),
            rule.rationale,
        );
    }
    for rule in EXTERNAL_PREFERRED_RULES {
        info!(
            "memory policy rule class={} placement={} rationale={}",
            rule.class,
            rule.placement.as_str(),
            rule.rationale,
        );
    }
    crate::memtrace!(
        "static_inventory",
        "component" = "memory_policy",
        "at_ms" = Instant::now().as_millis(),
        "legacy_default_policy" = LEGACY_GLOBAL_DEFAULT.placement.as_str(),
        "internal_only_rule_count" = INTERNAL_ONLY_RULES.len(),
        "external_preferred_rule_count" = EXTERNAL_PREFERRED_RULES.len(),
        "helper_external_vec" = 1,
        "helper_internal_vec" = 1,
        "helper_external_box" = 1,
        "helper_internal_box" = 1,
        "helper_external_pinned_box" = 1,
        "helper_internal_pinned_box" = 1,
        "helper_external_zeroed_array_box" = 1,
        "helper_internal_zeroed_array_box" = 1,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_counts_match_expectations() {
        assert_eq!(INTERNAL_ONLY_RULES.len(), 4);
        assert_eq!(EXTERNAL_PREFERRED_RULES.len(), 4);
        assert_eq!(
            INTERNAL_ONLY_RULES
                .iter()
                .filter(|rule| rule.placement == MemoryPlacement::InternalOnly)
                .count(),
            INTERNAL_ONLY_RULES.len()
        );
        assert_eq!(
            EXTERNAL_PREFERRED_RULES
                .iter()
                .filter(|rule| rule.placement == MemoryPlacement::ExternalPreferred)
                .count(),
            EXTERNAL_PREFERRED_RULES.len()
        );
    }
}
