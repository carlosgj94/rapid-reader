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
    pub chapter_index: u16,
    pub chapter_total: u16,
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

fn read_u16_le(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn eq_ascii_case_insensitive(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b.iter())
            .all(|(left, right)| left.eq_ignore_ascii_case(right))
}

fn ends_with_ascii_case_insensitive(bytes: &[u8], suffix: &[u8]) -> bool {
    if suffix.len() > bytes.len() {
        return false;
    }
    eq_ascii_case_insensitive(&bytes[bytes.len() - suffix.len()..], suffix)
}

fn find_ascii_case_insensitive(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from >= haystack.len() || needle.len() > haystack.len() {
        return None;
    }

    let max_start = haystack.len().saturating_sub(needle.len());
    if from > max_start {
        return None;
    }

    (from..=max_start)
        .find(|&idx| eq_ascii_case_insensitive(&haystack[idx..idx + needle.len()], needle))
}

fn contains_ascii_case_insensitive(haystack: &[u8], needle: &[u8]) -> bool {
    find_ascii_case_insensitive(haystack, needle, 0).is_some()
}

fn is_text_resource_name(name: &[u8]) -> bool {
    if name.is_empty() || name[name.len() - 1] == b'/' {
        return false;
    }

    if contains_ascii_case_insensitive(name, b"META-INF/")
        || contains_ascii_case_insensitive(name, b"/META-INF/")
    {
        return false;
    }

    ends_with_ascii_case_insensitive(name, b".xhtml")
        || ends_with_ascii_case_insensitive(name, b".html")
        || ends_with_ascii_case_insensitive(name, b".htm")
        || ends_with_ascii_case_insensitive(name, b".txt")
}

fn copy_ascii_or_lossy<const N: usize>(source: &[u8], out: &mut String<N>) {
    out.clear();
    for &byte in source {
        let ch = if byte.is_ascii() { byte as char } else { '?' };
        if out.push(ch).is_err() {
            break;
        }
    }
}

fn trim_ascii(slice: &[u8]) -> &[u8] {
    let mut start = 0usize;
    let mut end = slice.len();
    while start < end && slice[start].is_ascii_whitespace() {
        start += 1;
    }
    while end > start && slice[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    &slice[start..end]
}

fn copy_utf8_or_ascii<const N: usize>(source: &[u8], out: &mut String<N>) -> bool {
    out.clear();
    let source = trim_ascii(source);
    if source.is_empty() {
        return false;
    }

    if let Ok(text) = str::from_utf8(source) {
        for ch in text.chars() {
            if out.push(ch).is_err() {
                break;
            }
        }
    } else {
        for &byte in source {
            let ch = if byte.is_ascii() { byte as char } else { '?' };
            if out.push(ch).is_err() {
                break;
            }
        }
    }

    !out.is_empty()
}

fn parse_xml_tag_text<const N: usize>(xml: &[u8], tag: &[u8], out: &mut String<N>) -> bool {
    let mut search_from = 0usize;
    while let Some(tag_pos) = find_ascii_case_insensitive(xml, tag, search_from) {
        let after_tag = tag_pos.saturating_add(tag.len());
        let Some(open_end_rel) = xml[after_tag..].iter().position(|b| *b == b'>') else {
            break;
        };
        let text_start = after_tag + open_end_rel + 1;
        let Some(text_end_rel) = xml[text_start..].iter().position(|b| *b == b'<') else {
            break;
        };
        let text_end = text_start + text_end_rel;
        if copy_utf8_or_ascii(&xml[text_start..text_end], out) {
            return true;
        }
        search_from = text_end.saturating_add(1);
    }
    false
}

fn parse_container_full_path<const N: usize>(xml: &[u8], out: &mut String<N>) -> bool {
    let mut search_from = 0usize;
    while let Some(attr_pos) = find_ascii_case_insensitive(xml, b"full-path", search_from) {
        let mut idx = attr_pos + b"full-path".len();
        while idx < xml.len() && xml[idx].is_ascii_whitespace() {
            idx += 1;
        }
        if idx >= xml.len() || xml[idx] != b'=' {
            search_from = idx.saturating_add(1);
            continue;
        }
        idx += 1;
        while idx < xml.len() && xml[idx].is_ascii_whitespace() {
            idx += 1;
        }
        if idx >= xml.len() {
            break;
        }
        let quote = xml[idx];
        if quote != b'\'' && quote != b'"' {
            search_from = idx.saturating_add(1);
            continue;
        }
        idx += 1;
        let start = idx;
        while idx < xml.len() && xml[idx] != quote {
            idx += 1;
        }
        if idx <= start {
            search_from = idx.saturating_add(1);
            continue;
        }

        out.clear();
        for &byte in &xml[start..idx] {
            if out.push(byte as char).is_err() {
                break;
            }
        }
        if !out.is_empty() {
            return true;
        }
        search_from = idx.saturating_add(1);
    }
    false
}

fn find_xml_element_bounds_in_range(
    xml: &[u8],
    local_name: &[u8],
    from: usize,
    end: usize,
) -> Option<(usize, usize)> {
    if from >= end || end > xml.len() {
        return None;
    }

    let mut cursor = from;
    while cursor < end {
        let lt_rel = xml[cursor..end].iter().position(|b| *b == b'<')?;
        let start = cursor + lt_rel;
        let mut name_start = start.saturating_add(1);
        if name_start >= end {
            return None;
        }

        // Ignore closing/doctype/pi tags.
        if matches!(xml[name_start], b'/' | b'!' | b'?') {
            cursor = name_start.saturating_add(1);
            continue;
        }

        while name_start < end && xml[name_start].is_ascii_whitespace() {
            name_start += 1;
        }
        if name_start >= end {
            return None;
        }

        let mut name_end = name_start;
        while name_end < end
            && !xml[name_end].is_ascii_whitespace()
            && xml[name_end] != b'/'
            && xml[name_end] != b'>'
        {
            name_end += 1;
        }
        if name_end <= name_start {
            cursor = start.saturating_add(1);
            continue;
        }

        let full_name = &xml[name_start..name_end];
        let local = full_name
            .iter()
            .rposition(|b| *b == b':')
            .map(|idx| &full_name[idx + 1..])
            .unwrap_or(full_name);

        if eq_ascii_case_insensitive(local, local_name) {
            let end_rel = xml[name_end..end].iter().position(|b| *b == b'>')?;
            return Some((start, name_end + end_rel + 1));
        }

        cursor = name_end.saturating_add(1);
    }

    None
}

fn find_xml_attr_value<'a>(tag: &'a [u8], attr: &[u8]) -> Option<&'a [u8]> {
    let mut search_from = 0usize;
    while let Some(attr_pos) = find_ascii_case_insensitive(tag, attr, search_from) {
        let prev_ok = attr_pos == 0
            || tag[attr_pos - 1].is_ascii_whitespace()
            || tag[attr_pos - 1] == b'<'
            || tag[attr_pos - 1] == b'/';
        if !prev_ok {
            search_from = attr_pos.saturating_add(1);
            continue;
        }

        let mut idx = attr_pos + attr.len();
        while idx < tag.len() && tag[idx].is_ascii_whitespace() {
            idx += 1;
        }
        if idx >= tag.len() || tag[idx] != b'=' {
            search_from = attr_pos.saturating_add(1);
            continue;
        }
        idx += 1;
        while idx < tag.len() && tag[idx].is_ascii_whitespace() {
            idx += 1;
        }
        if idx >= tag.len() {
            return None;
        }

        let quote = tag[idx];
        if quote == b'"' || quote == b'\'' {
            idx += 1;
            let start = idx;
            while idx < tag.len() && tag[idx] != quote {
                idx += 1;
            }
            if idx > start {
                return Some(&tag[start..idx]);
            }
            return None;
        }

        let start = idx;
        while idx < tag.len() && !tag[idx].is_ascii_whitespace() && tag[idx] != b'>' {
            idx += 1;
        }
        if idx > start {
            return Some(&tag[start..idx]);
        }
        return None;
    }
    None
}

fn parse_opf_meta_content<const N: usize>(
    opf: &[u8],
    attr_name: &[u8],
    out: &mut String<N>,
) -> bool {
    let mut cursor = 0usize;
    while let Some((start, end)) = find_xml_element_bounds_in_range(opf, b"meta", cursor, opf.len())
    {
        let tag = &opf[start..end];
        let name = find_xml_attr_value(tag, attr_name);
        let content = find_xml_attr_value(tag, b"content");
        if let (Some(name), Some(content)) = (name, content)
            && contains_ascii_case_insensitive(name, b"title")
            && copy_utf8_or_ascii(content, out)
        {
            return true;
        }
        cursor = end;
    }
    false
}

fn is_text_media_type(media: &[u8]) -> bool {
    contains_ascii_case_insensitive(media, b"xhtml")
        || contains_ascii_case_insensitive(media, b"html")
        || contains_ascii_case_insensitive(media, b"text/plain")
}

fn path_is_probably_front_matter(path: &[u8]) -> bool {
    contains_ascii_case_insensitive(path, b"cover")
        || contains_ascii_case_insensitive(path, b"portada")
        || contains_ascii_case_insensitive(path, b"cubierta")
        || contains_ascii_case_insensitive(path, b"info")
        || contains_ascii_case_insensitive(path, b"about")
        || contains_ascii_case_insensitive(path, b"acerca")
        || contains_ascii_case_insensitive(path, b"title")
        || contains_ascii_case_insensitive(path, b"frontmatter")
        || contains_ascii_case_insensitive(path, b"toc")
        || contains_ascii_case_insensitive(path, b"indice")
        || contains_ascii_case_insensitive(path, b"index")
        || contains_ascii_case_insensitive(path, b"nav")
        || contains_ascii_case_insensitive(path, b"contents")
        || contains_ascii_case_insensitive(path, b"credit")
        || contains_ascii_case_insensitive(path, b"license")
        || contains_ascii_case_insensitive(path, b"licencia")
        || contains_ascii_case_insensitive(path, b"imprint")
        || contains_ascii_case_insensitive(path, b"preface")
        || contains_ascii_case_insensitive(path, b"foreword")
        || contains_ascii_case_insensitive(path, b"prologue")
        || contains_ascii_case_insensitive(path, b"prologo")
        || contains_ascii_case_insensitive(path, b"dedicat")
        || contains_ascii_case_insensitive(path, b"introduc")
        || contains_ascii_case_insensitive(path, b"nota")
        || contains_ascii_case_insensitive(path, b"note")
        || contains_ascii_case_insensitive(path, b"warning")
        || contains_ascii_case_insensitive(path, b"advert")
        || contains_ascii_case_insensitive(path, b"copyright")
        || contains_ascii_case_insensitive(path, b"colophon")
        || contains_ascii_case_insensitive(path, b"legal")
        || contains_ascii_case_insensitive(path, b"acknowledg")
}

fn resolve_opf_href<const PATH_BYTES: usize>(
    opf_path: &str,
    href: &[u8],
    out: &mut String<PATH_BYTES>,
) -> bool {
    let href = trim_ascii(href);
    if href.is_empty() {
        return false;
    }

    let mut href_clean = href;
    if let Some(hash_pos) = href_clean.iter().position(|b| *b == b'#') {
        href_clean = &href_clean[..hash_pos];
    }
    if let Some(query_pos) = href_clean.iter().position(|b| *b == b'?') {
        href_clean = &href_clean[..query_pos];
    }
    let href_clean = trim_ascii(href_clean);
    if href_clean.is_empty() {
        return false;
    }

    let base_dir = opf_path
        .rsplit_once('/')
        .map(|(left, _)| left)
        .unwrap_or("");

    let mut provisional = String::<PATH_BYTES>::new();
    if !href_clean.starts_with(b"/") && !base_dir.is_empty() {
        for ch in base_dir.chars() {
            if provisional.push(ch).is_err() {
                return false;
            }
        }
        let _ = provisional.push('/');
    }
    for &byte in href_clean {
        let ch = if byte.is_ascii() { byte as char } else { '?' };
        if provisional.push(ch).is_err() {
            return false;
        }
    }

    let mut segments: Vec<&str, ZIP_PATH_SEGMENTS_MAX> = Vec::new();
    for seg in provisional.as_str().split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            let _ = segments.pop();
            continue;
        }
        if segments.push(seg).is_err() {
            return false;
        }
    }

    out.clear();
    for (idx, seg) in segments.iter().enumerate() {
        if idx > 0 {
            let _ = out.push('/');
        }
        for ch in seg.chars() {
            if out.push(ch).is_err() {
                return false;
            }
        }
    }

    !out.is_empty()
}

type ManifestItemMatch<'a> = (&'a [u8], Option<&'a [u8]>, Option<&'a [u8]>);

fn find_manifest_item_by_id_in_range<'a>(
    opf: &'a [u8],
    idref: &[u8],
    start_cursor: usize,
    end_cursor: usize,
) -> Option<(usize, ManifestItemMatch<'a>)> {
    let mut cursor = start_cursor.min(opf.len());
    let limit = end_cursor.min(opf.len());
    while let Some((start, end)) = find_xml_element_bounds_in_range(opf, b"item", cursor, limit) {
        let tag = &opf[start..end];
        let Some(item_id) = find_xml_attr_value(tag, b"id") else {
            cursor = end;
            continue;
        };
        if !eq_ascii_case_insensitive(item_id, idref) {
            cursor = end;
            continue;
        }

        let href = find_xml_attr_value(tag, b"href")?;
        let media = find_xml_attr_value(tag, b"media-type");
        let properties = find_xml_attr_value(tag, b"properties");
        return Some((end, (href, media, properties)));
    }

    None
}

fn find_manifest_item_by_id_with_hint<'a>(
    opf: &'a [u8],
    idref: &[u8],
    cursor_hint: &mut usize,
) -> Option<ManifestItemMatch<'a>> {
    let hint = (*cursor_hint).min(opf.len());
    if let Some((next, matched)) = find_manifest_item_by_id_in_range(opf, idref, hint, opf.len()) {
        *cursor_hint = next;
        return Some(matched);
    }

    if hint > 0
        && let Some((next, matched)) = find_manifest_item_by_id_in_range(opf, idref, 0, hint)
    {
        *cursor_hint = next;
        return Some(matched);
    }

    None
}

fn find_manifest_item_by_id<'a>(opf: &'a [u8], idref: &[u8]) -> Option<ManifestItemMatch<'a>> {
    let mut cursor_hint = 0usize;
    find_manifest_item_by_id_with_hint(opf, idref, &mut cursor_hint)
}

fn is_image_media_type(media: &[u8]) -> bool {
    contains_ascii_case_insensitive(media, b"image/")
}

fn strip_resource_suffix(path: &[u8]) -> &[u8] {
    let mut end = path.len();
    for (idx, &byte) in path.iter().enumerate() {
        if byte == b'?' || byte == b'#' {
            end = idx;
            break;
        }
    }
    &path[..end]
}

fn is_cover_media_pbm(media: &[u8], path: &[u8]) -> bool {
    let path = strip_resource_suffix(path);
    contains_ascii_case_insensitive(media, b"image/x-portable-bitmap")
        || contains_ascii_case_insensitive(media, b"image/pbm")
        || ends_with_ascii_case_insensitive(path, b".pbm")
}

fn is_cover_media_png(media: &[u8], path: &[u8]) -> bool {
    let path = strip_resource_suffix(path);
    contains_ascii_case_insensitive(media, b"image/png")
        || ends_with_ascii_case_insensitive(path, b".png")
}

fn is_cover_media_jpeg(media: &[u8], path: &[u8]) -> bool {
    let path = strip_resource_suffix(path);
    contains_ascii_case_insensitive(media, b"image/jpeg")
        || contains_ascii_case_insensitive(media, b"image/jpg")
        || ends_with_ascii_case_insensitive(path, b".jpg")
        || ends_with_ascii_case_insensitive(path, b".jpeg")
        || ends_with_ascii_case_insensitive(path, b".jpe")
        || ends_with_ascii_case_insensitive(path, b".jfif")
}

fn is_cover_like_image_path(path: &[u8]) -> bool {
    contains_ascii_case_insensitive(path, b"cover")
        || contains_ascii_case_insensitive(path, b"portada")
        || contains_ascii_case_insensitive(path, b"front")
        || contains_ascii_case_insensitive(path, b"titlepage")
        || contains_ascii_case_insensitive(path, b"jacket")
}

fn is_image_resource_name(path: &[u8]) -> bool {
    let path = strip_resource_suffix(path);
    ends_with_ascii_case_insensitive(path, b".pbm")
        || ends_with_ascii_case_insensitive(path, b".png")
        || ends_with_ascii_case_insensitive(path, b".jpg")
        || ends_with_ascii_case_insensitive(path, b".jpeg")
        || ends_with_ascii_case_insensitive(path, b".jpe")
        || ends_with_ascii_case_insensitive(path, b".jfif")
        || ends_with_ascii_case_insensitive(path, b".gif")
        || ends_with_ascii_case_insensitive(path, b".webp")
        || ends_with_ascii_case_insensitive(path, b".svg")
        || ends_with_ascii_case_insensitive(path, b".bmp")
        || ends_with_ascii_case_insensitive(path, b".tif")
        || ends_with_ascii_case_insensitive(path, b".tiff")
}

fn is_probable_image_resource_path(path: &[u8]) -> bool {
    let path = strip_resource_suffix(path);
    contains_ascii_case_insensitive(path, b"/images/")
        || contains_ascii_case_insensitive(path, b"/image/")
        || contains_ascii_case_insensitive(path, b"/img/")
        || contains_ascii_case_insensitive(path, b"illustration")
        || contains_ascii_case_insensitive(path, b"artwork")
        || is_cover_like_image_path(path)
}

fn sniff_cover_media_flags(bytes: &[u8]) -> (bool, bool, bool) {
    let png = bytes.starts_with(&[137, 80, 78, 71, 13, 10, 26, 10]);
    let jpeg = bytes.starts_with(&[0xFF, 0xD8]);

    let mut idx = 0usize;
    while idx < bytes.len() && bytes[idx].is_ascii_whitespace() {
        idx = idx.saturating_add(1);
    }
    let pbm = idx + 1 < bytes.len() && bytes[idx] == b'P' && bytes[idx + 1] == b'4';

    (pbm, png, jpeg)
}

fn sniff_cover_media_from_entry<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    entry: ZipEntryRef,
) -> Result<Option<&'static str>, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let mut sniff = [0u8; 16];
    let sniff_read = read_zip_entry_prefix(file, entry, &mut sniff)?;
    if sniff_read == 0 {
        return Ok(None);
    }

    let (pbm, png, jpeg) = sniff_cover_media_flags(&sniff[..sniff_read]);
    if pbm {
        return Ok(Some("image/x-portable-bitmap"));
    }
    if png {
        return Ok(Some("image/png"));
    }
    if jpeg {
        return Ok(Some("image/jpeg"));
    }

    Ok(None)
}

fn is_text_media_document(media: &[u8], path: &[u8]) -> bool {
    is_text_media_type(media)
        || ends_with_ascii_case_insensitive(path, b".xhtml")
        || ends_with_ascii_case_insensitive(path, b".html")
}

