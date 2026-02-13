use core::{ffi::c_void, str};

use embedded_hal::{
    delay::DelayNs,
    digital::OutputPin,
    spi::{Error as SpiErrorTrait, ErrorKind, ErrorType, Operation, SpiBus, SpiDevice},
};
use embedded_sdmmc::{
    Mode, SdCard, SdCardError, ShortFileName, TimeSource, Timestamp, VolumeIdx, VolumeManager,
};
use heapless::{String, Vec};
use log::info;
use miniz_oxide::inflate::{
    TINFLStatus,
    core::{DecompressorOxide, decompress, inflate_flags},
    stream::{InflateState, inflate},
};
use miniz_oxide::{DataFormat, MZError, MZFlush, MZStatus};

/// Fixed timestamp source used until RTC time integration is added.
#[derive(Clone, Copy, Debug, Default)]
pub struct FixedTimeSource;

impl TimeSource for FixedTimeSource {
    fn get_timestamp(&self) -> Timestamp {
        // 2026-01-01 00:00:00
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

/// Result of a phase-1 SD probe and test file read.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct SdProbeResult {
    pub card_size_bytes: u64,
    pub bytes_read: usize,
}

/// One EPUB-like entry discovered during `/books` scan.
#[derive(Debug)]
pub struct SdEpubEntry<const NAME_BYTES: usize> {
    pub short_name: String<NAME_BYTES>,
    pub display_title: String<NAME_BYTES>,
    pub has_cover: bool,
    pub size_bytes: u32,
}

/// EPUB scan result for phase-0 SD file discovery.
#[derive(Debug)]
pub struct SdEpubScanResult<const MAX_EPUBS: usize, const NAME_BYTES: usize> {
    pub card_size_bytes: u64,
    pub books_dir_found: bool,
    pub scanned_file_count: u16,
    pub epub_count_total: u16,
    pub epub_entries: Vec<SdEpubEntry<NAME_BYTES>, MAX_EPUBS>,
    pub truncated: bool,
}

/// Phase-1 SD probing error details.
#[derive(Debug)]
pub enum SdProbeError<BusErr, CsErr>
where
    BusErr: core::fmt::Debug,
    CsErr: core::fmt::Debug,
{
    ChipSelect(CsErr),
    Spi(BusErr),
    Card(SdCardError),
    Filesystem(embedded_sdmmc::Error<SdCardError>),
}

/// Result status for a one-shot EPUB text-chunk probe.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SdEpubTextChunkStatus {
    ReadOk,
    NotZip,
    NoTextResource,
    UnsupportedCompression,
    DecodeFailed,
}

/// Result of reading a first text chunk from one EPUB.
#[derive(Debug)]
pub struct SdEpubTextChunkResult<const PATH_BYTES: usize> {
    pub card_size_bytes: u64,
    pub text_resource: String<PATH_BYTES>,
    pub start_offset: u32,
    pub chapter_index: u16,
    pub chapter_total: u16,
    pub chapter_label: String<SD_CHAPTER_LABEL_BYTES>,
    pub compression: u16,
    pub bytes_read: usize,
    pub end_of_resource: bool,
    pub status: SdEpubTextChunkStatus,
}

/// Result status for a one-shot EPUB cover thumbnail probe.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SdEpubCoverStatus {
    ReadOk,
    NotZip,
    NoCoverResource,
    UnsupportedMediaType,
    DecodeFailed,
}

/// Result of reading and decoding an EPUB cover into a 1bpp thumbnail buffer.
#[derive(Debug)]
pub struct SdEpubCoverResult<const PATH_BYTES: usize, const MEDIA_BYTES: usize> {
    pub card_size_bytes: u64,
    pub cover_resource: String<PATH_BYTES>,
    pub media_type: String<MEDIA_BYTES>,
    pub source_width: u16,
    pub source_height: u16,
    pub thumb_width: u16,
    pub thumb_height: u16,
    pub bytes_written: usize,
    pub status: SdEpubCoverStatus,
}

#[derive(Debug)]
enum ExclusiveSpiError<BusErr, CsErr>
where
    BusErr: core::fmt::Debug,
    CsErr: core::fmt::Debug,
{
    Bus(BusErr),
    Cs(CsErr),
    DelayNotSupported,
}

impl<BusErr, CsErr> SpiErrorTrait for ExclusiveSpiError<BusErr, CsErr>
where
    BusErr: core::fmt::Debug,
    CsErr: core::fmt::Debug,
{
    fn kind(&self) -> ErrorKind {
        ErrorKind::Other
    }
}

