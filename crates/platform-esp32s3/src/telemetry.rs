use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use domain::content::CollectionKind;
use esp_alloc::{MemoryCapability, RegionStats};

pub const MEMTRACE_VERSION: u32 = 1;

static NEXT_EVENT_ID: AtomicU32 = AtomicU32::new(1);
static NEXT_SYNC_ID: AtomicU32 = AtomicU32::new(1);
static NEXT_REQUEST_ID: AtomicU32 = AtomicU32::new(1);
static INTERNAL_PEAK_USED: AtomicUsize = AtomicUsize::new(0);
static INTERNAL_MIN_FREE: AtomicUsize = AtomicUsize::new(usize::MAX);
static EXTERNAL_PEAK_USED: AtomicUsize = AtomicUsize::new(0);
static EXTERNAL_MIN_FREE: AtomicUsize = AtomicUsize::new(usize::MAX);
static REGION_PEAK_USED: [AtomicUsize; 3] = [const { AtomicUsize::new(0) }; 3];
static REGION_MIN_FREE: [AtomicUsize; 3] = [const { AtomicUsize::new(usize::MAX) }; 3];

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub struct TraceContext {
    pub sync_id: u32,
    pub req_id: u32,
}

impl TraceContext {
    pub const fn none() -> Self {
        Self {
            sync_id: 0,
            req_id: 0,
        }
    }