fn copy_media_type_or_default<const MEDIA_BYTES: usize>(
    media: Option<&[u8]>,
    path: &[u8],
    out: &mut String<MEDIA_BYTES>,
) {
    let path = strip_resource_suffix(path);
    out.clear();
    if media.is_some_and(|value| copy_utf8_or_ascii(value, out)) {
        return;
    }

    let inferred = if ends_with_ascii_case_insensitive(path, b".pbm") {
        "image/x-portable-bitmap"
    } else if ends_with_ascii_case_insensitive(path, b".png") {
        "image/png"
    } else if ends_with_ascii_case_insensitive(path, b".jpg")
        || ends_with_ascii_case_insensitive(path, b".jpeg")
        || ends_with_ascii_case_insensitive(path, b".jpe")
        || ends_with_ascii_case_insensitive(path, b".jfif")
    {
        "image/jpeg"
    } else if ends_with_ascii_case_insensitive(path, b".gif") {
        "image/gif"
    } else if ends_with_ascii_case_insensitive(path, b".webp") {
        "image/webp"
    } else if ends_with_ascii_case_insensitive(path, b".svg") {
        "image/svg+xml"
    } else if ends_with_ascii_case_insensitive(path, b".bmp") {
        "image/bmp"
    } else if ends_with_ascii_case_insensitive(path, b".tif")
        || ends_with_ascii_case_insensitive(path, b".tiff")
    {
        "image/tiff"
    } else if ends_with_ascii_case_insensitive(path, b".xhtml")
        || ends_with_ascii_case_insensitive(path, b".html")
    {
        "application/xhtml+xml"
    } else {
        ""
    };

    if !inferred.is_empty() {
        let _ = out.push_str(inferred);
    }
}

fn parse_meta_cover_id<const ID_BYTES: usize>(opf: &[u8], out: &mut String<ID_BYTES>) -> bool {
    let mut cursor = 0usize;
    while let Some((start, end)) = find_xml_element_bounds_in_range(opf, b"meta", cursor, opf.len())
    {
        let tag = &opf[start..end];
        let name = find_xml_attr_value(tag, b"name");
        let content = find_xml_attr_value(tag, b"content");
        if let (Some(name), Some(content)) = (name, content)
            && eq_ascii_case_insensitive(trim_ascii(name), b"cover")
            && copy_utf8_or_ascii(content, out)
        {
            return true;
        }

        cursor = end;
    }

    false
}

fn parse_manifest_image_href_with_filter<const PATH_BYTES: usize, const MEDIA_BYTES: usize, F>(
    opf: &[u8],
    opf_path: &str,
    mut matches: F,
    out_path: &mut String<PATH_BYTES>,
    out_media: &mut String<MEDIA_BYTES>,
) -> bool
where
    F: FnMut(&[u8], &[u8], Option<&[u8]>, Option<&[u8]>) -> bool,
{
    let mut cursor = 0usize;
    while let Some((start, end)) = find_xml_element_bounds_in_range(opf, b"item", cursor, opf.len())
    {
        let tag = &opf[start..end];
        let id = find_xml_attr_value(tag, b"id").unwrap_or(b"");
        let Some(href) = find_xml_attr_value(tag, b"href") else {
            cursor = end;
            continue;
        };
        let media = find_xml_attr_value(tag, b"media-type");
        let properties = find_xml_attr_value(tag, b"properties");
        let href_looks_image =
            is_image_resource_name(href) || is_probable_image_resource_path(href);
        if !media.is_some_and(is_image_media_type) && !href_looks_image {
            cursor = end;
            continue;
        }
        if !matches(id, href, media, properties) {
            cursor = end;
            continue;
        }

        if !resolve_opf_href(opf_path, href, out_path) {
            cursor = end;
            continue;
        }
        copy_media_type_or_default(media, out_path.as_bytes(), out_media);
        return true;
    }

    false
}

fn parse_opf_cover_resource<const PATH_BYTES: usize, const MEDIA_BYTES: usize>(
    opf: &[u8],
    opf_path: &str,
    out_path: &mut String<PATH_BYTES>,
    out_media: &mut String<MEDIA_BYTES>,
) -> bool {
    out_path.clear();
    out_media.clear();

    let mut cover_id = String::<ZIP_PATH_BYTES>::new();
    if parse_meta_cover_id(opf, &mut cover_id)
        && let Some((href, media, _properties)) = find_manifest_item_by_id(opf, cover_id.as_bytes())
        && resolve_opf_href(opf_path, href, out_path)
    {
        copy_media_type_or_default(media, out_path.as_bytes(), out_media);
        return true;
    }

    if parse_manifest_image_href_with_filter(
        opf,
        opf_path,
        |_id, _href, _media, properties| {
            properties.is_some_and(|value| contains_ascii_case_insensitive(value, b"cover-image"))
        },
        out_path,
        out_media,
    ) {
        return true;
    }

    if parse_manifest_image_href_with_filter(
        opf,
        opf_path,
        |id, _href, _media, _properties| contains_ascii_case_insensitive(id, b"cover"),
        out_path,
        out_media,
    ) {
        return true;
    }

    if parse_manifest_image_href_with_filter(
        opf,
        opf_path,
        |_id, href, _media, _properties| contains_ascii_case_insensitive(href, b"cover"),
        out_path,
        out_media,
    ) {
        return true;
    }

    parse_manifest_image_href_with_filter(
        opf,
        opf_path,
        |_id, _href, _media, _properties| true,
        out_path,
        out_media,
    )
}

fn parse_html_first_img_src<const PATH_BYTES: usize>(
    html: &[u8],
    html_path: &str,
    out: &mut String<PATH_BYTES>,
) -> bool {
    let mut cursor = 0usize;
    while let Some(start) = find_ascii_case_insensitive(html, b"<img", cursor) {
        let rel_end = html[start..]
            .iter()
            .position(|b| *b == b'>')
            .map(|idx| start.saturating_add(idx).saturating_add(1));
        let Some(end) = rel_end else {
            break;
        };
        let tag = &html[start..end];
        if let Some(src) = find_xml_attr_value(tag, b"src")
            && resolve_opf_href(html_path, src, out)
        {
            return true;
        }
        cursor = end;
    }

    false
}

fn parse_spine_first_text_href<const PATH_BYTES: usize>(
    opf: &[u8],
    opf_path: &str,
    out: &mut String<PATH_BYTES>,
) -> bool {
    let mut cursor = 0usize;
    let mut manifest_cursor_hint = 0usize;
    while let Some((start, end)) =
        find_xml_element_bounds_in_range(opf, b"itemref", cursor, opf.len())
    {
        let tag = &opf[start..end];
        let Some(idref) = find_xml_attr_value(tag, b"idref") else {
            cursor = end;
            continue;
        };
        let linear = find_xml_attr_value(tag, b"linear");
        if linear.is_some_and(|value| eq_ascii_case_insensitive(trim_ascii(value), b"no")) {
            cursor = end;
            continue;
        }

        let Some((href, media, properties)) =
            find_manifest_item_by_id_with_hint(opf, idref, &mut manifest_cursor_hint)
        else {
            cursor = end;
            continue;
        };
        if !media.is_some_and(is_text_media_type) {
            cursor = end;
            continue;
        }

        let mut resolved = String::<PATH_BYTES>::new();
        if !resolve_opf_href(opf_path, href, &mut resolved) {
            cursor = end;
            continue;
        }

        let skip_by_properties =
            properties.is_some_and(|prop| contains_ascii_case_insensitive(prop, b"nav"));
        if skip_by_properties
            || path_is_probably_front_matter(idref)
            || path_is_probably_front_matter(href)
            || path_is_probably_front_matter(resolved.as_bytes())
        {
            cursor = end;
            continue;
        }

        *out = resolved;
        return true;
    }

    false
}

fn parse_spine_next_text_href<const PATH_BYTES: usize>(
    opf: &[u8],
    opf_path: &str,
    current_path: &str,
    out: &mut String<PATH_BYTES>,
) -> bool {
    if current_path.is_empty() {
        return false;
    }

    let mut cursor = 0usize;
    let mut manifest_cursor_hint = 0usize;
    let mut seen_current = false;
    while let Some((start, end)) =
        find_xml_element_bounds_in_range(opf, b"itemref", cursor, opf.len())
    {
        let tag = &opf[start..end];
        let Some(idref) = find_xml_attr_value(tag, b"idref") else {
            cursor = end;
            continue;
        };
        let linear = find_xml_attr_value(tag, b"linear");
        if linear.is_some_and(|value| eq_ascii_case_insensitive(trim_ascii(value), b"no")) {
            cursor = end;
            continue;
        }

        let Some((href, media, properties)) =
            find_manifest_item_by_id_with_hint(opf, idref, &mut manifest_cursor_hint)
        else {
            cursor = end;
            continue;
        };
        if !media.is_some_and(is_text_media_type) {
            cursor = end;
            continue;
        }

        let mut resolved = String::<PATH_BYTES>::new();
        if !resolve_opf_href(opf_path, href, &mut resolved) {
            cursor = end;
            continue;
        }

        if properties.is_some_and(|prop| contains_ascii_case_insensitive(prop, b"nav")) {
            cursor = end;
            continue;
        }

        if !seen_current {
            if eq_ascii_case_insensitive(resolved.as_bytes(), current_path.as_bytes()) {
                seen_current = true;
            }
            cursor = end;
            continue;
        }

        *out = resolved;
        return true;
    }

    false
}

fn parse_spine_text_href_at_with_filter<const PATH_BYTES: usize>(
    opf: &[u8],
    opf_path: &str,
    target_index: u16,
    skip_front_matter: bool,
    out: &mut String<PATH_BYTES>,
    out_index: &mut u16,
    out_total: &mut u16,
) -> bool {
    let mut cursor = 0usize;
    let mut manifest_cursor_hint = 0usize;
    let mut total = 0u16;
    let mut selected = String::<PATH_BYTES>::new();
    let mut selected_index = 0u16;
    let mut found = false;
    let mut last = String::<PATH_BYTES>::new();
    let mut last_index = 0u16;
    let mut have_last = false;

    while let Some((start, end)) =
        find_xml_element_bounds_in_range(opf, b"itemref", cursor, opf.len())
    {
        let tag = &opf[start..end];
        let Some(idref) = find_xml_attr_value(tag, b"idref") else {
            cursor = end;
            continue;
        };
        let linear = find_xml_attr_value(tag, b"linear");
        if linear.is_some_and(|value| eq_ascii_case_insensitive(trim_ascii(value), b"no")) {
            cursor = end;
            continue;
        }

        let Some((href, media, properties)) =
            find_manifest_item_by_id_with_hint(opf, idref, &mut manifest_cursor_hint)
        else {
            cursor = end;
            continue;
        };
        if !media.is_some_and(is_text_media_type) {
            cursor = end;
            continue;
        }
        if properties.is_some_and(|prop| contains_ascii_case_insensitive(prop, b"nav")) {
            cursor = end;
            continue;
        }

        let mut resolved = String::<PATH_BYTES>::new();
        if !resolve_opf_href(opf_path, href, &mut resolved) {
            cursor = end;
            continue;
        }

        if skip_front_matter
            && (path_is_probably_front_matter(idref)
                || path_is_probably_front_matter(href)
                || path_is_probably_front_matter(resolved.as_bytes()))
        {
            cursor = end;
            continue;
        }

        if total == target_index && !found {
            selected = resolved.clone();
            selected_index = total;
            found = true;
        }
        last = resolved;
        last_index = total;
        have_last = true;
        total = total.saturating_add(1);
        cursor = end;
    }

    if total == 0 {
        return false;
    }

    if !found {
        if !have_last {
            return false;
        }
        selected = last;
        selected_index = last_index;
    }

    *out = selected;
    *out_index = selected_index;
    *out_total = total.max(1);
    true
}

fn parse_spine_text_href_at<const PATH_BYTES: usize>(
    opf: &[u8],
    opf_path: &str,
    target_index: u16,
    out: &mut String<PATH_BYTES>,
    out_index: &mut u16,
    out_total: &mut u16,
) -> bool {
    parse_spine_text_href_at_with_filter(
        opf,
        opf_path,
        target_index,
        true,
        out,
        out_index,
        out_total,
    ) || parse_spine_text_href_at_with_filter(
        opf,
        opf_path,
        target_index,
        false,
        out,
        out_index,
        out_total,
    )
}

fn parse_spine_position_for_path_with_filter(
    opf: &[u8],
    opf_path: &str,
    target_path: &str,
    skip_front_matter: bool,
) -> Option<(u16, u16)> {
    if target_path.is_empty() {
        return None;
    }

    let mut cursor = 0usize;
    let mut manifest_cursor_hint = 0usize;
    let mut total = 0u16;
    let mut target_index = None;
    while let Some((start, end)) =
        find_xml_element_bounds_in_range(opf, b"itemref", cursor, opf.len())
    {
        let tag = &opf[start..end];
        let Some(idref) = find_xml_attr_value(tag, b"idref") else {
            cursor = end;
            continue;
        };
        let linear = find_xml_attr_value(tag, b"linear");
        if linear.is_some_and(|value| eq_ascii_case_insensitive(trim_ascii(value), b"no")) {
            cursor = end;
            continue;
        }

        let Some((href, media, properties)) =
            find_manifest_item_by_id_with_hint(opf, idref, &mut manifest_cursor_hint)
        else {
            cursor = end;
            continue;
        };
        if !media.is_some_and(is_text_media_type) {
            cursor = end;
            continue;
        }
        if properties.is_some_and(|prop| contains_ascii_case_insensitive(prop, b"nav")) {
            cursor = end;
            continue;
        }

        let mut resolved = String::<ZIP_PATH_BYTES>::new();
        if !resolve_opf_href(opf_path, href, &mut resolved) {
            cursor = end;
            continue;
        }

        if skip_front_matter
            && (path_is_probably_front_matter(idref)
                || path_is_probably_front_matter(href)
                || path_is_probably_front_matter(resolved.as_bytes()))
        {
            cursor = end;
            continue;
        }

        if total < u16::MAX {
            if eq_ascii_case_insensitive(resolved.as_bytes(), target_path.as_bytes()) {
                target_index = Some(total);
            }
            total = total.saturating_add(1);
        }
        cursor = end;
    }

    target_index.map(|index| (index, total.max(1)))
}

fn parse_spine_position_for_path(
    opf: &[u8],
    opf_path: &str,
    target_path: &str,
) -> Option<(u16, u16)> {
    parse_spine_position_for_path_with_filter(opf, opf_path, target_path, true)
        .or_else(|| parse_spine_position_for_path_with_filter(opf, opf_path, target_path, false))
}

fn spine_position_for_resource<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    file_size: u32,
    resource_path: &str,
) -> Result<Option<(u16, u16)>, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let mut opf_buf = [0u8; ZIP_OPF_BYTES];
    let Some((opf_entry, opf_path)) =
        find_opf_entry_and_path::<_, _, _, _, _, ZIP_PATH_BYTES>(file, file_size)?
    else {
        return cdir_position_for_resource(file, file_size, resource_path);
    };
    let opf_read = read_zip_entry_prefix(file, opf_entry, &mut opf_buf)?;
    if opf_read == 0 {
        return cdir_position_for_resource(file, file_size, resource_path);
    }

    if let Some(spine_pos) =
        parse_spine_position_for_path(&opf_buf[..opf_read], opf_path.as_str(), resource_path)
    {
        return Ok(Some(spine_pos));
    }

    cdir_position_for_resource(file, file_size, resource_path)
}

fn cdir_position_for_resource_with_filter<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    file_size: u32,
    resource_path: &str,
    skip_front_matter: bool,
) -> Result<Option<(u16, u16)>, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    if resource_path.is_empty() {
        return Ok(None);
    }

    let Some((cdir_offset, cdir_entries)) = cdir_info(file, file_size)? else {
        return Ok(None);
    };

    let mut cdir_header = [0u8; ZIP_CDIR_HEADER_BYTES];
    let mut cdir_name = [0u8; ZIP_NAME_BYTES];
    let mut cdir_cursor = cdir_offset;
    let mut total = 0u16;
    let mut target = None;

    for _ in 0..cdir_entries.min(ZIP_MAX_CDIR_ENTRIES) {
        let header_read = read_file_at(file, cdir_cursor, &mut cdir_header)?;
        if header_read < ZIP_CDIR_HEADER_BYTES || !cdir_header.starts_with(&ZIP_CDIR_SIG) {
            break;
        }

        let name_len = read_u16_le(&cdir_header, 28) as usize;
        let extra_len = read_u16_le(&cdir_header, 30) as usize;
        let comment_len = read_u16_le(&cdir_header, 32) as usize;
        let Some(next_cursor) = cdir_cursor
            .checked_add(ZIP_CDIR_HEADER_BYTES as u32)
            .and_then(|value| value.checked_add(name_len as u32))
            .and_then(|value| value.checked_add(extra_len as u32))
            .and_then(|value| value.checked_add(comment_len as u32))
        else {
            break;
        };

        let name_read_len = name_len.min(cdir_name.len());
        if name_read_len == 0 {
            cdir_cursor = next_cursor;
            continue;
        }

        let name_read = read_file_at(
            file,
            cdir_cursor.saturating_add(ZIP_CDIR_HEADER_BYTES as u32),
            &mut cdir_name[..name_read_len],
        )?;
        if name_read < name_read_len {
            break;
        }

        let name_complete = name_len <= cdir_name.len();
        let name_slice = &cdir_name[..name_read_len];
        if name_complete && is_text_resource_name(name_slice) {
            if skip_front_matter && path_is_probably_front_matter(name_slice) {
                cdir_cursor = next_cursor;
                continue;
            }

            if total < u16::MAX {
                if eq_ascii_case_insensitive(name_slice, resource_path.as_bytes()) {
                    target = Some(total);
                }
                total = total.saturating_add(1);
            }
        }

        cdir_cursor = next_cursor;
    }

    Ok(target.map(|index| (index, total.max(1))))
}

fn cdir_position_for_resource<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    file_size: u32,
    resource_path: &str,
) -> Result<Option<(u16, u16)>, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let with_filter = cdir_position_for_resource_with_filter(file, file_size, resource_path, true)?;
    if with_filter.is_some() {
        return Ok(with_filter);
    }
    cdir_position_for_resource_with_filter(file, file_size, resource_path, false)
}

fn cdir_text_entry_at_with_filter<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
    const PATH_BYTES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    file_size: u32,
    target_index: u16,
    skip_front_matter: bool,
) -> Result<Option<ChapterEntry<PATH_BYTES>>, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let Some((cdir_offset, cdir_entries)) = cdir_info(file, file_size)? else {
        return Ok(None);
    };

    let mut cdir_header = [0u8; ZIP_CDIR_HEADER_BYTES];
    let mut cdir_name = [0u8; ZIP_NAME_BYTES];
    let mut cdir_cursor = cdir_offset;
    let mut total = 0u16;
    let mut selected: Option<(ZipEntryRef, String<PATH_BYTES>, u16)> = None;
    let mut fallback_last: Option<(ZipEntryRef, String<PATH_BYTES>, u16)> = None;

    for _ in 0..cdir_entries.min(ZIP_MAX_CDIR_ENTRIES) {
        let header_read = read_file_at(file, cdir_cursor, &mut cdir_header)?;
        if header_read < ZIP_CDIR_HEADER_BYTES || !cdir_header.starts_with(&ZIP_CDIR_SIG) {
            break;
        }

        let compression = read_u16_le(&cdir_header, 10);
        let compressed_size = read_u32_le(&cdir_header, 20);
        let uncompressed_size = read_u32_le(&cdir_header, 24);
        let name_len = read_u16_le(&cdir_header, 28) as usize;
        let extra_len = read_u16_le(&cdir_header, 30) as usize;
        let comment_len = read_u16_le(&cdir_header, 32) as usize;
        let local_header_offset = read_u32_le(&cdir_header, 42);

        let Some(next_cursor) = cdir_cursor
            .checked_add(ZIP_CDIR_HEADER_BYTES as u32)
            .and_then(|value| value.checked_add(name_len as u32))
            .and_then(|value| value.checked_add(extra_len as u32))
            .and_then(|value| value.checked_add(comment_len as u32))
        else {
            break;
        };

        let name_read_len = name_len.min(cdir_name.len());
        if name_read_len == 0 {
            cdir_cursor = next_cursor;
            continue;
        }

        let name_read = read_file_at(
            file,
            cdir_cursor.saturating_add(ZIP_CDIR_HEADER_BYTES as u32),
            &mut cdir_name[..name_read_len],
        )?;
        if name_read < name_read_len {
            break;
        }

        let name_complete = name_len <= cdir_name.len();
        let name_slice = &cdir_name[..name_read_len];
        if name_complete && is_text_resource_name(name_slice) {
            if skip_front_matter && path_is_probably_front_matter(name_slice) {
                cdir_cursor = next_cursor;
                continue;
            }

            let entry_ref = ZipEntryRef {
                compression,
                compressed_size,
                uncompressed_size,
                local_header_offset,
            };
            let mut resource = String::<PATH_BYTES>::new();
            copy_ascii_or_lossy(name_slice, &mut resource);
            if total == target_index && selected.is_none() {
                selected = Some((entry_ref, resource.clone(), total));
            }
            fallback_last = Some((entry_ref, resource, total));
            total = total.saturating_add(1);
        }

        cdir_cursor = next_cursor;
    }

    if total == 0 {
        return Ok(None);
    }

    if let Some((entry, resource, index)) = selected {
        return Ok(Some((entry, resource, index, total.max(1))));
    }
    if let Some((entry, resource, index)) = fallback_last {
        return Ok(Some((entry, resource, index, total.max(1))));
    }
    Ok(None)
}