struct ExclusiveSpiDevice<'a, BUS, CS>
where
    BUS: SpiBus<u8>,
    CS: OutputPin,
{
    bus: &'a mut BUS,
    cs: &'a mut CS,
}

impl<'a, BUS, CS> ExclusiveSpiDevice<'a, BUS, CS>
where
    BUS: SpiBus<u8>,
    CS: OutputPin,
{
    fn new(bus: &'a mut BUS, cs: &'a mut CS) -> Self {
        Self { bus, cs }
    }
}

impl<BUS, CS> ErrorType for ExclusiveSpiDevice<'_, BUS, CS>
where
    BUS: SpiBus<u8>,
    CS: OutputPin,
    BUS::Error: core::fmt::Debug,
    CS::Error: core::fmt::Debug,
{
    type Error = ExclusiveSpiError<BUS::Error, CS::Error>;
}

impl<BUS, CS> SpiDevice<u8> for ExclusiveSpiDevice<'_, BUS, CS>
where
    BUS: SpiBus<u8>,
    CS: OutputPin,
    BUS::Error: core::fmt::Debug,
    CS::Error: core::fmt::Debug,
{
    fn transaction(&mut self, operations: &mut [Operation<'_, u8>]) -> Result<(), Self::Error> {
        if operations
            .iter()
            .any(|op| matches!(op, Operation::DelayNs(_)))
        {
            return Err(ExclusiveSpiError::DelayNotSupported);
        }

        self.cs.set_low().map_err(ExclusiveSpiError::Cs)?;

        let op_result = (|| {
            for operation in operations {
                match operation {
                    Operation::Read(buf) => self.bus.read(buf).map_err(ExclusiveSpiError::Bus)?,
                    Operation::Write(buf) => self.bus.write(buf).map_err(ExclusiveSpiError::Bus)?,
                    Operation::Transfer(read, write) => self
                        .bus
                        .transfer(read, write)
                        .map_err(ExclusiveSpiError::Bus)?,
                    Operation::TransferInPlace(buf) => self
                        .bus
                        .transfer_in_place(buf)
                        .map_err(ExclusiveSpiError::Bus)?,
                    Operation::DelayNs(_) => return Err(ExclusiveSpiError::DelayNotSupported),
                }
            }
            self.bus.flush().map_err(ExclusiveSpiError::Bus)?;
            Ok(())
        })();

        let cs_result = self.cs.set_high().map_err(ExclusiveSpiError::Cs);
        match (op_result, cs_result) {
            (Err(e), _) => Err(e),
            (Ok(_), Err(e)) => Err(e),
            (Ok(_), Ok(_)) => Ok(()),
        }
    }
}

struct DelayRef<'a, D>(&'a mut D);

impl<D> DelayNs for DelayRef<'_, D>
where
    D: DelayNs,
{
    fn delay_ns(&mut self, ns: u32) {
        self.0.delay_ns(ns);
    }
}

fn has_zip_signature(header: [u8; 4], read_bytes: usize) -> bool {
    if read_bytes < 4 {
        return false;
    }

    matches!(
        header,
        [0x50, 0x4B, 0x03, 0x04] | [0x50, 0x4B, 0x05, 0x06] | [0x50, 0x4B, 0x07, 0x08]
    )
}

fn short_file_name_to_string<const NAME_BYTES: usize>(name: &ShortFileName) -> String<NAME_BYTES> {
    let mut out = String::new();

    for &byte in name.base_name() {
        if out.push(byte as char).is_err() {
            return out;
        }
    }

    let ext = name.extension();
    if !ext.is_empty() {
        let _ = out.push('.');
        for &byte in ext {
            if out.push(byte as char).is_err() {
                break;
            }
        }
    }

    out
}

fn display_title_from_file_name<const NAME_BYTES: usize>(name: &str) -> String<NAME_BYTES> {
    let mut out = String::<NAME_BYTES>::new();

    let stem = name
        .rsplit_once('.')
        .map(|(left, _)| left)
        .unwrap_or(name)
        .trim();

    let mut word_start = true;
    let mut wrote_any = false;
    for byte in stem.as_bytes() {
        let mut ch = *byte;
        if ch == b'_' || ch == b'-' {
            ch = b' ';
        }

        if ch == b' ' {
            if !wrote_any || word_start {
                continue;
            }
            if out.push(' ').is_err() {
                return out;
            }
            word_start = true;
            continue;
        }

        let normalized = if ch.is_ascii_alphabetic() {
            if word_start {
                (ch as char).to_ascii_uppercase()
            } else {
                (ch as char).to_ascii_lowercase()
            }
        } else {
            ch as char
        };
        if out.push(normalized).is_err() {
            return out;
        }

        wrote_any = true;
        word_start = false;
    }

    if out.is_empty() {
        for ch in name.chars() {
            if out.push(ch).is_err() {
                break;
            }
        }
    }

    out
}