    pub const fn with_request(self, req_id: u32) -> Self {
        Self {
            sync_id: self.sync_id,
            req_id,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct RegionTelemetry {
    pub kind: &'static str,
    pub size: usize,
    pub used: usize,
    pub free: usize,
    pub peak_used: usize,
    pub min_free: usize,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct HeapTelemetry {
    pub size: usize,
    pub used: usize,
    pub free: usize,
    pub peak: usize,
    pub total_allocated: usize,
    pub total_freed: usize,
    pub internal_regions: usize,
    pub internal_size: usize,
    pub internal_used: usize,
    pub internal_free: usize,
    pub internal_peak_used: usize,
    pub internal_min_free: usize,
    pub external_regions: usize,
    pub external_size: usize,
    pub external_used: usize,
    pub external_free: usize,
    pub external_peak_used: usize,
    pub external_min_free: usize,
    pub regions: [RegionTelemetry; 3],
}

pub const fn bool_flag(value: bool) -> u8 {
    if value { 1 } else { 0 }
}

pub const fn collection_label(kind: CollectionKind) -> &'static str {
    match kind {
        CollectionKind::Saved => "saved",
        CollectionKind::Inbox => "inbox",
        CollectionKind::Recommendations => "recommendations",
    }
}

pub fn next_event_id() -> u32 {
    NEXT_EVENT_ID.fetch_add(1, Ordering::Relaxed)
}

pub fn next_sync_id() -> u32 {
    NEXT_SYNC_ID.fetch_add(1, Ordering::Relaxed)
}

pub fn next_request_id() -> u32 {
    NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed)
}

pub fn capture_heap() -> HeapTelemetry {
    let stats = esp_alloc::HEAP.stats();
    let mut regions = [RegionTelemetry {
        kind: "none",
        size: 0,
        used: 0,
        free: 0,
        peak_used: 0,
        min_free: 0,
    }; 3];
    let mut internal_regions = 0usize;
    let mut internal_size = 0usize;
    let mut internal_used = 0usize;
    let mut internal_free = 0usize;
    let mut external_regions = 0usize;
    let mut external_size = 0usize;
    let mut external_used = 0usize;
    let mut external_free = 0usize;

    let mut index = 0usize;
    while index < stats.region_stats.len() {
        if let Some(region) = stats.region_stats[index].as_ref() {
            regions[index] = map_region(index, region);
            if region.capabilities.contains(MemoryCapability::Internal) {
                internal_regions += 1;
                internal_size += region.size;
                internal_used += region.used;
                internal_free += region.free;
            }
            if region.capabilities.contains(MemoryCapability::External) {
                external_regions += 1;
                external_size += region.size;
                external_used += region.used;
                external_free += region.free;
            }
        }
        index += 1;
    }

    let internal_peak_used = if internal_regions > 0 {
        track_peak(&INTERNAL_PEAK_USED, internal_used)
    } else {
        0
    };
    let internal_min_free = if internal_regions > 0 {
        track_min(&INTERNAL_MIN_FREE, internal_free)
    } else {
        0
    };
    let external_peak_used = if external_regions > 0 {
        track_peak(&EXTERNAL_PEAK_USED, external_used)
    } else {
        0
    };
    let external_min_free = if external_regions > 0 {
        track_min(&EXTERNAL_MIN_FREE, external_free)
    } else {
        0
    };

    HeapTelemetry {
        size: stats.size,
        used: stats.current_usage,
        free: stats.size.saturating_sub(stats.current_usage),
        peak: stats.max_usage,
        total_allocated: stats.total_allocated,
        total_freed: stats.total_freed,
        internal_regions,
        internal_size,
        internal_used,
        internal_free,
        internal_peak_used,
        internal_min_free,
        external_regions,
        external_size,
        external_used,
        external_free,
        external_peak_used,
        external_min_free,
        regions,
    }
}

fn map_region(index: usize, region: &RegionStats) -> RegionTelemetry {
    RegionTelemetry {
        kind: region_kind(region),
        size: region.size,
        used: region.used,
        free: region.free,
        peak_used: track_peak(&REGION_PEAK_USED[index], region.used),
        min_free: track_min(&REGION_MIN_FREE[index], region.free),
    }
}

fn track_peak(atom: &AtomicUsize, value: usize) -> usize {
    let mut current = atom.load(Ordering::Relaxed);
    loop {
        if value <= current {
            return current;
        }
        match atom.compare_exchange_weak(current, value, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return value,
            Err(observed) => current = observed,
        }
    }
}

fn track_min(atom: &AtomicUsize, value: usize) -> usize {
    let mut current = atom.load(Ordering::Relaxed);
    loop {
        if value >= current {
            return current;
        }
        match atom.compare_exchange_weak(current, value, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return value,
            Err(observed) => current = observed,
        }
    }
}

fn region_kind(region: &RegionStats) -> &'static str {
    let internal = region.capabilities.contains(MemoryCapability::Internal);
    let external = region.capabilities.contains(MemoryCapability::External);

    match (internal, external) {
        (true, false) => "internal",
        (false, true) => "external",
        (true, true) => "mixed",
        (false, false) => "unknown",
    }
}

#[cfg(feature = "telemetry-memtrace")]
#[macro_export]
macro_rules! memtrace {
    ($kind:literal $(, $key:literal = $value:expr )* $(,)?) => {{
        let snapshot = $crate::telemetry::capture_heap();
        let event_id = $crate::telemetry::next_event_id();
        log::info!(
            concat!(
                "MEMTRACE v=1 kind=",
                $kind,
                " event_id={}",
                $(" ", $key, "={}"),*,
                " heap_size={} heap_used={} heap_free={} heap_peak={} heap_total_allocated={} heap_total_freed={}",
                " internal_heap_regions={} internal_heap_size={} internal_heap_used={} internal_heap_free={} internal_heap_peak_used={} internal_heap_min_free={}",
                " external_heap_regions={} external_heap_size={} external_heap_used={} external_heap_free={} external_heap_peak_used={} external_heap_min_free={}",
                " region0_kind={} region0_size={} region0_used={} region0_free={} region0_peak_used={} region0_min_free={}",
                " region1_kind={} region1_size={} region1_used={} region1_free={} region1_peak_used={} region1_min_free={}",
                " region2_kind={} region2_size={} region2_used={} region2_free={} region2_peak_used={} region2_min_free={}"
            ),
            event_id,
            $($value,)*
            snapshot.size,
            snapshot.used,
            snapshot.free,
            snapshot.peak,
            snapshot.total_allocated,
            snapshot.total_freed,
            snapshot.internal_regions,
            snapshot.internal_size,
            snapshot.internal_used,
            snapshot.internal_free,
            snapshot.internal_peak_used,
            snapshot.internal_min_free,
            snapshot.external_regions,
            snapshot.external_size,
            snapshot.external_used,
            snapshot.external_free,
            snapshot.external_peak_used,
            snapshot.external_min_free,
            snapshot.regions[0].kind,
            snapshot.regions[0].size,
            snapshot.regions[0].used,
            snapshot.regions[0].free,
            snapshot.regions[0].peak_used,
            snapshot.regions[0].min_free,
            snapshot.regions[1].kind,
            snapshot.regions[1].size,
            snapshot.regions[1].used,
            snapshot.regions[1].free,
            snapshot.regions[1].peak_used,
            snapshot.regions[1].min_free,
            snapshot.regions[2].kind,
            snapshot.regions[2].size,
            snapshot.regions[2].used,
            snapshot.regions[2].free,
            snapshot.regions[2].peak_used,
            snapshot.regions[2].min_free,
        );
    }};
}

#[cfg(not(feature = "telemetry-memtrace"))]
#[macro_export]
macro_rules! memtrace {
    ($($tt:tt)*) => {{}};
}

#[cfg(feature = "telemetry-verbose-diagnostics")]
#[macro_export]
macro_rules! verbose_diag {
    ($($arg:tt)*) => {{
        log::info!($($arg)*);
    }};
}

#[cfg(not(feature = "telemetry-verbose-diagnostics"))]
#[macro_export]
macro_rules! verbose_diag {
    ($($tt:tt)*) => {{}};
}