fn cdir_text_entry_at<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
    const PATH_BYTES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    file_size: u32,
    target_index: u16,
) -> Result<Option<ChapterEntry<PATH_BYTES>>, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let with_filter = cdir_text_entry_at_with_filter(file, file_size, target_index, true)?;
    if with_filter.is_some() {
        return Ok(with_filter);
    }
    cdir_text_entry_at_with_filter(file, file_size, target_index, false)
}

fn cdir_info<D, T, const MAX_DIRS: usize, const MAX_FILES: usize, const MAX_VOLUMES: usize>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    file_size: u32,
) -> Result<Option<(u32, usize)>, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    if file_size < ZIP_EOCD_MIN_BYTES as u32 {
        return Ok(None);
    }

    let tail_len = (file_size as usize).min(ZIP_EOCD_SEARCH_WINDOW);
    let tail_start = file_size.saturating_sub(tail_len as u32);
    let mut tail = [0u8; ZIP_EOCD_SEARCH_WINDOW];
    let tail_read = read_file_at(file, tail_start, &mut tail[..tail_len])?;
    if tail_read < ZIP_EOCD_MIN_BYTES {
        return Ok(None);
    }

    let Some(eocd_pos) = (0..=tail_read.saturating_sub(4))
        .rev()
        .find(|idx| tail[*idx..].starts_with(&ZIP_EOCD_SIG))
    else {
        return Ok(None);
    };
    if eocd_pos + ZIP_EOCD_MIN_BYTES > tail_read {
        return Ok(None);
    }

    let cdir_offset = read_u32_le(&tail, eocd_pos + 16);
    let cdir_entries = read_u16_le(&tail, eocd_pos + 10) as usize;
    if cdir_offset >= file_size {
        return Ok(None);
    }

    Ok(Some((cdir_offset, cdir_entries)))
}

fn find_opf_entry_and_path<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
    const PATH_BYTES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    file_size: u32,
) -> Result<Option<(ZipEntryRef, String<PATH_BYTES>)>, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let Some((cdir_offset, cdir_entries)) = cdir_info(file, file_size)? else {
        return Ok(None);
    };

    let mut cdir_header = [0u8; ZIP_CDIR_HEADER_BYTES];
    let mut cdir_name = [0u8; ZIP_NAME_BYTES];
    let mut container_buf = [0u8; ZIP_CONTAINER_BYTES];
    let mut opf_path_from_container = String::<ZIP_PATH_BYTES>::new();

    let mut selected: Option<(ZipEntryRef, String<PATH_BYTES>)> = None;
    let mut fallback: Option<(ZipEntryRef, String<PATH_BYTES>)> = None;

    let mut cdir_cursor = cdir_offset;
    for _ in 0..cdir_entries.min(ZIP_MAX_CDIR_ENTRIES) {
        let header_read = read_file_at(file, cdir_cursor, &mut cdir_header)?;
        if header_read < ZIP_CDIR_HEADER_BYTES || !cdir_header.starts_with(&ZIP_CDIR_SIG) {
            break;
        }

        let compression = read_u16_le(&cdir_header, 10);
        let compressed_size = read_u32_le(&cdir_header, 20);
        let uncompressed_size = read_u32_le(&cdir_header, 24);
        let name_len = read_u16_le(&cdir_header, 28) as usize;
        let extra_len = read_u16_le(&cdir_header, 30) as usize;
        let comment_len = read_u16_le(&cdir_header, 32) as usize;
        let local_header_offset = read_u32_le(&cdir_header, 42);

        let Some(next_cursor) = cdir_cursor
            .checked_add(ZIP_CDIR_HEADER_BYTES as u32)
            .and_then(|value| value.checked_add(name_len as u32))
            .and_then(|value| value.checked_add(extra_len as u32))
            .and_then(|value| value.checked_add(comment_len as u32))
        else {
            break;
        };

        let name_read_len = name_len.min(cdir_name.len());
        if name_read_len == 0 {
            cdir_cursor = next_cursor;
            continue;
        }
        let name_read = read_file_at(
            file,
            cdir_cursor.saturating_add(ZIP_CDIR_HEADER_BYTES as u32),
            &mut cdir_name[..name_read_len],
        )?;
        if name_read < name_read_len {
            break;
        }
        let name_slice = &cdir_name[..name_read_len];
        let name_complete = name_len <= cdir_name.len();
        if !name_complete {
            cdir_cursor = next_cursor;
            continue;
        }

        let entry = ZipEntryRef {
            compression,
            compressed_size,
            uncompressed_size,
            local_header_offset,
        };
        let mut name_text = String::<PATH_BYTES>::new();
        copy_ascii_or_lossy(name_slice, &mut name_text);

        if opf_path_from_container.is_empty()
            && eq_ascii_case_insensitive(name_slice, b"META-INF/container.xml")
        {
            let read_now = read_zip_entry_prefix(file, entry, &mut container_buf)?;
            if read_now > 0 {
                let _ = parse_container_full_path(
                    &container_buf[..read_now],
                    &mut opf_path_from_container,
                );
            }
        }

        if selected.is_none()
            && !opf_path_from_container.is_empty()
            && eq_ascii_case_insensitive(name_slice, opf_path_from_container.as_bytes())
        {
            selected = Some((entry, name_text.clone()));
        }

        if fallback.is_none() && ends_with_ascii_case_insensitive(name_slice, b".opf") {
            fallback = Some((entry, name_text));
        }

        cdir_cursor = next_cursor;
    }

    Ok(selected.or(fallback))
}

fn find_entry_by_path<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    file_size: u32,
    path: &[u8],
) -> Result<Option<ZipEntryRef>, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let Some((cdir_offset, cdir_entries)) = cdir_info(file, file_size)? else {
        return Ok(None);
    };

    let mut cdir_header = [0u8; ZIP_CDIR_HEADER_BYTES];
    let mut cdir_name = [0u8; ZIP_NAME_BYTES];
    let mut cdir_cursor = cdir_offset;
    for _ in 0..cdir_entries.min(ZIP_MAX_CDIR_ENTRIES) {
        let header_read = read_file_at(file, cdir_cursor, &mut cdir_header)?;
        if header_read < ZIP_CDIR_HEADER_BYTES || !cdir_header.starts_with(&ZIP_CDIR_SIG) {
            break;
        }

        let compression = read_u16_le(&cdir_header, 10);
        let compressed_size = read_u32_le(&cdir_header, 20);
        let uncompressed_size = read_u32_le(&cdir_header, 24);
        let name_len = read_u16_le(&cdir_header, 28) as usize;
        let extra_len = read_u16_le(&cdir_header, 30) as usize;
        let comment_len = read_u16_le(&cdir_header, 32) as usize;
        let local_header_offset = read_u32_le(&cdir_header, 42);

        let Some(next_cursor) = cdir_cursor
            .checked_add(ZIP_CDIR_HEADER_BYTES as u32)
            .and_then(|value| value.checked_add(name_len as u32))
            .and_then(|value| value.checked_add(extra_len as u32))
            .and_then(|value| value.checked_add(comment_len as u32))
        else {
            break;
        };

        let name_read_len = name_len.min(cdir_name.len());
        if name_read_len == path.len() {
            let name_read = read_file_at(
                file,
                cdir_cursor.saturating_add(ZIP_CDIR_HEADER_BYTES as u32),
                &mut cdir_name[..name_read_len],
            )?;
            if name_read == name_read_len
                && eq_ascii_case_insensitive(&cdir_name[..name_read_len], path)
            {
                return Ok(Some(ZipEntryRef {
                    compression,
                    compressed_size,
                    uncompressed_size,
                    local_header_offset,
                }));
            }
        }

        cdir_cursor = next_cursor;
    }

    Ok(None)
}

fn find_fallback_cover_image_entry<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
    const PATH_BYTES: usize,
    const MEDIA_BYTES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    file_size: u32,
    exclude_path: Option<&[u8]>,
) -> CoverFallbackEntryResult<D::Error, PATH_BYTES, MEDIA_BYTES>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    find_fallback_cover_image_entry_with_skip::<
        D,
        T,
        MAX_DIRS,
        MAX_FILES,
        MAX_VOLUMES,
        PATH_BYTES,
        MEDIA_BYTES,
    >(file, file_size, exclude_path, 0)
}

fn find_fallback_cover_image_entry_with_skip<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
    const PATH_BYTES: usize,
    const MEDIA_BYTES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    file_size: u32,
    exclude_path: Option<&[u8]>,
    skip_count: usize,
) -> CoverFallbackEntryResult<D::Error, PATH_BYTES, MEDIA_BYTES>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let Some((cdir_offset, cdir_entries)) = cdir_info(file, file_size)? else {
        return Ok(None);
    };

    let mut cover_like_total = 0usize;
    let mut cdir_header = [0u8; ZIP_CDIR_HEADER_BYTES];
    let mut cdir_name = [0u8; ZIP_NAME_BYTES];
    let mut cdir_cursor = cdir_offset;

    for _ in 0..cdir_entries.min(ZIP_MAX_CDIR_ENTRIES) {
        let header_read = read_file_at(file, cdir_cursor, &mut cdir_header)?;
        if header_read < ZIP_CDIR_HEADER_BYTES || !cdir_header.starts_with(&ZIP_CDIR_SIG) {
            break;
        }

        let compression = read_u16_le(&cdir_header, 10);
        let compressed_size = read_u32_le(&cdir_header, 20);
        let uncompressed_size = read_u32_le(&cdir_header, 24);
        let name_len = read_u16_le(&cdir_header, 28) as usize;
        let extra_len = read_u16_le(&cdir_header, 30) as usize;
        let comment_len = read_u16_le(&cdir_header, 32) as usize;
        let local_header_offset = read_u32_le(&cdir_header, 42);

        let Some(next_cursor) = cdir_cursor
            .checked_add(ZIP_CDIR_HEADER_BYTES as u32)
            .and_then(|value| value.checked_add(name_len as u32))
            .and_then(|value| value.checked_add(extra_len as u32))
            .and_then(|value| value.checked_add(comment_len as u32))
        else {
            break;
        };

        let name_read_len = name_len.min(cdir_name.len());
        if name_read_len > 0 {
            let name_read = read_file_at(
                file,
                cdir_cursor.saturating_add(ZIP_CDIR_HEADER_BYTES as u32),
                &mut cdir_name[..name_read_len],
            )?;
            if name_read == name_read_len {
                let name_slice = &cdir_name[..name_read_len];
                let is_dir = name_slice.last().copied() == Some(b'/');
                if !is_dir {
                    if exclude_path
                        .is_some_and(|excluded| eq_ascii_case_insensitive(excluded, name_slice))
                    {
                        cdir_cursor = next_cursor;
                        continue;
                    }

                    let entry = ZipEntryRef {
                        compression,
                        compressed_size,
                        uncompressed_size,
                        local_header_offset,
                    };
                    let mut is_image = is_image_resource_name(name_slice)
                        || is_probable_image_resource_path(name_slice);
                    if !is_image {
                        is_image = sniff_cover_media_from_entry(file, entry)?.is_some();
                    }

                    if is_image && is_cover_like_image_path(name_slice) {
                        cover_like_total = cover_like_total.saturating_add(1);
                    }
                }
            }
        }

        cdir_cursor = next_cursor;
    }

    let target_cover_like = if skip_count < cover_like_total {
        Some(skip_count)
    } else {
        None
    };
    let target_other = if skip_count >= cover_like_total {
        Some(skip_count - cover_like_total)
    } else {
        None
    };

    let mut seen_cover_like = 0usize;
    let mut seen_other = 0usize;
    cdir_cursor = cdir_offset;

    for _ in 0..cdir_entries.min(ZIP_MAX_CDIR_ENTRIES) {
        let header_read = read_file_at(file, cdir_cursor, &mut cdir_header)?;
        if header_read < ZIP_CDIR_HEADER_BYTES || !cdir_header.starts_with(&ZIP_CDIR_SIG) {
            break;
        }

        let compression = read_u16_le(&cdir_header, 10);
        let compressed_size = read_u32_le(&cdir_header, 20);
        let uncompressed_size = read_u32_le(&cdir_header, 24);
        let name_len = read_u16_le(&cdir_header, 28) as usize;
        let extra_len = read_u16_le(&cdir_header, 30) as usize;
        let comment_len = read_u16_le(&cdir_header, 32) as usize;
        let local_header_offset = read_u32_le(&cdir_header, 42);

        let Some(next_cursor) = cdir_cursor
            .checked_add(ZIP_CDIR_HEADER_BYTES as u32)
            .and_then(|value| value.checked_add(name_len as u32))
            .and_then(|value| value.checked_add(extra_len as u32))
            .and_then(|value| value.checked_add(comment_len as u32))
        else {
            break;
        };

        let name_read_len = name_len.min(cdir_name.len());
        if name_read_len > 0 {
            let name_read = read_file_at(
                file,
                cdir_cursor.saturating_add(ZIP_CDIR_HEADER_BYTES as u32),
                &mut cdir_name[..name_read_len],
            )?;
            if name_read == name_read_len {
                let name_slice = &cdir_name[..name_read_len];
                let is_dir = name_slice.last().copied() == Some(b'/');
                if !is_dir {
                    if exclude_path
                        .is_some_and(|excluded| eq_ascii_case_insensitive(excluded, name_slice))
                    {
                        cdir_cursor = next_cursor;
                        continue;
                    }

                    let entry = ZipEntryRef {
                        compression,
                        compressed_size,
                        uncompressed_size,
                        local_header_offset,
                    };
                    let mut sniffed_media = None;
                    let mut is_image = is_image_resource_name(name_slice)
                        || is_probable_image_resource_path(name_slice);
                    if !is_image {
                        sniffed_media = sniff_cover_media_from_entry(file, entry)?;
                        is_image = sniffed_media.is_some();
                    }

                    if is_image {
                        let cover_like = is_cover_like_image_path(name_slice);
                        let match_candidate = if cover_like {
                            if let Some(target_idx) = target_cover_like {
                                if seen_cover_like == target_idx {
                                    true
                                } else {
                                    seen_cover_like = seen_cover_like.saturating_add(1);
                                    false
                                }
                            } else {
                                false
                            }
                        } else if let Some(target_idx) = target_other {
                            if seen_other == target_idx {
                                true
                            } else {
                                seen_other = seen_other.saturating_add(1);
                                false
                            }
                        } else {
                            false
                        };

                        if match_candidate {
                            let mut path = String::<PATH_BYTES>::new();
                            copy_ascii_or_lossy(name_slice, &mut path);
                            let mut media = String::<MEDIA_BYTES>::new();
                            copy_media_type_or_default(None, name_slice, &mut media);
                            if let Some(sniffed) = sniffed_media {
                                media.clear();
                                let _ = media.push_str(sniffed);
                            }
                            return Ok(Some((entry, path, media)));
                        }
                    }
                }
            }
        }

        cdir_cursor = next_cursor;
    }

    Ok(None)
}

fn parse_ascii_u32(token: &[u8]) -> Option<u32> {
    if token.is_empty() {
        return None;
    }

    let mut value = 0u32;
    for &byte in token {
        if !byte.is_ascii_digit() {
            return None;
        }
        let digit = (byte - b'0') as u32;
        value = value.checked_mul(10)?.checked_add(digit)?;
    }
    Some(value)
}

fn pbm_next_token<'a>(buf: &'a [u8], cursor: &mut usize) -> Option<&'a [u8]> {
    while *cursor < buf.len() {
        let byte = buf[*cursor];
        if byte.is_ascii_whitespace() {
            *cursor = cursor.saturating_add(1);
            continue;
        }
        if byte == b'#' {
            while *cursor < buf.len() && buf[*cursor] != b'\n' {
                *cursor = cursor.saturating_add(1);
            }
            continue;
        }
        break;
    }
    if *cursor >= buf.len() {
        return None;
    }

    let start = *cursor;
    while *cursor < buf.len() {
        let byte = buf[*cursor];
        if byte.is_ascii_whitespace() || byte == b'#' {
            break;
        }
        *cursor = cursor.saturating_add(1);
    }

    Some(&buf[start..*cursor])
}

fn mono_set_pixel(bits: &mut [u8], width: usize, x: usize, y: usize, on: bool) {
    if width == 0 {
        return;
    }
    let row_bytes = width.div_ceil(8);
    let idx = y.saturating_mul(row_bytes).saturating_add(x / 8);
    if idx >= bits.len() {
        return;
    }
    let mask = 1u8 << (7 - (x % 8));
    if on {
        bits[idx] |= mask;
    } else {
        bits[idx] &= !mask;
    }
}

fn decode_pbm_thumbnail_p4(
    pbm: &[u8],
    thumb_width: u16,
    thumb_height: u16,
    out_bits: &mut [u8],
) -> Option<(u16, u16, usize)> {
    let mut cursor = 0usize;
    let magic = pbm_next_token(pbm, &mut cursor)?;
    if !eq_ascii_case_insensitive(magic, b"P4") {
        return None;
    }

    let src_width = parse_ascii_u32(pbm_next_token(pbm, &mut cursor)?)?;
    let src_height = parse_ascii_u32(pbm_next_token(pbm, &mut cursor)?)?;
    if src_width == 0 || src_height == 0 {
        return None;
    }
    let src_width = src_width.min(u16::MAX as u32) as usize;
    let src_height = src_height.min(u16::MAX as u32) as usize;

    while cursor < pbm.len() && pbm[cursor].is_ascii_whitespace() {
        cursor = cursor.saturating_add(1);
    }
    if cursor >= pbm.len() {
        return None;
    }

    let src_row_bytes = src_width.div_ceil(8);
    let src_bitmap_bytes = src_row_bytes.checked_mul(src_height)?;
    if cursor.saturating_add(src_bitmap_bytes) > pbm.len() {
        return None;
    }
    let src_pixels = &pbm[cursor..cursor + src_bitmap_bytes];

    let tw = thumb_width.max(1) as usize;
    let th = thumb_height.max(1) as usize;
    let dst_row_bytes = tw.div_ceil(8);
    let dst_bytes = dst_row_bytes.checked_mul(th)?;
    if dst_bytes > out_bits.len() {
        return None;
    }
    out_bits[..dst_bytes].fill(0);

    for dy in 0..th {
        let sy = dy.saturating_mul(src_height) / th;
        for dx in 0..tw {
            let sx = dx.saturating_mul(src_width) / tw;
            let src_idx = sy.saturating_mul(src_row_bytes).saturating_add(sx / 8);
            if src_idx >= src_pixels.len() {
                continue;
            }
            let src_mask = 1u8 << (7 - (sx % 8));
            let on = (src_pixels[src_idx] & src_mask) != 0;
            mono_set_pixel(out_bits, tw, dx, dy, on);
        }
    }

    Some((src_width as u16, src_height as u16, dst_bytes))
}