const ZIP_EOCD_SIG: [u8; 4] = [0x50, 0x4B, 0x05, 0x06];
const ZIP_CDIR_SIG: [u8; 4] = [0x50, 0x4B, 0x01, 0x02];
const ZIP_LOCAL_SIG: [u8; 4] = [0x50, 0x4B, 0x03, 0x04];
const ZIP_EOCD_MIN_BYTES: usize = 22;
const ZIP_CDIR_HEADER_BYTES: usize = 46;
const ZIP_LOCAL_HEADER_BYTES: usize = 30;
const ZIP_EOCD_SEARCH_WINDOW: usize = 2048;
const ZIP_NAME_BYTES: usize = 384;
const ZIP_CONTAINER_BYTES: usize = 1024;
const ZIP_OPF_BYTES: usize = 8192;
const ZIP_PATH_BYTES: usize = 192;
const ZIP_MEDIA_BYTES: usize = 32;
pub const SD_CHAPTER_LABEL_BYTES: usize = 48;
const ZIP_COVER_BYTES: usize = 12288;
const ZIP_MAX_CDIR_ENTRIES: usize = 512;
const ZIP_DEFLATE_INPUT_BYTES: usize = 320;
const ZIP_DEFLATE_OUTPUT_BYTES: usize = 320;
const ZIP_PATH_SEGMENTS_MAX: usize = 24;
const ZIP_MIN_PRIMARY_TEXT_BYTES: u32 = 900;
const PNG_STREAM_BYTES: usize = 1024;
const PNG_IDAT_IN_BYTES: usize = 384;
const PNG_INFLATE_OUT_BYTES: usize = 768;
const PNG_ROW_BYTES_MAX: usize = 8192;
const JPEG_STREAM_BYTES: usize = 1024;
const JPEG_WORK_BYTES: usize = 8192;
const JPEG_DECODER_BYTES: usize = 1536;
const JPEG_DIM_SCAN_MAX_BYTES: usize = 262_144;

#[derive(Clone, Copy, Debug, Default)]
struct EpubMetadata {
    has_cover: bool,
}

#[derive(Clone, Copy, Debug)]
struct ZipEntryRef {
    compression: u16,
    compressed_size: u32,
    uncompressed_size: u32,
    local_header_offset: u32,
}

type ChapterEntry<const PATH_BYTES: usize> = (ZipEntryRef, String<PATH_BYTES>, u16, u16);
type PngThumbDecodeResult<DErr> = Result<Option<(u16, u16, usize)>, embedded_sdmmc::Error<DErr>>;
type JpegThumbDecodeResult<DErr> = Result<Option<(u16, u16, usize)>, embedded_sdmmc::Error<DErr>>;
type JpegDimScanResult<DErr> = Result<Option<(u16, u16, usize)>, embedded_sdmmc::Error<DErr>>;
type CoverFallbackEntry<const PATH_BYTES: usize, const MEDIA_BYTES: usize> =
    (ZipEntryRef, String<PATH_BYTES>, String<MEDIA_BYTES>);
type CoverFallbackEntryResult<DErr, const PATH_BYTES: usize, const MEDIA_BYTES: usize> =
    Result<Option<CoverFallbackEntry<PATH_BYTES, MEDIA_BYTES>>, embedded_sdmmc::Error<DErr>>;

include!("sd_spi/text_xml.rs");
include!("sd_spi/media_manifest.rs");
include!("sd_spi/spine_index.rs");
include!("sd_spi/toc_index.rs");
include!("sd_spi/cdir.rs");
include!("sd_spi/png_pbm.rs");
include!("sd_spi/jpeg.rs");
include!("sd_spi/io_entry.rs");
include!("sd_spi/chapter_seek.rs");
include!("sd_spi/text_probe_core.rs");
include!("sd_spi/cover_probe.rs");
include!("sd_spi/stream_scan.rs");