fn read_u32_be(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn paeth_predictor(a: u8, b: u8, c: u8) -> u8 {
    let a_i = a as i32;
    let b_i = b as i32;
    let c_i = c as i32;
    let p = a_i + b_i - c_i;
    let pa = (p - a_i).abs();
    let pb = (p - b_i).abs();
    let pc = (p - c_i).abs();
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

fn apply_png_filter(
    filter: u8,
    raw: &[u8],
    prev_row: &[u8],
    bpp: usize,
    out_row: &mut [u8],
) -> bool {
    if raw.len() != out_row.len() || prev_row.len() < raw.len() || bpp == 0 {
        return false;
    }

    match filter {
        0 => {
            out_row.copy_from_slice(raw);
            true
        }
        1 => {
            for i in 0..raw.len() {
                let left = if i >= bpp { out_row[i - bpp] } else { 0 };
                out_row[i] = raw[i].wrapping_add(left);
            }
            true
        }
        2 => {
            for i in 0..raw.len() {
                out_row[i] = raw[i].wrapping_add(prev_row[i]);
            }
            true
        }
        3 => {
            for i in 0..raw.len() {
                let left = if i >= bpp { out_row[i - bpp] } else { 0 };
                let up = prev_row[i];
                let avg = ((left as u16 + up as u16) / 2) as u8;
                out_row[i] = raw[i].wrapping_add(avg);
            }
            true
        }
        4 => {
            for i in 0..raw.len() {
                let left = if i >= bpp { out_row[i - bpp] } else { 0 };
                let up = prev_row[i];
                let up_left = if i >= bpp { prev_row[i - bpp] } else { 0 };
                out_row[i] = raw[i].wrapping_add(paeth_predictor(left, up, up_left));
            }
            true
        }
        _ => false,
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "pixel sampling needs explicit palette + PNG mode params and stays allocation-free"
)]
fn mono_sample_from_row(
    row: &[u8],
    x: usize,
    color_type: u8,
    bit_depth: u8,
    channels: usize,
    palette_rgb: &[u8],
    palette_alpha: &[u8],
    palette_entries: usize,
) -> bool {
    fn sample_bits(row: &[u8], x: usize, bit_depth: u8) -> Option<u8> {
        if bit_depth == 0 || bit_depth > 8 {
            return None;
        }
        if bit_depth == 8 {
            return row.get(x).copied();
        }
        let depth = bit_depth as usize;
        let bits_per_row = row.len().saturating_mul(8);
        let bit_offset = x.saturating_mul(depth);
        if bit_offset + depth > bits_per_row {
            return None;
        }
        let byte_idx = bit_offset / 8;
        let bit_in_byte = bit_offset % 8;
        let shift = 8usize.saturating_sub(bit_in_byte + depth);
        let mask = ((1u16 << depth) - 1) as u8;
        Some((row[byte_idx] >> shift) & mask)
    }

    let pixel = x.saturating_mul(channels);

    let (luma, alpha) = match color_type {
        // grayscale
        0 => {
            let Some(value) = sample_bits(row, x, bit_depth) else {
                return false;
            };
            if bit_depth == 8 {
                (value as u32, 255u32)
            } else {
                let max = (1u32 << bit_depth) - 1;
                (((value as u32).saturating_mul(255) / max.max(1)), 255u32)
            }
        }
        // truecolor (RGB)
        2 => {
            if pixel + channels > row.len() {
                return false;
            }
            let r = row[pixel] as u32;
            let g = row[pixel + 1] as u32;
            let b = row[pixel + 2] as u32;
            (((r * 30 + g * 59 + b * 11) / 100), 255u32)
        }
        // grayscale + alpha
        4 => {
            if pixel + channels > row.len() {
                return false;
            }
            (row[pixel] as u32, row[pixel + 1] as u32)
        }
        // RGBA
        6 => {
            if pixel + channels > row.len() {
                return false;
            }
            let r = row[pixel] as u32;
            let g = row[pixel + 1] as u32;
            let b = row[pixel + 2] as u32;
            let a = row[pixel + 3] as u32;
            (((r * 30 + g * 59 + b * 11) / 100), a)
        }
        // indexed-color palette
        3 => {
            let Some(idx) = sample_bits(row, x, bit_depth).map(|value| value as usize) else {
                return false;
            };
            if idx >= palette_entries {
                return false;
            }
            let base = idx.saturating_mul(3);
            if base + 2 >= palette_rgb.len() {
                return false;
            }
            let r = palette_rgb[base] as u32;
            let g = palette_rgb[base + 1] as u32;
            let b = palette_rgb[base + 2] as u32;
            let a = palette_alpha.get(idx).copied().unwrap_or(255) as u32;
            (((r * 30 + g * 59 + b * 11) / 100), a)
        }
        _ => return false,
    };

    // Composite against white background before thresholding to 1bpp.
    let comp = (luma.saturating_mul(alpha) + 255 * (255 - alpha)) / 255;
    comp < 160
}

#[allow(
    clippy::too_many_arguments,
    reason = "streamed ZIP reads require explicit parser state to avoid heap allocations"
)]
fn zip_entry_stream_read_exact<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
    const STREAM_BYTES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    entry: ZipEntryRef,
    stream_offset: &mut u32,
    stream_buf: &mut [u8; STREAM_BYTES],
    stream_len: &mut usize,
    stream_pos: &mut usize,
    stream_end: &mut bool,
    out: &mut [u8],
) -> Result<bool, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let mut copied = 0usize;
    let mut no_progress_reads = 0u8;
    while copied < out.len() {
        if *stream_pos >= *stream_len {
            if *stream_end {
                return Ok(false);
            }
            let (read_now, end) = read_zip_entry_chunk(file, entry, *stream_offset, stream_buf)?;
            *stream_offset = stream_offset.saturating_add(read_now as u32);
            *stream_len = read_now;
            *stream_pos = 0;
            *stream_end = end;
            if read_now == 0 {
                if end {
                    return Ok(false);
                }
                no_progress_reads = no_progress_reads.saturating_add(1);
                if no_progress_reads >= 4 {
                    return Ok(false);
                }
                continue;
            }
            no_progress_reads = 0;
        }

        let avail = (*stream_len - *stream_pos).min(out.len() - copied);
        out[copied..copied + avail].copy_from_slice(&stream_buf[*stream_pos..*stream_pos + avail]);
        *stream_pos += avail;
        copied += avail;
    }

    Ok(true)
}

#[allow(
    clippy::too_many_arguments,
    reason = "streamed ZIP parser keeps state external and explicit for no_std usage"
)]
fn zip_entry_stream_skip<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
    const STREAM_BYTES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    entry: ZipEntryRef,
    stream_offset: &mut u32,
    stream_buf: &mut [u8; STREAM_BYTES],
    stream_len: &mut usize,
    stream_pos: &mut usize,
    stream_end: &mut bool,
    mut skip_len: usize,
) -> Result<bool, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let mut no_progress_reads = 0u8;
    while skip_len > 0 {
        if *stream_pos >= *stream_len {
            if *stream_end {
                return Ok(false);
            }
            let (read_now, end) = read_zip_entry_chunk(file, entry, *stream_offset, stream_buf)?;
            *stream_offset = stream_offset.saturating_add(read_now as u32);
            *stream_len = read_now;
            *stream_pos = 0;
            *stream_end = end;
            if read_now == 0 {
                if end {
                    return Ok(false);
                }
                no_progress_reads = no_progress_reads.saturating_add(1);
                if no_progress_reads >= 4 {
                    return Ok(false);
                }
                continue;
            }
            no_progress_reads = 0;
        }

        let avail = (*stream_len - *stream_pos).min(skip_len);
        *stream_pos += avail;
        skip_len -= avail;
    }

    Ok(true)
}

fn decode_png_thumbnail_stream<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    entry: ZipEntryRef,
    thumb_width: u16,
    thumb_height: u16,
    out_bits: &mut [u8],
) -> PngThumbDecodeResult<D::Error>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let tw = thumb_width.max(1) as usize;
    let th = thumb_height.max(1) as usize;
    let dst_row_bytes = tw.div_ceil(8);
    let dst_bytes = dst_row_bytes.saturating_mul(th);
    if dst_bytes == 0 || dst_bytes > out_bits.len() {
        return Ok(None);
    }
    out_bits[..dst_bytes].fill(0);

    let mut stream_buf = [0u8; PNG_STREAM_BYTES];
    let mut stream_offset = 0u32;
    let mut stream_len = 0usize;
    let mut stream_pos = 0usize;
    let mut stream_end = false;

    let mut sig = [0u8; 8];
    if !zip_entry_stream_read_exact::<_, _, _, _, _, PNG_STREAM_BYTES>(
        file,
        entry,
        &mut stream_offset,
        &mut stream_buf,
        &mut stream_len,
        &mut stream_pos,
        &mut stream_end,
        &mut sig,
    )? {
        return Ok(None);
    }
    if sig != [137, 80, 78, 71, 13, 10, 26, 10] {
        info!("sd: png decode fail reason=bad_signature");
        return Ok(None);
    }

    let mut src_width = 0usize;
    let mut src_height = 0usize;
    let mut channels = 0usize;
    let mut color_type = 0u8;
    let mut bit_depth = 0u8;
    let mut bpp = 0usize;
    let mut row_raw_len = 0usize;
    let mut row_len = 0usize;
    let mut header_ok = false;
    let mut palette_rgb = [0u8; 256 * 3];
    let mut palette_alpha = [255u8; 256];
    let mut palette_entries = 0usize;

    let mut inflater = InflateState::new(DataFormat::Zlib);
    let mut inflate_out = [0u8; PNG_INFLATE_OUT_BYTES];
    let mut idat_input = [0u8; PNG_IDAT_IN_BYTES];
    let mut row_accum = [0u8; PNG_ROW_BYTES_MAX];
    let mut row_prev = [0u8; PNG_ROW_BYTES_MAX];
    let mut row_cur = [0u8; PNG_ROW_BYTES_MAX];
    let mut row_fill = 0usize;
    let mut row_index = 0usize;
    let mut saw_idat = false;

    let mut chunk_header = [0u8; 8];
    let mut chunk_crc = [0u8; 4];
    loop {
        if !zip_entry_stream_read_exact::<_, _, _, _, _, PNG_STREAM_BYTES>(
            file,
            entry,
            &mut stream_offset,
            &mut stream_buf,
            &mut stream_len,
            &mut stream_pos,
            &mut stream_end,
            &mut chunk_header,
        )? {
            break;
        }

        let chunk_len = read_u32_be(&chunk_header, 0) as usize;
        let chunk_type = &chunk_header[4..8];
        if chunk_type == b"IHDR" {
            if chunk_len != 13 {
                return Ok(None);
            }
            let mut ihdr = [0u8; 13];
            if !zip_entry_stream_read_exact::<_, _, _, _, _, PNG_STREAM_BYTES>(
                file,
                entry,
                &mut stream_offset,
                &mut stream_buf,
                &mut stream_len,
                &mut stream_pos,
                &mut stream_end,
                &mut ihdr,
            )? {
                return Ok(None);
            }
            src_width = read_u32_be(&ihdr, 0) as usize;
            src_height = read_u32_be(&ihdr, 4) as usize;
            bit_depth = ihdr[8];
            color_type = ihdr[9];
            let compression_method = ihdr[10];
            let filter_method = ihdr[11];
            let interlace_method = ihdr[12];
            if src_width == 0
                || src_height == 0
                || compression_method != 0
                || filter_method != 0
                || interlace_method != 0
            {
                info!(
                    "sd: png decode fail reason=unsupported_ihdr width={} height={} bit_depth={} color_type={} compression={} filter={} interlace={}",
                    src_width,
                    src_height,
                    bit_depth,
                    color_type,
                    compression_method,
                    filter_method,
                    interlace_method
                );
                return Ok(None);
            }
            channels = match color_type {
                0 => 1,
                2 => 3,
                3 => 1,
                4 => 2,
                6 => 4,
                _ => {
                    info!(
                        "sd: png decode fail reason=unsupported_color_type color_type={}",
                        color_type
                    );
                    return Ok(None);
                }
            };
            let bits_per_pixel = channels.saturating_mul(bit_depth as usize);
            if bits_per_pixel == 0 {
                return Ok(None);
            }
            // Keep implementation compact: packed samples only for grayscale/palette.
            if bit_depth != 8 && !matches!(color_type, 0 | 3) {
                info!(
                    "sd: png decode fail reason=unsupported_bit_depth color_type={} bit_depth={}",
                    color_type, bit_depth
                );
                return Ok(None);
            }
            let row_bits = match src_width.checked_mul(bits_per_pixel) {
                Some(value) => value,
                None => return Ok(None),
            };
            row_raw_len = row_bits.div_ceil(8);
            row_len = match row_raw_len.checked_add(1) {
                Some(value) => value,
                None => return Ok(None),
            };
            if row_len == 0 || row_len > PNG_ROW_BYTES_MAX {
                info!(
                    "sd: png decode fail reason=row_too_wide row_len={} max={}",
                    row_len, PNG_ROW_BYTES_MAX
                );
                return Ok(None);
            }
            bpp = bits_per_pixel.div_ceil(8).max(1);
            header_ok = true;
        } else if chunk_type == b"PLTE" {
            if chunk_len == 0 || !chunk_len.is_multiple_of(3) {
                info!(
                    "sd: png decode fail reason=invalid_plte_len chunk_len={}",
                    chunk_len
                );
                return Ok(None);
            }
            if chunk_len > palette_rgb.len() {
                info!(
                    "sd: png decode fail reason=plte_too_large chunk_len={}",
                    chunk_len
                );
                return Ok(None);
            }
            let entries = chunk_len / 3;
            if !zip_entry_stream_read_exact::<_, _, _, _, _, PNG_STREAM_BYTES>(
                file,
                entry,
                &mut stream_offset,
                &mut stream_buf,
                &mut stream_len,
                &mut stream_pos,
                &mut stream_end,
                &mut palette_rgb[..chunk_len],
            )? {
                return Ok(None);
            }
            palette_entries = entries;
            palette_alpha.fill(255);
        } else if chunk_type == b"tRNS" {
            if color_type != 3 {
                if chunk_len > 0
                    && !zip_entry_stream_skip::<_, _, _, _, _, PNG_STREAM_BYTES>(
                        file,
                        entry,
                        &mut stream_offset,
                        &mut stream_buf,
                        &mut stream_len,
                        &mut stream_pos,
                        &mut stream_end,
                        chunk_len,
                    )?
                {
                    return Ok(None);
                }
            } else {
                let take = chunk_len.min(palette_alpha.len());
                if take > 0
                    && !zip_entry_stream_read_exact::<_, _, _, _, _, PNG_STREAM_BYTES>(
                        file,
                        entry,
                        &mut stream_offset,
                        &mut stream_buf,
                        &mut stream_len,
                        &mut stream_pos,
                        &mut stream_end,
                        &mut palette_alpha[..take],
                    )?
                {
                    return Ok(None);
                }
                if chunk_len > take
                    && !zip_entry_stream_skip::<_, _, _, _, _, PNG_STREAM_BYTES>(
                        file,
                        entry,
                        &mut stream_offset,
                        &mut stream_buf,
                        &mut stream_len,
                        &mut stream_pos,
                        &mut stream_end,
                        chunk_len - take,
                    )?
                {
                    return Ok(None);
                }
            }
        } else if chunk_type == b"IDAT" {
            if !header_ok {
                info!("sd: png decode fail reason=idat_before_ihdr");
                return Ok(None);
            }
            if color_type == 3 && palette_entries == 0 {
                info!("sd: png decode fail reason=missing_palette");
                return Ok(None);
            }
            saw_idat = true;

            let mut remaining = chunk_len;
            while remaining > 0 {
                let take = remaining.min(idat_input.len());
                if !zip_entry_stream_read_exact::<_, _, _, _, _, PNG_STREAM_BYTES>(
                    file,
                    entry,
                    &mut stream_offset,
                    &mut stream_buf,
                    &mut stream_len,
                    &mut stream_pos,
                    &mut stream_end,
                    &mut idat_input[..take],
                )? {
                    return Ok(None);
                }
                remaining -= take;

                let mut in_pos = 0usize;
                while in_pos < take {
                    let stream = inflate(
                        &mut inflater,
                        &idat_input[in_pos..take],
                        &mut inflate_out,
                        MZFlush::None,
                    );
                    in_pos = in_pos.saturating_add(stream.bytes_consumed);
                    if stream.bytes_written > 0 {
                        for &byte in &inflate_out[..stream.bytes_written] {
                            if row_fill < row_len {
                                row_accum[row_fill] = byte;
                                row_fill += 1;
                            }
                            if row_fill == row_len {
                                let filter = row_accum[0];
                                if !apply_png_filter(
                                    filter,
                                    &row_accum[1..row_len],
                                    &row_prev[..row_raw_len],
                                    bpp,
                                    &mut row_cur[..row_raw_len],
                                ) {
                                    info!(
                                        "sd: png decode fail reason=bad_filter filter={}",
                                        filter
                                    );
                                    return Ok(None);
                                }

                                for dy in 0..th {
                                    let sy = dy.saturating_mul(src_height) / th;
                                    if sy != row_index {
                                        continue;
                                    }
                                    for dx in 0..tw {
                                        let sx = dx.saturating_mul(src_width) / tw;
                                        if mono_sample_from_row(
                                            &row_cur[..row_raw_len],
                                            sx,
                                            color_type,
                                            bit_depth,
                                            channels,
                                            &palette_rgb,
                                            &palette_alpha,
                                            palette_entries,
                                        ) {
                                            mono_set_pixel(out_bits, tw, dx, dy, true);
                                        }
                                    }
                                }

                                row_prev[..row_raw_len].copy_from_slice(&row_cur[..row_raw_len]);
                                row_fill = 0;
                                row_index = row_index.saturating_add(1);
                            }
                        }
                    }

                    match stream.status {
                        Ok(MZStatus::Ok) | Ok(MZStatus::NeedDict) | Ok(MZStatus::StreamEnd) => {}
                        Err(MZError::Buf) => {}
                        _ => {
                            info!(
                                "sd: png decode fail reason=inflate_status status={:?}",
                                stream.status
                            );
                            return Ok(None);
                        }
                    }

                    if stream.bytes_consumed == 0 && stream.bytes_written == 0 {
                        break;
                    }
                }
            }
        } else if chunk_type == b"IEND" {
            if chunk_len > 0
                && !zip_entry_stream_skip::<_, _, _, _, _, PNG_STREAM_BYTES>(
                    file,
                    entry,
                    &mut stream_offset,
                    &mut stream_buf,
                    &mut stream_len,
                    &mut stream_pos,
                    &mut stream_end,
                    chunk_len,
                )?
            {
                return Ok(None);
            }
            if !zip_entry_stream_read_exact::<_, _, _, _, _, PNG_STREAM_BYTES>(
                file,
                entry,
                &mut stream_offset,
                &mut stream_buf,
                &mut stream_len,
                &mut stream_pos,
                &mut stream_end,
                &mut chunk_crc,
            )? {
                return Ok(None);
            }
            break;
        } else if chunk_len > 0
            && !zip_entry_stream_skip::<_, _, _, _, _, PNG_STREAM_BYTES>(
                file,
                entry,
                &mut stream_offset,
                &mut stream_buf,
                &mut stream_len,
                &mut stream_pos,
                &mut stream_end,
                chunk_len,
            )?
        {
            return Ok(None);
        }

        if !zip_entry_stream_read_exact::<_, _, _, _, _, PNG_STREAM_BYTES>(
            file,
            entry,
            &mut stream_offset,
            &mut stream_buf,
            &mut stream_len,
            &mut stream_pos,
            &mut stream_end,
            &mut chunk_crc,
        )? {
            return Ok(None);
        }
    }

    if !header_ok || !saw_idat || src_width == 0 || src_height == 0 {
        info!(
            "sd: png decode fail reason=incomplete_stream header_ok={} saw_idat={} width={} height={}",
            header_ok, saw_idat, src_width, src_height
        );
        return Ok(None);
    }

    Ok(Some((src_width as u16, src_height as u16, dst_bytes)))
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct JpegRect {
    left: u16,
    right: u16,
    top: u16,
    bottom: u16,
}

type JpegReadFn = unsafe fn(*mut c_void, *mut u8, usize) -> usize;
type JpegSkipFn = unsafe fn(*mut c_void, usize) -> usize;

#[derive(Clone, Copy)]
struct JpegIoDispatch {
    user: *mut c_void,
    read: JpegReadFn,
    skip: JpegSkipFn,
}

struct JpegThumbContext {
    out_bits: *mut u8,
    out_len: usize,
    thumb_width: usize,
    thumb_height: usize,
    source_width: usize,
    source_height: usize,
    max_right: usize,
    max_bottom: usize,
    pixels_written: usize,
}

impl JpegThumbContext {
    fn set_thumb_pixel(&mut self, x: usize, y: usize, on: bool) {
        if x >= self.thumb_width || y >= self.thumb_height {
            return;
        }
        if self.out_bits.is_null() || self.out_len == 0 {
            return;
        }
        // SAFETY: Pointer and length are initialized from a valid mutable slice
        // in decode_jpeg_thumbnail_stream and stay valid for callback lifetime.
        let out = unsafe { core::slice::from_raw_parts_mut(self.out_bits, self.out_len) };
        mono_set_pixel(out, self.thumb_width, x, y, on);
    }
}

static mut JPEG_IO_DISPATCH: Option<JpegIoDispatch> = None;
static mut JPEG_THUMB_CTX: *mut JpegThumbContext = core::ptr::null_mut();

#[inline]
unsafe fn jpeg_io_dispatch_load() -> Option<JpegIoDispatch> {
    // SAFETY: Access is guarded by single-threaded decode guard discipline.
    unsafe { core::ptr::read(core::ptr::addr_of!(JPEG_IO_DISPATCH)) }
}

#[inline]
unsafe fn jpeg_io_dispatch_store(value: Option<JpegIoDispatch>) {
    // SAFETY: Access is guarded by single-threaded decode guard discipline.
    unsafe { core::ptr::write(core::ptr::addr_of_mut!(JPEG_IO_DISPATCH), value) }
}

#[inline]
unsafe fn jpeg_thumb_ctx_load() -> *mut JpegThumbContext {
    // SAFETY: Access is guarded by single-threaded decode guard discipline.
    unsafe { core::ptr::read(core::ptr::addr_of!(JPEG_THUMB_CTX)) }
}

#[inline]
unsafe fn jpeg_thumb_ctx_store(value: *mut JpegThumbContext) {
    // SAFETY: Access is guarded by single-threaded decode guard discipline.
    unsafe { core::ptr::write(core::ptr::addr_of_mut!(JPEG_THUMB_CTX), value) }
}

struct JpegCallbackGuard;

impl JpegCallbackGuard {
    unsafe fn install(io: JpegIoDispatch, thumb_ctx: *mut JpegThumbContext) -> Option<Self> {
        // SAFETY: Access to global callback state is guarded by this install/drop pair;
        // only one JPEG decode is allowed concurrently.
        unsafe {
            if jpeg_io_dispatch_load().is_some() || !jpeg_thumb_ctx_load().is_null() {
                return None;
            }
            jpeg_io_dispatch_store(Some(io));
            jpeg_thumb_ctx_store(thumb_ctx);
        }
        Some(Self)
    }
}

impl Drop for JpegCallbackGuard {
    fn drop(&mut self) {
        // SAFETY: Clear global callback context on scope exit.
        unsafe {
            jpeg_io_dispatch_store(None);
            jpeg_thumb_ctx_store(core::ptr::null_mut());
        }
    }
}

unsafe extern "C" {
    fn jd_prepare(
        jd: *mut c_void,
        infunc: Option<unsafe extern "C" fn(*mut c_void, *mut u8, u32) -> u32>,
        pool: *mut c_void,
        sz_pool: u32,
        device: *mut c_void,
    ) -> i32;

    fn jd_decomp(
        jd: *mut c_void,
        outfunc: Option<unsafe extern "C" fn(*mut c_void, *mut c_void, *mut JpegRect) -> u32>,
        scale: u8,
    ) -> i32;
}

const JDR_OK: i32 = 0;
const JDR_INTR: i32 = 1;
const JDR_INP: i32 = 2;
const JDR_MEM1: i32 = 3;
const JDR_MEM2: i32 = 4;
const JDR_PAR: i32 = 5;
const JDR_FMT1: i32 = 6;
const JDR_FMT2: i32 = 7;
const JDR_FMT3: i32 = 8;

fn jpeg_jdr_name(status: i32) -> &'static str {
    match status {
        JDR_OK => "ok",
        JDR_INTR => "intr",
        JDR_INP => "inp",
        JDR_MEM1 => "mem1",
        JDR_MEM2 => "mem2",
        JDR_PAR => "par",
        JDR_FMT1 => "fmt1",
        JDR_FMT2 => "fmt2",
        JDR_FMT3 => "fmt3_progressive_or_unsupported",
        _ => "unknown",
    }
}

unsafe extern "C" fn jpeg_in_callback(_jd: *mut c_void, buff: *mut u8, nbyte: u32) -> u32 {
    let want = nbyte as usize;
    if want == 0 {
        return 0;
    }

    // SAFETY: Read-only copy from global dispatch installed by JpegCallbackGuard.
    // SAFETY: Read callback dispatch installed by JpegCallbackGuard.
    let dispatch = unsafe { jpeg_io_dispatch_load() };
    let Some(io) = dispatch else {
        return 0;
    };

    let read = if buff.is_null() {
        // SAFETY: Function pointer/user context are installed for this decode.
        unsafe { (io.skip)(io.user, want) }
    } else {
        // SAFETY: Function pointer/user context are installed for this decode.
        unsafe { (io.read)(io.user, buff, want) }
    };
    read.min(want) as u32
}

unsafe extern "C" fn jpeg_out_callback(
    _jd: *mut c_void,
    bitmap: *mut c_void,
    rect: *mut JpegRect,
) -> u32 {
    if bitmap.is_null() || rect.is_null() {
        return 0;
    }

    // SAFETY: Global pointer is installed for this decode and cleared by guard.
    // SAFETY: Read callback output context installed by JpegCallbackGuard.
    let ctx_ptr = unsafe { jpeg_thumb_ctx_load() };
    if ctx_ptr.is_null() {
        return 0;
    }
    // SAFETY: Pointer validity is guaranteed by decode_jpeg_thumbnail_stream scope.
    let ctx = unsafe { &mut *ctx_ptr };

    // SAFETY: Decoder provides a valid rectangle pointer for callback duration.
    let rect = unsafe { *rect };
    let left = rect.left as usize;
    let top = rect.top as usize;
    let right = rect.right as usize;
    let bottom = rect.bottom as usize;
    if right < left || bottom < top {
        return 0;
    }

    let block_w = right.saturating_sub(left).saturating_add(1);
    let block_h = bottom.saturating_sub(top).saturating_add(1);
    if block_w == 0 || block_h == 0 {
        return 1;
    }

    // ESP32-S3 ROM TJPGD uses RGB888 output blocks.
    let block_pixels = block_w.saturating_mul(block_h);
    let block_bytes = block_pixels.saturating_mul(3);
    if block_bytes == 0 {
        return 0;
    }

    // SAFETY: TJPGD callback hands contiguous block bitmap bytes.
    let pixels = unsafe { core::slice::from_raw_parts(bitmap as *const u8, block_bytes) };
    let src_w = ctx.source_width.max(1);
    let src_h = ctx.source_height.max(1);

    for by in 0..block_h {
        let sy = top.saturating_add(by);
        if sy >= src_h {
            continue;
        }
        let dy = sy.saturating_mul(ctx.thumb_height) / src_h;
        if dy >= ctx.thumb_height {
            continue;
        }

        for bx in 0..block_w {
            let sx = left.saturating_add(bx);
            if sx >= src_w {
                continue;
            }
            let dx = sx.saturating_mul(ctx.thumb_width) / src_w;
            if dx >= ctx.thumb_width {
                continue;
            }

            let base = (by.saturating_mul(block_w).saturating_add(bx)).saturating_mul(3);
            if base + 2 >= pixels.len() {
                return 0;
            }

            let r = pixels[base] as u32;
            let g = pixels[base + 1] as u32;
            let b = pixels[base + 2] as u32;
            let luma = (r * 30 + g * 59 + b * 11) / 100;
            ctx.set_thumb_pixel(dx, dy, luma < 160);
        }
    }

    ctx.max_right = ctx.max_right.max(right);
    ctx.max_bottom = ctx.max_bottom.max(bottom);
    ctx.pixels_written = ctx.pixels_written.saturating_add(block_pixels);
    1
}

struct ZipJpegStream<
    'a,
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
> where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    file: *mut embedded_sdmmc::File<'a, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    entry: ZipEntryRef,
    stream_offset: u32,
    stream_buf: [u8; JPEG_STREAM_BYTES],
    stream_len: usize,
    stream_pos: usize,
    stream_end: bool,
    io_failed: bool,
}

impl<'a, D, T, const MAX_DIRS: usize, const MAX_FILES: usize, const MAX_VOLUMES: usize>
    ZipJpegStream<'a, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    fn new(
        file: &mut embedded_sdmmc::File<'a, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
        entry: ZipEntryRef,
    ) -> Self {
        Self {
            file,
            entry,
            stream_offset: 0,
            stream_buf: [0u8; JPEG_STREAM_BYTES],
            stream_len: 0,
            stream_pos: 0,
            stream_end: false,
            io_failed: false,
        }
    }

    fn read_into(&mut self, out: &mut [u8]) -> Result<usize, embedded_sdmmc::Error<D::Error>> {
        if out.is_empty() {
            return Ok(0);
        }

        let mut copied = 0usize;
        let mut no_progress_reads = 0u8;
        while copied < out.len() {
            if self.stream_pos >= self.stream_len {
                if self.stream_end {
                    break;
                }

                // SAFETY: File pointer comes from a unique mutable borrow in decode scope.
                let file = unsafe { &mut *self.file };
                let (read_now, end_now) = read_zip_entry_chunk(
                    file,
                    self.entry,
                    self.stream_offset,
                    &mut self.stream_buf,
                )?;
                self.stream_offset = self.stream_offset.saturating_add(read_now as u32);
                self.stream_len = read_now;
                self.stream_pos = 0;
                self.stream_end = end_now;

                if read_now == 0 {
                    no_progress_reads = no_progress_reads.saturating_add(1);
                    if self.stream_end || no_progress_reads >= 4 {
                        break;
                    }
                    continue;
                }
                no_progress_reads = 0;
            }

            let avail = (self.stream_len - self.stream_pos).min(out.len() - copied);
            out[copied..copied + avail]
                .copy_from_slice(&self.stream_buf[self.stream_pos..self.stream_pos + avail]);
            self.stream_pos = self.stream_pos.saturating_add(avail);
            copied = copied.saturating_add(avail);
        }

        Ok(copied)
    }

    fn skip_bytes(&mut self, mut len: usize) -> Result<usize, embedded_sdmmc::Error<D::Error>> {
        let mut skipped = 0usize;
        let mut no_progress_reads = 0u8;
        while len > 0 {
            if self.stream_pos >= self.stream_len {
                if self.stream_end {
                    break;
                }

                // SAFETY: File pointer comes from a unique mutable borrow in decode scope.
                let file = unsafe { &mut *self.file };
                let (read_now, end_now) = read_zip_entry_chunk(
                    file,
                    self.entry,
                    self.stream_offset,
                    &mut self.stream_buf,
                )?;
                self.stream_offset = self.stream_offset.saturating_add(read_now as u32);
                self.stream_len = read_now;
                self.stream_pos = 0;
                self.stream_end = end_now;

                if read_now == 0 {
                    no_progress_reads = no_progress_reads.saturating_add(1);
                    if self.stream_end || no_progress_reads >= 4 {
                        break;
                    }
                    continue;
                }
                no_progress_reads = 0;
            }

            let avail = (self.stream_len - self.stream_pos).min(len);
            self.stream_pos = self.stream_pos.saturating_add(avail);
            len = len.saturating_sub(avail);
            skipped = skipped.saturating_add(avail);
        }

        Ok(skipped)
    }

    fn read_byte(&mut self) -> Result<Option<u8>, embedded_sdmmc::Error<D::Error>> {
        let mut no_progress_reads = 0u8;
        loop {
            if self.stream_pos < self.stream_len {
                let byte = self.stream_buf[self.stream_pos];
                self.stream_pos = self.stream_pos.saturating_add(1);
                return Ok(Some(byte));
            }

            if self.stream_end {
                return Ok(None);
            }

            // SAFETY: File pointer comes from a unique mutable borrow in decode scope.
            let file = unsafe { &mut *self.file };
            let (read_now, end_now) =
                read_zip_entry_chunk(file, self.entry, self.stream_offset, &mut self.stream_buf)?;
            self.stream_offset = self.stream_offset.saturating_add(read_now as u32);
            self.stream_len = read_now;
            self.stream_pos = 0;
            self.stream_end = end_now;
            if read_now == 0 {
                no_progress_reads = no_progress_reads.saturating_add(1);
                if self.stream_end || no_progress_reads >= 4 {
                    return Ok(None);
                }
                continue;
            }
            no_progress_reads = 0;
        }
    }

    fn bytes_consumed(&self) -> usize {
        (self.stream_offset as usize)
            .saturating_sub(self.stream_len.saturating_sub(self.stream_pos))
    }
}

fn is_jpeg_sof_marker(marker: u8) -> bool {
    matches!(
        marker,
        0xC0 | 0xC1 | 0xC2 | 0xC3 | 0xC5 | 0xC6 | 0xC7 | 0xC9 | 0xCA | 0xCB | 0xCD | 0xCE | 0xCF
    )
}

fn parse_jpeg_dimensions_stream<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    entry: ZipEntryRef,
) -> JpegDimScanResult<D::Error>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let mut stream = ZipJpegStream::new(file, entry);
    let Some(first) = stream.read_byte()? else {
        return Ok(None);
    };
    let Some(second) = stream.read_byte()? else {
        return Ok(None);
    };
    if first != 0xFF || second != 0xD8 {
        return Ok(None);
    }

    loop {
        if stream.bytes_consumed() > JPEG_DIM_SCAN_MAX_BYTES {
            info!(
                "sd: jpeg decode fail reason=dims_scan_limit scanned={}",
                stream.bytes_consumed()
            );
            return Ok(None);
        }

        // Find marker prefix.
        let mut byte = loop {
            let Some(next) = stream.read_byte()? else {
                return Ok(None);
            };
            if next == 0xFF {
                break next;
            }
        };

        // Collapse fill bytes 0xFF... then read marker code.
        while byte == 0xFF {
            let Some(next) = stream.read_byte()? else {
                return Ok(None);
            };
            byte = next;
        }
        let marker = byte;

        // Stuffed 0xFF00 is possible in entropy data; ignore.
        if marker == 0x00 {
            continue;
        }
        if marker == 0xD9 || marker == 0xDA {
            return Ok(None);
        }
        if marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            continue;
        }

        let Some(len_hi) = stream.read_byte()? else {
            return Ok(None);
        };
        let Some(len_lo) = stream.read_byte()? else {
            return Ok(None);
        };
        let seg_len = u16::from_be_bytes([len_hi, len_lo]) as usize;
        if seg_len < 2 {
            return Ok(None);
        }
        let payload_len = seg_len - 2;

        if is_jpeg_sof_marker(marker) {
            if payload_len < 5 {
                return Ok(None);
            }

            let Some(_precision) = stream.read_byte()? else {
                return Ok(None);
            };
            let Some(h_hi) = stream.read_byte()? else {
                return Ok(None);
            };
            let Some(h_lo) = stream.read_byte()? else {
                return Ok(None);
            };
            let Some(w_hi) = stream.read_byte()? else {
                return Ok(None);
            };
            let Some(w_lo) = stream.read_byte()? else {
                return Ok(None);
            };
            let height = u16::from_be_bytes([h_hi, h_lo]);
            let width = u16::from_be_bytes([w_hi, w_lo]);

            if payload_len > 5 {
                let skipped = stream.skip_bytes(payload_len - 5)?;
                if skipped < payload_len - 5 {
                    return Ok(None);
                }
            }

            if width > 0 && height > 0 {
                return Ok(Some((width, height, stream.bytes_consumed())));
            }
            return Ok(None);
        }

        let skipped = stream.skip_bytes(payload_len)?;
        if skipped < payload_len {
            return Ok(None);
        }
    }
}

unsafe fn zip_jpeg_read_dispatch<
    'a,
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    user: *mut c_void,
    buff: *mut u8,
    len: usize,
) -> usize
where
    D: embedded_sdmmc::BlockDevice + 'a,
    T: TimeSource + 'a,
{
    if user.is_null() || buff.is_null() || len == 0 {
        return 0;
    }

    // SAFETY: `user` is installed from a live ZipJpegStream in decode scope.
    let stream =
        unsafe { &mut *(user as *mut ZipJpegStream<'a, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>) };
    if stream.io_failed {
        return 0;
    }

    // SAFETY: `buff` comes from TJPGD callback request with `len` capacity.
    let out = unsafe { core::slice::from_raw_parts_mut(buff, len) };
    match stream.read_into(out) {
        Ok(read_now) => read_now,
        Err(_) => {
            stream.io_failed = true;
            0
        }
    }
}

unsafe fn zip_jpeg_skip_dispatch<
    'a,
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    user: *mut c_void,
    len: usize,
) -> usize
where
    D: embedded_sdmmc::BlockDevice + 'a,
    T: TimeSource + 'a,
{
    if user.is_null() || len == 0 {
        return 0;
    }

    // SAFETY: `user` is installed from a live ZipJpegStream in decode scope.
    let stream =
        unsafe { &mut *(user as *mut ZipJpegStream<'a, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>) };
    if stream.io_failed {
        return 0;
    }

    match stream.skip_bytes(len) {
        Ok(skipped) => skipped,
        Err(_) => {
            stream.io_failed = true;
            0
        }
    }
}

fn decode_jpeg_thumbnail_stream<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    entry: ZipEntryRef,
    thumb_width: u16,
    thumb_height: u16,
    out_bits: &mut [u8],
) -> JpegThumbDecodeResult<D::Error>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let tw = thumb_width.max(1) as usize;
    let th = thumb_height.max(1) as usize;
    let dst_row_bytes = tw.div_ceil(8);
    let dst_bytes = dst_row_bytes.saturating_mul(th);
    if dst_bytes == 0 || dst_bytes > out_bits.len() {
        return Ok(None);
    }
    out_bits[..dst_bytes].fill(0);

    let Some((src_w, src_h, scanned_bytes)) = parse_jpeg_dimensions_stream(file, entry)? else {
        info!("sd: jpeg decode fail reason=missing_sof_dims");
        return Ok(None);
    };
    info!(
        "sd: jpeg dims source={}x{} scanned_bytes={}",
        src_w, src_h, scanned_bytes
    );

    let mut thumb_ctx = JpegThumbContext {
        out_bits: out_bits.as_mut_ptr(),
        out_len: dst_bytes,
        thumb_width: tw,
        thumb_height: th,
        source_width: src_w as usize,
        source_height: src_h as usize,
        max_right: 0,
        max_bottom: 0,
        pixels_written: 0,
    };

    let mut jpeg_stream = ZipJpegStream::new(file, entry);
    let io_dispatch = JpegIoDispatch {
        user: (&mut jpeg_stream as *mut _) as *mut c_void,
        read: zip_jpeg_read_dispatch::<D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
        skip: zip_jpeg_skip_dispatch::<D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    };

    let mut decoder_state = [0u32; JPEG_DECODER_BYTES / core::mem::size_of::<u32>()];
    let mut work = [0u8; JPEG_WORK_BYTES];

    // SAFETY: Callbacks are installed for this decode and guarded by drop cleanup.
    let Some(_guard) = (unsafe {
        JpegCallbackGuard::install(io_dispatch, &mut thumb_ctx as *mut JpegThumbContext)
    }) else {
        info!("sd: jpeg decode fail reason=callback_busy");
        return Ok(None);
    };

    // SAFETY: ROM TJPGD expects opaque state pointer + callbacks + work pool.
    let prep = unsafe {
        jd_prepare(
            decoder_state.as_mut_ptr() as *mut c_void,
            Some(jpeg_in_callback),
            work.as_mut_ptr() as *mut c_void,
            JPEG_WORK_BYTES as u32,
            core::ptr::null_mut(),
        )
    };
    if prep != JDR_OK {
        info!(
            "sd: jpeg decode fail reason=jd_prepare status={} kind={}",
            prep,
            jpeg_jdr_name(prep)
        );
        return Ok(None);
    }

    // Scale=0 keeps full source resolution before thumbnail mapping.
    // SAFETY: Decoder state is initialized by jd_prepare.
    let decomp = unsafe {
        jd_decomp(
            decoder_state.as_mut_ptr() as *mut c_void,
            Some(jpeg_out_callback),
            0,
        )
    };
    if decomp != JDR_OK {
        info!(
            "sd: jpeg decode fail reason=jd_decomp status={} kind={}",
            decomp,
            jpeg_jdr_name(decomp)
        );
        return Ok(None);
    }
    if jpeg_stream.io_failed {
        info!("sd: jpeg decode fail reason=stream_io");
        return Ok(None);
    }
    if thumb_ctx.pixels_written == 0 {
        info!("sd: jpeg decode fail reason=no_pixels");
        return Ok(None);
    }

    if thumb_ctx.max_right.saturating_add(1) < src_w as usize
        || thumb_ctx.max_bottom.saturating_add(1) < src_h as usize
    {
        info!(
            "sd: jpeg decode partial blocks src={}x{} seen={}x{}",
            src_w,
            src_h,
            thumb_ctx.max_right.saturating_add(1),
            thumb_ctx.max_bottom.saturating_add(1)
        );
    }

    Ok(Some((src_w, src_h, dst_bytes)))
}

fn parse_opf_metadata<const TITLE_BYTES: usize>(
    opf: &[u8],
    opf_path: &str,
    title_out: &mut String<TITLE_BYTES>,
) -> EpubMetadata {
    let mut metadata = EpubMetadata::default();

    let _ = parse_xml_tag_text(opf, b"<dc:title", title_out)
        || parse_xml_tag_text(opf, b"<title", title_out)
        || parse_opf_meta_content(opf, b"property", title_out)
        || parse_opf_meta_content(opf, b"name", title_out);

    let mut cover_path = String::<ZIP_PATH_BYTES>::new();
    let mut cover_media = String::<ZIP_MEDIA_BYTES>::new();
    metadata.has_cover = parse_opf_cover_resource(opf, opf_path, &mut cover_path, &mut cover_media)
        || find_ascii_case_insensitive(opf, b"cover-image", 0).is_some()
        || find_ascii_case_insensitive(opf, b"name=\"cover\"", 0).is_some()
        || find_ascii_case_insensitive(opf, b"name='cover'", 0).is_some();

    metadata
}

fn read_file_at<D, T, const MAX_DIRS: usize, const MAX_FILES: usize, const MAX_VOLUMES: usize>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    offset: u32,
    out: &mut [u8],
) -> Result<usize, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    file.seek_from_start(offset)?;
    let mut total = 0usize;
    while total < out.len() {
        let read_now = file.read(&mut out[total..])?;
        if read_now == 0 {
            break;
        }
        total = total.saturating_add(read_now);
    }
    Ok(total)
}

fn entry_data_offset<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    entry: ZipEntryRef,
) -> Result<Option<u32>, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let mut local = [0u8; ZIP_LOCAL_HEADER_BYTES];
    let local_read = read_file_at(file, entry.local_header_offset, &mut local)?;
    if local_read < ZIP_LOCAL_HEADER_BYTES || !local.starts_with(&ZIP_LOCAL_SIG) {
        return Ok(None);
    }

    let name_len = read_u16_le(&local, 26) as u32;
    let extra_len = read_u16_le(&local, 28) as u32;
    let data_offset = entry
        .local_header_offset
        .checked_add(ZIP_LOCAL_HEADER_BYTES as u32)
        .and_then(|value| value.checked_add(name_len))
        .and_then(|value| value.checked_add(extra_len));

    Ok(data_offset)
}

fn read_stored_entry_chunk<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    entry: ZipEntryRef,
    start_offset: u32,
    out: &mut [u8],
) -> Result<(usize, bool), embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    if out.is_empty() || entry.uncompressed_size == 0 {
        return Ok((0, true));
    }

    if start_offset >= entry.uncompressed_size {
        return Ok((0, true));
    }

    let Some(data_offset) = entry_data_offset(file, entry)? else {
        return Ok((0, true));
    };

    let Some(read_start) = data_offset.checked_add(start_offset) else {
        return Ok((0, true));
    };
    let remaining = (entry.uncompressed_size - start_offset) as usize;
    let read_len = remaining.min(out.len());
    let read_now = read_file_at(file, read_start, &mut out[..read_len])?;
    let end_of_resource = start_offset.saturating_add(read_now as u32) >= entry.uncompressed_size;
    Ok((read_now, end_of_resource))
}

fn read_deflated_entry_prefix_fast<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    entry: ZipEntryRef,
    out: &mut [u8],
) -> Result<usize, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    if out.is_empty() || entry.uncompressed_size == 0 || entry.compressed_size == 0 {
        return Ok(0);
    }

    let Some(data_offset) = entry_data_offset(file, entry)? else {
        return Ok(0);
    };

    file.seek_from_start(data_offset)?;

    let mut inflater = DecompressorOxide::new();
    let mut input = [0u8; ZIP_DEFLATE_INPUT_BYTES];
    let mut input_len = 0usize;
    let mut input_pos = 0usize;
    let mut compressed_remaining = entry.compressed_size;
    let mut out_pos = 0usize;

    loop {
        if input_pos >= input_len && compressed_remaining > 0 {
            let read_len = input.len().min(compressed_remaining as usize);
            let read_now = file.read(&mut input[..read_len])?;
            if read_now == 0 {
                return Ok(out_pos);
            }
            input_len = read_now;
            input_pos = 0;
            compressed_remaining = compressed_remaining.saturating_sub(read_now as u32);
        }

        let has_more_input = input_pos < input_len || compressed_remaining > 0;
        let mut flags = inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF;
        if has_more_input {
            flags |= inflate_flags::TINFL_FLAG_HAS_MORE_INPUT;
        }

        let (status, consumed, written) = decompress(
            &mut inflater,
            &input[input_pos..input_len],
            out,
            out_pos,
            flags,
        );
        input_pos = input_pos.saturating_add(consumed);
        out_pos = out_pos.saturating_add(written);

        match status {
            TINFLStatus::Done | TINFLStatus::HasMoreOutput => return Ok(out_pos),
            TINFLStatus::NeedsMoreInput => {
                if !has_more_input {
                    return Ok(out_pos);
                }
            }
            _ => return Ok(0),
        }

        if out_pos >= out.len() {
            return Ok(out_pos);
        }
    }
}

fn read_deflated_entry_chunk<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    entry: ZipEntryRef,
    start_offset: u32,
    out: &mut [u8],
) -> Result<(usize, bool), embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    if out.is_empty() || entry.uncompressed_size == 0 || entry.compressed_size == 0 {
        return Ok((0, true));
    }

    if start_offset == 0 {
        let bytes = read_deflated_entry_prefix_fast(file, entry, out)?;
        let end_of_resource = bytes >= entry.uncompressed_size as usize;
        return Ok((bytes, end_of_resource));
    }

    if start_offset >= entry.uncompressed_size {
        return Ok((0, true));
    }

    let Some(data_offset) = entry_data_offset(file, entry)? else {
        return Ok((0, true));
    };

    file.seek_from_start(data_offset)?;

    let mut inflater = InflateState::new(DataFormat::Raw);
    let mut input = [0u8; ZIP_DEFLATE_INPUT_BYTES];
    let mut output = [0u8; ZIP_DEFLATE_OUTPUT_BYTES];
    let mut input_len = 0usize;
    let mut input_pos = 0usize;
    let mut compressed_remaining = entry.compressed_size;
    let mut uncompressed_total = 0u32;
    let mut written_total = 0usize;
    let mut no_progress_loops = 0u8;
    let mut stream_done = false;

    loop {
        if input_pos >= input_len && compressed_remaining > 0 {
            let read_len = input.len().min(compressed_remaining as usize);
            let read_now = file.read(&mut input[..read_len])?;
            if read_now == 0 {
                break;
            }
            input_len = read_now;
            input_pos = 0;
            compressed_remaining = compressed_remaining.saturating_sub(read_now as u32);
        }

        let compressed_input = &input[input_pos..input_len];
        let flush = if compressed_remaining == 0 && compressed_input.is_empty() {
            MZFlush::Finish
        } else {
            MZFlush::None
        };
        let stream = inflate(&mut inflater, compressed_input, &mut output, flush);
        input_pos = input_pos.saturating_add(stream.bytes_consumed);

        if stream.bytes_consumed == 0 && stream.bytes_written == 0 {
            no_progress_loops = no_progress_loops.saturating_add(1);
            if no_progress_loops >= 2 {
                break;
            }
        } else {
            no_progress_loops = 0;
        }

        if stream.bytes_written > 0 {
            let chunk_start = uncompressed_total;
            let chunk_end = chunk_start.saturating_add(stream.bytes_written as u32);

            if chunk_end > start_offset && written_total < out.len() {
                let copy_start = start_offset.saturating_sub(chunk_start) as usize;
                let available = stream.bytes_written.saturating_sub(copy_start);
                let copy_len = available.min(out.len().saturating_sub(written_total));
                if copy_len > 0 {
                    out[written_total..written_total + copy_len]
                        .copy_from_slice(&output[copy_start..copy_start + copy_len]);
                    written_total = written_total.saturating_add(copy_len);
                }
            }

            uncompressed_total = chunk_end;
        }

        match stream.status {
            Ok(MZStatus::StreamEnd) => {
                stream_done = true;
                break;
            }
            Ok(MZStatus::Ok) => {}
            Err(MZError::Buf) => {
                if compressed_remaining == 0 && input_pos >= input_len {
                    break;
                }
            }
            _ => return Ok((0, false)),
        }

        if written_total >= out.len() {
            return Ok((written_total, false));
        }

        if compressed_remaining == 0 && input_pos >= input_len && stream.bytes_written == 0 {
            break;
        }
    }

    let end = stream_done
        || uncompressed_total >= entry.uncompressed_size
        || (compressed_remaining == 0 && input_pos >= input_len);
    if written_total == 0 && !end {
        return Ok((0, false));
    }
    Ok((written_total, end))
}

fn read_zip_entry_chunk<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    entry: ZipEntryRef,
    start_offset: u32,
    out: &mut [u8],
) -> Result<(usize, bool), embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    match entry.compression {
        0 => read_stored_entry_chunk(file, entry, start_offset, out),
        8 => read_deflated_entry_chunk(file, entry, start_offset, out),
        _ => Ok((0, false)),
    }
}

fn read_zip_entry_prefix<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    entry: ZipEntryRef,
    out: &mut [u8],
) -> Result<usize, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let (read_now, _) = read_zip_entry_chunk(file, entry, 0, out)?;
    Ok(read_now)
}

fn scan_epub_metadata<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
    const TITLE_BYTES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    file_size: u32,
    title_out: &mut String<TITLE_BYTES>,
) -> Result<EpubMetadata, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let Some((opf_entry, opf_path)) =
        find_opf_entry_and_path::<_, _, _, _, _, ZIP_PATH_BYTES>(file, file_size)?
    else {
        return Ok(EpubMetadata::default());
    };

    let mut opf_buf = [0u8; ZIP_OPF_BYTES];
    let opf_read = read_zip_entry_prefix(file, opf_entry, &mut opf_buf)?;
    if opf_read == 0 {
        return Ok(EpubMetadata::default());
    }

    Ok(parse_opf_metadata(
        &opf_buf[..opf_read],
        opf_path.as_str(),
        title_out,
    ))
}

fn find_first_text_entry<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
    const PATH_BYTES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    file_size: u32,
) -> Result<Option<(ZipEntryRef, String<PATH_BYTES>)>, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let mut opf_buf = [0u8; ZIP_OPF_BYTES];
    let mut spine_candidate: Option<(ZipEntryRef, String<PATH_BYTES>)> = None;
    if let Some((opf_entry, opf_path)) =
        find_opf_entry_and_path::<_, _, _, _, _, PATH_BYTES>(file, file_size)?
    {
        let opf_read = read_zip_entry_prefix(file, opf_entry, &mut opf_buf)?;
        if opf_read > 0 {
            let mut spine_path = String::<PATH_BYTES>::new();
            if parse_spine_first_text_href(&opf_buf[..opf_read], opf_path.as_str(), &mut spine_path)
                && let Some(entry) = find_entry_by_path(file, file_size, spine_path.as_bytes())?
            {
                if entry.uncompressed_size >= ZIP_MIN_PRIMARY_TEXT_BYTES {
                    return Ok(Some((entry, spine_path)));
                }
                spine_candidate = Some((entry, spine_path));
            }
        }
    }

    let Some((cdir_offset, cdir_entries)) = cdir_info(file, file_size)? else {
        return Ok(None);
    };

    let mut cdir_header = [0u8; ZIP_CDIR_HEADER_BYTES];
    let mut cdir_name = [0u8; ZIP_NAME_BYTES];
    let mut cdir_cursor = cdir_offset;
    let mut fallback_non_front_matter: Option<(ZipEntryRef, String<PATH_BYTES>)> = None;
    let mut preferred_non_front_matter: Option<(ZipEntryRef, String<PATH_BYTES>)> = None;
    let mut fallback_front_matter: Option<(ZipEntryRef, String<PATH_BYTES>)> = None;

    for _ in 0..cdir_entries.min(ZIP_MAX_CDIR_ENTRIES) {
        let header_read = read_file_at(file, cdir_cursor, &mut cdir_header)?;
        if header_read < ZIP_CDIR_HEADER_BYTES || !cdir_header.starts_with(&ZIP_CDIR_SIG) {
            break;
        }

        let compression = read_u16_le(&cdir_header, 10);
        let compressed_size = read_u32_le(&cdir_header, 20);
        let uncompressed_size = read_u32_le(&cdir_header, 24);
        let name_len = read_u16_le(&cdir_header, 28) as usize;
        let extra_len = read_u16_le(&cdir_header, 30) as usize;
        let comment_len = read_u16_le(&cdir_header, 32) as usize;
        let local_header_offset = read_u32_le(&cdir_header, 42);

        let Some(next_cursor) = cdir_cursor
            .checked_add(ZIP_CDIR_HEADER_BYTES as u32)
            .and_then(|value| value.checked_add(name_len as u32))
            .and_then(|value| value.checked_add(extra_len as u32))
            .and_then(|value| value.checked_add(comment_len as u32))
        else {
            break;
        };

        let name_read_len = name_len.min(cdir_name.len());
        if name_read_len == 0 {
            cdir_cursor = next_cursor;
            continue;
        }

        let name_read = read_file_at(
            file,
            cdir_cursor.saturating_add(ZIP_CDIR_HEADER_BYTES as u32),
            &mut cdir_name[..name_read_len],
        )?;
        if name_read < name_read_len {
            break;
        }

        let name_complete = name_len <= cdir_name.len();
        let name_slice = &cdir_name[..name_read_len];
        if name_complete && is_text_resource_name(name_slice) {
            let entry_ref = ZipEntryRef {
                compression,
                compressed_size,
                uncompressed_size,
                local_header_offset,
            };
            let mut resource = String::<PATH_BYTES>::new();
            copy_ascii_or_lossy(name_slice, &mut resource);
            if path_is_probably_front_matter(name_slice) {
                if fallback_front_matter.is_none() {
                    fallback_front_matter = Some((entry_ref, resource));
                }
            } else {
                if fallback_non_front_matter.is_none() {
                    fallback_non_front_matter = Some((entry_ref, resource.clone()));
                }
                if preferred_non_front_matter.is_none()
                    && entry_ref.uncompressed_size >= ZIP_MIN_PRIMARY_TEXT_BYTES
                {
                    preferred_non_front_matter = Some((entry_ref, resource));
                }
            }
        }

        cdir_cursor = next_cursor;
    }

    if let Some(candidate) = preferred_non_front_matter {
        return Ok(Some(candidate));
    }
    if let Some(candidate) = spine_candidate {
        return Ok(Some(candidate));
    }
    if let Some(candidate) = fallback_non_front_matter {
        return Ok(Some(candidate));
    }
    if let Some(candidate) = fallback_front_matter {
        return Ok(Some(candidate));
    }

    Ok(None)
}

fn find_next_text_entry<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
    const PATH_BYTES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    file_size: u32,
    current_path: &str,
) -> Result<Option<(ZipEntryRef, String<PATH_BYTES>)>, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    if current_path.is_empty() {
        return find_first_text_entry::<_, _, _, _, _, PATH_BYTES>(file, file_size);
    }

    let mut opf_buf = [0u8; ZIP_OPF_BYTES];
    if let Some((opf_entry, opf_path)) =
        find_opf_entry_and_path::<_, _, _, _, _, PATH_BYTES>(file, file_size)?
    {
        let opf_read = read_zip_entry_prefix(file, opf_entry, &mut opf_buf)?;
        if opf_read > 0 {
            let mut next_path = String::<PATH_BYTES>::new();
            if parse_spine_next_text_href(
                &opf_buf[..opf_read],
                opf_path.as_str(),
                current_path,
                &mut next_path,
            ) && let Some(entry) = find_entry_by_path(file, file_size, next_path.as_bytes())?
            {
                return Ok(Some((entry, next_path)));
            }
        }
    }

    let Some((cdir_offset, cdir_entries)) = cdir_info(file, file_size)? else {
        return Ok(None);
    };

    let mut cdir_header = [0u8; ZIP_CDIR_HEADER_BYTES];
    let mut cdir_name = [0u8; ZIP_NAME_BYTES];
    let mut cdir_cursor = cdir_offset;
    let mut seen_current = false;
    let mut fallback_any_text: Option<(ZipEntryRef, String<PATH_BYTES>)> = None;
    let mut fallback_non_front_matter: Option<(ZipEntryRef, String<PATH_BYTES>)> = None;
    let mut preferred_non_front_matter: Option<(ZipEntryRef, String<PATH_BYTES>)> = None;

    for _ in 0..cdir_entries.min(ZIP_MAX_CDIR_ENTRIES) {
        let header_read = read_file_at(file, cdir_cursor, &mut cdir_header)?;
        if header_read < ZIP_CDIR_HEADER_BYTES || !cdir_header.starts_with(&ZIP_CDIR_SIG) {
            break;
        }

        let compression = read_u16_le(&cdir_header, 10);
        let compressed_size = read_u32_le(&cdir_header, 20);
        let uncompressed_size = read_u32_le(&cdir_header, 24);
        let name_len = read_u16_le(&cdir_header, 28) as usize;
        let extra_len = read_u16_le(&cdir_header, 30) as usize;
        let comment_len = read_u16_le(&cdir_header, 32) as usize;
        let local_header_offset = read_u32_le(&cdir_header, 42);

        let Some(next_cursor) = cdir_cursor
            .checked_add(ZIP_CDIR_HEADER_BYTES as u32)
            .and_then(|value| value.checked_add(name_len as u32))
            .and_then(|value| value.checked_add(extra_len as u32))
            .and_then(|value| value.checked_add(comment_len as u32))
        else {
            break;
        };

        let name_read_len = name_len.min(cdir_name.len());
        if name_read_len == 0 {
            cdir_cursor = next_cursor;
            continue;
        }

        let name_read = read_file_at(
            file,
            cdir_cursor.saturating_add(ZIP_CDIR_HEADER_BYTES as u32),
            &mut cdir_name[..name_read_len],
        )?;
        if name_read < name_read_len {
            break;
        }

        let name_complete = name_len <= cdir_name.len();
        let name_slice = &cdir_name[..name_read_len];
        if !seen_current {
            if name_complete && eq_ascii_case_insensitive(name_slice, current_path.as_bytes()) {
                seen_current = true;
            }
            cdir_cursor = next_cursor;
            continue;
        }

        if name_complete && is_text_resource_name(name_slice) {
            let entry_ref = ZipEntryRef {
                compression,
                compressed_size,
                uncompressed_size,
                local_header_offset,
            };
            let mut resource = String::<PATH_BYTES>::new();
            copy_ascii_or_lossy(name_slice, &mut resource);

            if fallback_any_text.is_none() {
                fallback_any_text = Some((entry_ref, resource.clone()));
            }

            if !path_is_probably_front_matter(name_slice) {
                if fallback_non_front_matter.is_none() {
                    fallback_non_front_matter = Some((entry_ref, resource.clone()));
                }
                if preferred_non_front_matter.is_none()
                    && entry_ref.uncompressed_size >= ZIP_MIN_PRIMARY_TEXT_BYTES
                {
                    preferred_non_front_matter = Some((entry_ref, resource));
                }
            }
        }

        cdir_cursor = next_cursor;
    }

    if let Some(candidate) = preferred_non_front_matter {
        return Ok(Some(candidate));
    }
    if let Some(candidate) = fallback_non_front_matter {
        return Ok(Some(candidate));
    }
    if let Some(candidate) = fallback_any_text {
        return Ok(Some(candidate));
    }

    Ok(None)
}

fn find_text_entry_by_chapter_index<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
    const PATH_BYTES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    file_size: u32,
    target_chapter: u16,
) -> Result<Option<ChapterEntry<PATH_BYTES>>, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let mut opf_buf = [0u8; ZIP_OPF_BYTES];
    if let Some((opf_entry, opf_path)) =
        find_opf_entry_and_path::<_, _, _, _, _, PATH_BYTES>(file, file_size)?
    {
        let opf_read = read_zip_entry_prefix(file, opf_entry, &mut opf_buf)?;
        if opf_read > 0 {
            let mut chapter_path = String::<PATH_BYTES>::new();
            let mut chapter_index = 0u16;
            let mut chapter_total = 1u16;
            if parse_spine_text_href_at(
                &opf_buf[..opf_read],
                opf_path.as_str(),
                target_chapter,
                &mut chapter_path,
                &mut chapter_index,
                &mut chapter_total,
            ) && let Some(entry) = find_entry_by_path(file, file_size, chapter_path.as_bytes())?
            {
                return Ok(Some((entry, chapter_path, chapter_index, chapter_total)));
            }
        }
    }

    cdir_text_entry_at::<_, _, _, _, _, PATH_BYTES>(file, file_size, target_chapter)
}

/// Probe card + mount FAT + read a first text chunk from one EPUB file.
///
/// `short_name` must be an 8.3 short filename (for example `BOOK~1.EPU`).
pub fn probe_and_read_epub_text_chunk<BUS, CS, DELAY, const PATH_BYTES: usize>(
    bus: &mut BUS,
    cs: &mut CS,
    delay: &mut DELAY,
    books_dir_name: &str,
    short_name: &str,
    out: &mut [u8],
) -> Result<SdEpubTextChunkResult<PATH_BYTES>, SdProbeError<BUS::Error, CS::Error>>
where
    BUS: SpiBus<u8>,
    CS: OutputPin,
    DELAY: DelayNs,
    BUS::Error: core::fmt::Debug,
    CS::Error: core::fmt::Debug,
{
    probe_and_read_epub_text_chunk_from_resource::<_, _, _, PATH_BYTES>(
        bus,
        cs,
        delay,
        books_dir_name,
        short_name,
        "",
        0,
        out,
    )
}

/// Probe card + mount FAT + read a text chunk from a chapter index.
///
/// `target_chapter` is zero-based. When the requested chapter is out of range,
/// the reader falls back to the last available chapter.
pub fn probe_and_read_epub_text_chunk_at_chapter<BUS, CS, DELAY, const PATH_BYTES: usize>(
    bus: &mut BUS,
    cs: &mut CS,
    delay: &mut DELAY,
    books_dir_name: &str,
    short_name: &str,
    target_chapter: u16,
    out: &mut [u8],
) -> Result<SdEpubTextChunkResult<PATH_BYTES>, SdProbeError<BUS::Error, CS::Error>>
where
    BUS: SpiBus<u8>,
    CS: OutputPin,
    DELAY: DelayNs,
    BUS::Error: core::fmt::Debug,
    CS::Error: core::fmt::Debug,
{
    cs.set_high().map_err(SdProbeError::ChipSelect)?;

    // SD SPI init requires >=74 clock cycles with CS deasserted.
    let preclock = [0xFFu8; 10];
    bus.write(&preclock).map_err(SdProbeError::Spi)?;

    let spi_device = ExclusiveSpiDevice::new(bus, cs);
    let mut delay_ref = DelayRef(delay);
    let sd_card = SdCard::new(spi_device, &mut delay_ref);
    let card_size_bytes = sd_card.num_bytes().map_err(SdProbeError::Card)?;

    let mut result = SdEpubTextChunkResult {
        card_size_bytes,
        text_resource: String::new(),
        chapter_index: 0,
        chapter_total: 1,
        compression: 0,
        bytes_read: 0,
        end_of_resource: false,
        status: SdEpubTextChunkStatus::NoTextResource,
    };

    let mut volume_mgr = VolumeManager::new(sd_card, FixedTimeSource);
    let mut volume = volume_mgr
        .open_volume(VolumeIdx(0))
        .map_err(SdProbeError::Filesystem)?;
    let mut root_dir = volume.open_root_dir().map_err(SdProbeError::Filesystem)?;

    let mut books_dir = match root_dir.open_dir(books_dir_name) {
        Ok(dir) => dir,
        Err(embedded_sdmmc::Error::NotFound) => return Ok(result),
        Err(err) => return Err(SdProbeError::Filesystem(err)),
    };

    let short_name = match ShortFileName::create_from_str(short_name) {
        Ok(name) => name,
        Err(_) => return Ok(result),
    };

    let mut file = match books_dir.open_file_in_dir(&short_name, Mode::ReadOnly) {
        Ok(file) => file,
        Err(embedded_sdmmc::Error::NotFound) => return Ok(result),
        Err(err) => return Err(SdProbeError::Filesystem(err)),
    };

    let mut header = [0u8; 4];
    let read_now = file.read(&mut header).map_err(SdProbeError::Filesystem)?;
    if !has_zip_signature(header, read_now) {
        result.status = SdEpubTextChunkStatus::NotZip;
        file.close().map_err(SdProbeError::Filesystem)?;
        books_dir.close().map_err(SdProbeError::Filesystem)?;
        root_dir.close().map_err(SdProbeError::Filesystem)?;
        volume.close().map_err(SdProbeError::Filesystem)?;
        return Ok(result);
    }

    let file_size = file.length();
    if let Some((entry, resource, chapter_index, chapter_total)) =
        find_text_entry_by_chapter_index::<_, _, _, _, _, PATH_BYTES>(
            &mut file,
            file_size,
            target_chapter,
        )
        .map_err(SdProbeError::Filesystem)?
    {
        result.compression = entry.compression;
        result.text_resource = resource;
        result.chapter_index = chapter_index;
        result.chapter_total = chapter_total.max(1);

        if matches!(entry.compression, 0 | 8) {
            let (bytes_read, end_of_resource) =
                read_zip_entry_chunk(&mut file, entry, 0, out).map_err(SdProbeError::Filesystem)?;
            result.bytes_read = bytes_read;
            result.end_of_resource = end_of_resource;
            result.status = if bytes_read > 0 || end_of_resource {
                SdEpubTextChunkStatus::ReadOk
            } else {
                SdEpubTextChunkStatus::DecodeFailed
            };
        } else {
            result.status = SdEpubTextChunkStatus::UnsupportedCompression;
        }
    }

    file.close().map_err(SdProbeError::Filesystem)?;
    books_dir.close().map_err(SdProbeError::Filesystem)?;
    root_dir.close().map_err(SdProbeError::Filesystem)?;
    volume.close().map_err(SdProbeError::Filesystem)?;

    Ok(result)
}

fn decode_cover_entry_thumbnail<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    entry: ZipEntryRef,
    media: &[u8],
    path: &[u8],
    thumb_width: u16,
    thumb_height: u16,
    out: &mut [u8],
) -> Result<(SdEpubCoverStatus, u16, u16, usize), embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let mut media_is_pbm = is_cover_media_pbm(media, path);
    let mut media_is_png = is_cover_media_png(media, path);
    let mut media_is_jpeg = is_cover_media_jpeg(media, path);
    if !media_is_pbm && !media_is_png && !media_is_jpeg {
        let mut sniff = [0u8; 16];
        let sniff_read = read_zip_entry_prefix(file, entry, &mut sniff)?;
        if sniff_read > 0 {
            let (pbm_guess, png_guess, jpeg_guess) = sniff_cover_media_flags(&sniff[..sniff_read]);
            media_is_pbm = pbm_guess;
            media_is_png = png_guess;
            media_is_jpeg = jpeg_guess;
        }
        if !media_is_pbm && !media_is_png && !media_is_jpeg {
            return Ok((SdEpubCoverStatus::UnsupportedMediaType, 0, 0, 0));
        }
    }

    if media_is_pbm {
        let mut cover_bytes = [0u8; ZIP_COVER_BYTES];
        let read_cover = read_zip_entry_prefix(file, entry, &mut cover_bytes)?;
        if read_cover > 0
            && let Some((source_width, source_height, bytes_written)) =
                decode_pbm_thumbnail_p4(&cover_bytes[..read_cover], thumb_width, thumb_height, out)
        {
            return Ok((
                SdEpubCoverStatus::ReadOk,
                source_width,
                source_height,
                bytes_written,
            ));
        }
        return Ok((SdEpubCoverStatus::DecodeFailed, 0, 0, 0));
    }

    if media_is_png {
        if let Some((source_width, source_height, bytes_written)) =
            decode_png_thumbnail_stream(file, entry, thumb_width, thumb_height, out)?
        {
            return Ok((
                SdEpubCoverStatus::ReadOk,
                source_width,
                source_height,
                bytes_written,
            ));
        }
        return Ok((SdEpubCoverStatus::DecodeFailed, 0, 0, 0));
    }

    if let Some((source_width, source_height, bytes_written)) =
        decode_jpeg_thumbnail_stream(file, entry, thumb_width, thumb_height, out)?
    {
        return Ok((
            SdEpubCoverStatus::ReadOk,
            source_width,
            source_height,
            bytes_written,
        ));
    }
    Ok((SdEpubCoverStatus::DecodeFailed, 0, 0, 0))
}

/// Probe card + mount FAT + decode EPUB cover into a 1bpp thumbnail buffer.
///
/// Current decoder support:
/// - `image/x-portable-bitmap` / `.pbm` (binary `P4`)
/// - `image/png` / `.png` (non-interlaced)
/// - `image/jpeg` / `.jpg` / `.jpeg` (ESP32-S3 ROM TJPGD path)
///
/// When cover metadata points to an XHTML/HTML resource, the first `<img src>`
/// is followed before decoding.
#[allow(
    clippy::too_many_arguments,
    reason = "embedded call-site clarity; explicit bus/cs/delay/book/thumb params"
)]
pub fn probe_and_read_epub_cover_thumbnail<
    BUS,
    CS,
    DELAY,
    const PATH_BYTES: usize,
    const MEDIA_BYTES: usize,
>(
    bus: &mut BUS,
    cs: &mut CS,
    delay: &mut DELAY,
    books_dir_name: &str,
    short_name: &str,
    thumb_width: u16,
    thumb_height: u16,
    out: &mut [u8],
) -> Result<SdEpubCoverResult<PATH_BYTES, MEDIA_BYTES>, SdProbeError<BUS::Error, CS::Error>>
where
    BUS: SpiBus<u8>,
    CS: OutputPin,
    DELAY: DelayNs,
    BUS::Error: core::fmt::Debug,
    CS::Error: core::fmt::Debug,
{
    cs.set_high().map_err(SdProbeError::ChipSelect)?;

    // SD SPI init requires >=74 clock cycles with CS deasserted.
    let preclock = [0xFFu8; 10];
    bus.write(&preclock).map_err(SdProbeError::Spi)?;

    let spi_device = ExclusiveSpiDevice::new(bus, cs);
    let mut delay_ref = DelayRef(delay);
    let sd_card = SdCard::new(spi_device, &mut delay_ref);
    let card_size_bytes = sd_card.num_bytes().map_err(SdProbeError::Card)?;

    let mut result = SdEpubCoverResult {
        card_size_bytes,
        cover_resource: String::new(),
        media_type: String::new(),
        source_width: 0,
        source_height: 0,
        thumb_width: thumb_width.max(1),
        thumb_height: thumb_height.max(1),
        bytes_written: 0,
        status: SdEpubCoverStatus::NoCoverResource,
    };

    let mut volume_mgr = VolumeManager::new(sd_card, FixedTimeSource);
    let mut volume = volume_mgr
        .open_volume(VolumeIdx(0))
        .map_err(SdProbeError::Filesystem)?;
    let mut root_dir = volume.open_root_dir().map_err(SdProbeError::Filesystem)?;

    let mut books_dir = match root_dir.open_dir(books_dir_name) {
        Ok(dir) => dir,
        Err(embedded_sdmmc::Error::NotFound) => return Ok(result),
        Err(err) => return Err(SdProbeError::Filesystem(err)),
    };

    let short_name = match ShortFileName::create_from_str(short_name) {
        Ok(name) => name,
        Err(_) => return Ok(result),
    };

    let mut file = match books_dir.open_file_in_dir(&short_name, Mode::ReadOnly) {
        Ok(file) => file,
        Err(embedded_sdmmc::Error::NotFound) => return Ok(result),
        Err(err) => return Err(SdProbeError::Filesystem(err)),
    };

    let mut header = [0u8; 4];
    let read_now = file.read(&mut header).map_err(SdProbeError::Filesystem)?;
    if !has_zip_signature(header, read_now) {
        result.status = SdEpubCoverStatus::NotZip;
        file.close().map_err(SdProbeError::Filesystem)?;
        books_dir.close().map_err(SdProbeError::Filesystem)?;
        root_dir.close().map_err(SdProbeError::Filesystem)?;
        volume.close().map_err(SdProbeError::Filesystem)?;
        return Ok(result);
    }

    let file_size = file.length();
    let mut opf_buf = [0u8; ZIP_OPF_BYTES];
    let mut cover_path = String::<PATH_BYTES>::new();
    let mut cover_media = String::<MEDIA_BYTES>::new();
    let Some((opf_entry, opf_path)) =
        find_opf_entry_and_path::<_, _, _, _, _, PATH_BYTES>(&mut file, file_size)
            .map_err(SdProbeError::Filesystem)?
    else {
        file.close().map_err(SdProbeError::Filesystem)?;
        books_dir.close().map_err(SdProbeError::Filesystem)?;
        root_dir.close().map_err(SdProbeError::Filesystem)?;
        volume.close().map_err(SdProbeError::Filesystem)?;
        return Ok(result);
    };
    let opf_read = read_zip_entry_prefix(&mut file, opf_entry, &mut opf_buf)
        .map_err(SdProbeError::Filesystem)?;
    if opf_read == 0 {
        file.close().map_err(SdProbeError::Filesystem)?;
        books_dir.close().map_err(SdProbeError::Filesystem)?;
        root_dir.close().map_err(SdProbeError::Filesystem)?;
        volume.close().map_err(SdProbeError::Filesystem)?;
        return Ok(result);
    }

    let mut parsed_cover = parse_opf_cover_resource(
        &opf_buf[..opf_read],
        opf_path.as_str(),
        &mut cover_path,
        &mut cover_media,
    );

    let mut cover_entry = if parsed_cover {
        find_entry_by_path(&mut file, file_size, cover_path.as_bytes())
            .map_err(SdProbeError::Filesystem)?
    } else {
        None
    };
    if cover_entry.is_none()
        && let Some((fallback_entry, fallback_path, fallback_media)) =
            find_fallback_cover_image_entry::<_, _, _, _, _, PATH_BYTES, MEDIA_BYTES>(
                &mut file, file_size, None,
            )
            .map_err(SdProbeError::Filesystem)?
    {
        parsed_cover = true;
        cover_entry = Some(fallback_entry);
        cover_path = fallback_path;
        cover_media = fallback_media;
    }

    let Some(mut cover_entry) = cover_entry else {
        if parsed_cover {
            result.cover_resource = cover_path;
            result.media_type = cover_media;
            result.status = SdEpubCoverStatus::DecodeFailed;
        }
        file.close().map_err(SdProbeError::Filesystem)?;
        books_dir.close().map_err(SdProbeError::Filesystem)?;
        root_dir.close().map_err(SdProbeError::Filesystem)?;
        volume.close().map_err(SdProbeError::Filesystem)?;
        return Ok(result);
    };

    if is_text_media_document(cover_media.as_bytes(), cover_path.as_bytes()) {
        let mut cover_doc = [0u8; ZIP_CONTAINER_BYTES];
        let read_cover_doc = read_zip_entry_prefix(&mut file, cover_entry, &mut cover_doc)
            .map_err(SdProbeError::Filesystem)?;
        let mut nested_cover_path = String::<PATH_BYTES>::new();
        if read_cover_doc > 0
            && parse_html_first_img_src(
                &cover_doc[..read_cover_doc],
                cover_path.as_str(),
                &mut nested_cover_path,
            )
            && let Some(nested_entry) =
                find_entry_by_path(&mut file, file_size, nested_cover_path.as_bytes())
                    .map_err(SdProbeError::Filesystem)?
        {
            cover_entry = nested_entry;
            cover_path = nested_cover_path;
            copy_media_type_or_default(None, cover_path.as_bytes(), &mut cover_media);
        }
    }

    result.cover_resource = cover_path;
    result.media_type = cover_media;

    let (status, source_width, source_height, bytes_written) = decode_cover_entry_thumbnail(
        &mut file,
        cover_entry,
        result.media_type.as_bytes(),
        result.cover_resource.as_bytes(),
        result.thumb_width,
        result.thumb_height,
        out,
    )
    .map_err(SdProbeError::Filesystem)?;
    result.status = status;
    if status == SdEpubCoverStatus::ReadOk {
        result.source_width = source_width;
        result.source_height = source_height;
        result.bytes_written = bytes_written;
    } else {
        let mut fallback_attempts = 0usize;
        for skip_count in 0..16usize {
            let Some((fallback_entry, fallback_path, fallback_media)) =
                find_fallback_cover_image_entry_with_skip::<
                    _,
                    _,
                    _,
                    _,
                    _,
                    PATH_BYTES,
                    MEDIA_BYTES,
                >(
                    &mut file,
                    file_size,
                    Some(result.cover_resource.as_bytes()),
                    skip_count,
                )
                .map_err(SdProbeError::Filesystem)?
            else {
                break;
            };

            fallback_attempts = fallback_attempts.saturating_add(1);
            info!(
                "sd: cover fallback candidate path={} media={} order={}",
                fallback_path.as_str(),
                fallback_media.as_str(),
                skip_count.saturating_add(1)
            );
            let (fallback_status, fallback_sw, fallback_sh, fallback_bytes) =
                decode_cover_entry_thumbnail(
                    &mut file,
                    fallback_entry,
                    fallback_media.as_bytes(),
                    fallback_path.as_bytes(),
                    result.thumb_width,
                    result.thumb_height,
                    out,
                )
                .map_err(SdProbeError::Filesystem)?;
            if fallback_status == SdEpubCoverStatus::ReadOk {
                result.cover_resource = fallback_path;
                result.media_type = fallback_media;
                result.status = fallback_status;
                result.source_width = fallback_sw;
                result.source_height = fallback_sh;
                result.bytes_written = fallback_bytes;
                info!(
                    "sd: cover fallback applied resource={} media={} source={}x{} bytes={}",
                    result.cover_resource.as_str(),
                    result.media_type.as_str(),
                    result.source_width,
                    result.source_height,
                    result.bytes_written
                );
                break;
            }

            info!(
                "sd: cover fallback decode_failed path={} media={} status={:?}",
                fallback_path.as_str(),
                fallback_media.as_str(),
                fallback_status
            );
        }

        if result.status != SdEpubCoverStatus::ReadOk {
            if fallback_attempts == 0 {
                info!(
                    "sd: cover fallback none base_resource={} base_media={}",
                    result.cover_resource.as_str(),
                    result.media_type.as_str()
                );
            } else {
                info!(
                    "sd: cover fallback exhausted base_resource={} attempts={}",
                    result.cover_resource.as_str(),
                    fallback_attempts
                );
            }
        }
    }

    file.close().map_err(SdProbeError::Filesystem)?;
    books_dir.close().map_err(SdProbeError::Filesystem)?;
    root_dir.close().map_err(SdProbeError::Filesystem)?;
    volume.close().map_err(SdProbeError::Filesystem)?;

    Ok(result)
}

/// Probe card + mount FAT + read a text chunk from a specific EPUB resource.
///
/// When `text_resource_hint` is empty, the first text resource discovered from
/// the EPUB spine/manifest heuristics is used.
#[allow(
    clippy::too_many_arguments,
    reason = "embedded call-site clarity; explicit bus/cs/delay/resource/offset params"
)]
pub fn probe_and_read_epub_text_chunk_from_resource<BUS, CS, DELAY, const PATH_BYTES: usize>(
    bus: &mut BUS,
    cs: &mut CS,
    delay: &mut DELAY,
    books_dir_name: &str,
    short_name: &str,
    text_resource_hint: &str,
    start_offset: u32,
    out: &mut [u8],
) -> Result<SdEpubTextChunkResult<PATH_BYTES>, SdProbeError<BUS::Error, CS::Error>>
where
    BUS: SpiBus<u8>,
    CS: OutputPin,
    DELAY: DelayNs,
    BUS::Error: core::fmt::Debug,
    CS::Error: core::fmt::Debug,
{
    cs.set_high().map_err(SdProbeError::ChipSelect)?;

    // SD SPI init requires >=74 clock cycles with CS deasserted.
    let preclock = [0xFFu8; 10];
    bus.write(&preclock).map_err(SdProbeError::Spi)?;

    let spi_device = ExclusiveSpiDevice::new(bus, cs);
    let mut delay_ref = DelayRef(delay);
    let sd_card = SdCard::new(spi_device, &mut delay_ref);
    let card_size_bytes = sd_card.num_bytes().map_err(SdProbeError::Card)?;

    let mut result = SdEpubTextChunkResult {
        card_size_bytes,
        text_resource: String::new(),
        chapter_index: 0,
        chapter_total: 1,
        compression: 0,
        bytes_read: 0,
        end_of_resource: false,
        status: SdEpubTextChunkStatus::NoTextResource,
    };

    let mut volume_mgr = VolumeManager::new(sd_card, FixedTimeSource);
    let mut volume = volume_mgr
        .open_volume(VolumeIdx(0))
        .map_err(SdProbeError::Filesystem)?;
    let mut root_dir = volume.open_root_dir().map_err(SdProbeError::Filesystem)?;

    let mut books_dir = match root_dir.open_dir(books_dir_name) {
        Ok(dir) => dir,
        Err(embedded_sdmmc::Error::NotFound) => return Ok(result),
        Err(err) => return Err(SdProbeError::Filesystem(err)),
    };

    let short_name = match ShortFileName::create_from_str(short_name) {
        Ok(name) => name,
        Err(_) => return Ok(result),
    };

    let mut file = match books_dir.open_file_in_dir(&short_name, Mode::ReadOnly) {
        Ok(file) => file,
        Err(embedded_sdmmc::Error::NotFound) => return Ok(result),
        Err(err) => return Err(SdProbeError::Filesystem(err)),
    };

    let mut header = [0u8; 4];
    let read_now = file.read(&mut header).map_err(SdProbeError::Filesystem)?;
    if !has_zip_signature(header, read_now) {
        result.status = SdEpubTextChunkStatus::NotZip;
        file.close().map_err(SdProbeError::Filesystem)?;
        books_dir.close().map_err(SdProbeError::Filesystem)?;
        root_dir.close().map_err(SdProbeError::Filesystem)?;
        volume.close().map_err(SdProbeError::Filesystem)?;
        return Ok(result);
    }

    let file_size = file.length();
    let selected_entry = if text_resource_hint.is_empty() {
        find_first_text_entry::<_, _, _, _, _, PATH_BYTES>(&mut file, file_size)
            .map_err(SdProbeError::Filesystem)?
    } else {
        let mut resource = String::<PATH_BYTES>::new();
        for ch in text_resource_hint.chars() {
            if resource.push(ch).is_err() {
                break;
            }
        }
        find_entry_by_path(&mut file, file_size, text_resource_hint.as_bytes())
            .map_err(SdProbeError::Filesystem)?
            .map(|entry| (entry, resource))
    };

    if let Some((entry, resource)) = selected_entry {
        result.compression = entry.compression;
        result.text_resource = resource;
        if let Ok(Some((chapter_index, chapter_total))) =
            spine_position_for_resource(&mut file, file_size, result.text_resource.as_str())
        {
            result.chapter_index = chapter_index;
            result.chapter_total = chapter_total;
        }

        if matches!(entry.compression, 0 | 8) {
            let (bytes_read, end_of_resource) =
                read_zip_entry_chunk(&mut file, entry, start_offset, out)
                    .map_err(SdProbeError::Filesystem)?;
            result.bytes_read = bytes_read;
            result.end_of_resource = end_of_resource;
            result.status = if bytes_read > 0 || end_of_resource {
                SdEpubTextChunkStatus::ReadOk
            } else {
                SdEpubTextChunkStatus::DecodeFailed
            };
        } else {
            result.status = SdEpubTextChunkStatus::UnsupportedCompression;
        }
    }

    file.close().map_err(SdProbeError::Filesystem)?;
    books_dir.close().map_err(SdProbeError::Filesystem)?;
    root_dir.close().map_err(SdProbeError::Filesystem)?;
    volume.close().map_err(SdProbeError::Filesystem)?;

    Ok(result)
}

/// Probe card + mount FAT + read the first chunk from the next text resource
/// after `current_resource`.
pub fn probe_and_read_next_epub_text_chunk<BUS, CS, DELAY, const PATH_BYTES: usize>(
    bus: &mut BUS,
    cs: &mut CS,
    delay: &mut DELAY,
    books_dir_name: &str,
    short_name: &str,
    current_resource: &str,
    out: &mut [u8],
) -> Result<SdEpubTextChunkResult<PATH_BYTES>, SdProbeError<BUS::Error, CS::Error>>
where
    BUS: SpiBus<u8>,
    CS: OutputPin,
    DELAY: DelayNs,
    BUS::Error: core::fmt::Debug,
    CS::Error: core::fmt::Debug,
{
    cs.set_high().map_err(SdProbeError::ChipSelect)?;

    // SD SPI init requires >=74 clock cycles with CS deasserted.
    let preclock = [0xFFu8; 10];
    bus.write(&preclock).map_err(SdProbeError::Spi)?;

    let spi_device = ExclusiveSpiDevice::new(bus, cs);
    let mut delay_ref = DelayRef(delay);
    let sd_card = SdCard::new(spi_device, &mut delay_ref);
    let card_size_bytes = sd_card.num_bytes().map_err(SdProbeError::Card)?;

    let mut result = SdEpubTextChunkResult {
        card_size_bytes,
        text_resource: String::new(),
        chapter_index: 0,
        chapter_total: 1,
        compression: 0,
        bytes_read: 0,
        end_of_resource: false,
        status: SdEpubTextChunkStatus::NoTextResource,
    };

    let mut volume_mgr = VolumeManager::new(sd_card, FixedTimeSource);
    let mut volume = volume_mgr
        .open_volume(VolumeIdx(0))
        .map_err(SdProbeError::Filesystem)?;
    let mut root_dir = volume.open_root_dir().map_err(SdProbeError::Filesystem)?;

    let mut books_dir = match root_dir.open_dir(books_dir_name) {
        Ok(dir) => dir,
        Err(embedded_sdmmc::Error::NotFound) => return Ok(result),
        Err(err) => return Err(SdProbeError::Filesystem(err)),
    };

    let short_name = match ShortFileName::create_from_str(short_name) {
        Ok(name) => name,
        Err(_) => return Ok(result),
    };

    let mut file = match books_dir.open_file_in_dir(&short_name, Mode::ReadOnly) {
        Ok(file) => file,
        Err(embedded_sdmmc::Error::NotFound) => return Ok(result),
        Err(err) => return Err(SdProbeError::Filesystem(err)),
    };

    let mut header = [0u8; 4];
    let read_now = file.read(&mut header).map_err(SdProbeError::Filesystem)?;
    if !has_zip_signature(header, read_now) {
        result.status = SdEpubTextChunkStatus::NotZip;
        file.close().map_err(SdProbeError::Filesystem)?;
        books_dir.close().map_err(SdProbeError::Filesystem)?;
        root_dir.close().map_err(SdProbeError::Filesystem)?;
        volume.close().map_err(SdProbeError::Filesystem)?;
        return Ok(result);
    }

    let file_size = file.length();
    if let Some((entry, resource)) =
        find_next_text_entry::<_, _, _, _, _, PATH_BYTES>(&mut file, file_size, current_resource)
            .map_err(SdProbeError::Filesystem)?
    {
        result.compression = entry.compression;
        result.text_resource = resource;
        if let Ok(Some((chapter_index, chapter_total))) =
            spine_position_for_resource(&mut file, file_size, result.text_resource.as_str())
        {
            result.chapter_index = chapter_index;
            result.chapter_total = chapter_total;
        }

        if matches!(entry.compression, 0 | 8) {
            let (bytes_read, end_of_resource) =
                read_zip_entry_chunk(&mut file, entry, 0, out).map_err(SdProbeError::Filesystem)?;
            result.bytes_read = bytes_read;
            result.end_of_resource = end_of_resource;
            result.status = if bytes_read > 0 || end_of_resource {
                SdEpubTextChunkStatus::ReadOk
            } else {
                SdEpubTextChunkStatus::DecodeFailed
            };
        } else {
            result.status = SdEpubTextChunkStatus::UnsupportedCompression;
        }
    }

    file.close().map_err(SdProbeError::Filesystem)?;
    books_dir.close().map_err(SdProbeError::Filesystem)?;
    root_dir.close().map_err(SdProbeError::Filesystem)?;
    volume.close().map_err(SdProbeError::Filesystem)?;

    Ok(result)
}

/// Probe card + mount FAT + scan `/books` for ZIP signatures (EPUB candidates).
pub fn probe_and_scan_epubs<
    BUS,
    CS,
    DELAY,
    const MAX_EPUBS: usize,
    const NAME_BYTES: usize,
    const MAX_CANDIDATES: usize,
>(
    bus: &mut BUS,
    cs: &mut CS,
    delay: &mut DELAY,
    books_dir_name: &str,
) -> Result<SdEpubScanResult<MAX_EPUBS, NAME_BYTES>, SdProbeError<BUS::Error, CS::Error>>
where
    BUS: SpiBus<u8>,
    CS: OutputPin,
    DELAY: DelayNs,
    BUS::Error: core::fmt::Debug,
    CS::Error: core::fmt::Debug,
{
    cs.set_high().map_err(SdProbeError::ChipSelect)?;

    // SD SPI init requires >=74 clock cycles with CS deasserted.
    let preclock = [0xFFu8; 10];
    bus.write(&preclock).map_err(SdProbeError::Spi)?;

    let spi_device = ExclusiveSpiDevice::new(bus, cs);
    let mut delay_ref = DelayRef(delay);
    let sd_card = SdCard::new(spi_device, &mut delay_ref);
    let card_size_bytes = sd_card.num_bytes().map_err(SdProbeError::Card)?;

    let mut scan = SdEpubScanResult {
        card_size_bytes,
        books_dir_found: false,
        scanned_file_count: 0,
        epub_count_total: 0,
        epub_entries: Vec::new(),
        truncated: false,
    };

    let mut volume_mgr = VolumeManager::new(sd_card, FixedTimeSource);
    let mut volume = volume_mgr
        .open_volume(VolumeIdx(0))
        .map_err(SdProbeError::Filesystem)?;
    let mut root_dir = volume.open_root_dir().map_err(SdProbeError::Filesystem)?;

    let mut books_dir = match root_dir.open_dir(books_dir_name) {
        Ok(dir) => {
            scan.books_dir_found = true;
            dir
        }
        Err(embedded_sdmmc::Error::NotFound) => return Ok(scan),
        Err(err) => return Err(SdProbeError::Filesystem(err)),
    };

    let mut candidates: Vec<ShortFileName, MAX_CANDIDATES> = Vec::new();
    books_dir
        .iterate_dir(|entry| {
            if entry.attributes.is_directory() || entry.attributes.is_volume() {
                return;
            }

            scan.scanned_file_count = scan.scanned_file_count.saturating_add(1);
            if candidates.push(entry.name.clone()).is_err() {
                scan.truncated = true;
            }
        })
        .map_err(SdProbeError::Filesystem)?;

    for name in candidates.iter() {
        let mut file = books_dir
            .open_file_in_dir(name, Mode::ReadOnly)
            .map_err(SdProbeError::Filesystem)?;
        let size_bytes = file.length();
        let short_name = short_file_name_to_string(name);
        let mut display_title = display_title_from_file_name(short_name.as_str());
        let mut has_cover = false;

        let mut header = [0u8; 4];
        let read_now = file.read(&mut header).map_err(SdProbeError::Filesystem)?;

        if has_zip_signature(header, read_now)
            && let Ok(metadata) = scan_epub_metadata(&mut file, size_bytes, &mut display_title)
        {
            has_cover = metadata.has_cover;
        }

        file.close().map_err(SdProbeError::Filesystem)?;

        if !has_zip_signature(header, read_now) {
            continue;
        }

        scan.epub_count_total = scan.epub_count_total.saturating_add(1);
        let entry = SdEpubEntry {
            short_name,
            display_title,
            has_cover,
            size_bytes,
        };
        if scan.epub_entries.push(entry).is_err() {
            scan.truncated = true;
        }
    }

    books_dir.close().map_err(SdProbeError::Filesystem)?;
    root_dir.close().map_err(SdProbeError::Filesystem)?;
    volume.close().map_err(SdProbeError::Filesystem)?;

    Ok(scan)
}

/// Probe card + mount FAT + read one fixed root file into `out`.
pub fn probe_and_read_test_file<BUS, CS, DELAY>(
    bus: &mut BUS,
    cs: &mut CS,
    delay: &mut DELAY,
    test_file_name: &str,
    out: &mut [u8],
) -> Result<SdProbeResult, SdProbeError<BUS::Error, CS::Error>>
where
    BUS: SpiBus<u8>,
    CS: OutputPin,
    DELAY: DelayNs,
    BUS::Error: core::fmt::Debug,
    CS::Error: core::fmt::Debug,
{
    cs.set_high().map_err(SdProbeError::ChipSelect)?;

    // SD SPI init requires >=74 clock cycles with CS deasserted.
    let preclock = [0xFFu8; 10];
    bus.write(&preclock).map_err(SdProbeError::Spi)?;

    let spi_device = ExclusiveSpiDevice::new(bus, cs);
    let mut delay_ref = DelayRef(delay);
    let sd_card = SdCard::new(spi_device, &mut delay_ref);
    let card_size_bytes = sd_card.num_bytes().map_err(SdProbeError::Card)?;

    let mut volume_mgr = VolumeManager::new(sd_card, FixedTimeSource);
    let mut volume = volume_mgr
        .open_volume(VolumeIdx(0))
        .map_err(SdProbeError::Filesystem)?;
    let mut root_dir = volume.open_root_dir().map_err(SdProbeError::Filesystem)?;
    let mut file = root_dir
        .open_file_in_dir(test_file_name, Mode::ReadOnly)
        .map_err(SdProbeError::Filesystem)?;

    let mut read_total = 0usize;
    while read_total < out.len() {
        let read_now = file
            .read(&mut out[read_total..])
            .map_err(SdProbeError::Filesystem)?;
        if read_now == 0 || file.is_eof() {
            break;
        }
        read_total = read_total.saturating_add(read_now);
    }

    file.close().map_err(SdProbeError::Filesystem)?;
    root_dir.close().map_err(SdProbeError::Filesystem)?;
    volume.close().map_err(SdProbeError::Filesystem)?;

    Ok(SdProbeResult {
        card_size_bytes,
        bytes_read: read_total,
    })
}
