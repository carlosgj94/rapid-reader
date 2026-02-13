use readily_core::{
    content::{
        NavigationCatalog, SelectableWordSource, WordSource, sd_catalog::SdCatalogSource,
    },
    settings::ResumeState,
};

pub const SD_BOOK_DB_MANIFEST_FILE: &str = "MANIFEST.RDB";
pub const SD_BOOK_DB_PROGRESS_FILE: &str = "PROGRESS.RDB";

const BOOK_DB_MAGIC: [u8; 4] = *b"RDB1";
const BOOK_DB_VERSION: u8 = 1;
const BOOK_DB_KIND_MANIFEST: u8 = 1;
const BOOK_DB_KIND_BOOK: u8 = 2;
const BOOK_DB_KIND_PROGRESS: u8 = 3;
const BOOK_DB_HEADER_BYTES: usize = 16;
const BOOK_DB_FILE_NAME_BYTES: usize = 12;
const BOOK_DB_CHUNK_BYTES: usize = 480;
const BOOK_DB_MAX_MANIFEST_PAYLOAD_BYTES: u32 = 64 * 1024;
const BOOK_DB_MAX_PROGRESS_PAYLOAD_BYTES: u32 = 16 * 1024;

#[derive(Debug, Clone)]
pub struct SdBookDbManifestEntry<
    const NAME_BYTES: usize,
    const TITLE_BYTES: usize,
    const PATH_BYTES: usize,
    const LABEL_BYTES: usize,
> {
    pub short_name: String<NAME_BYTES>,
    pub display_title: String<TITLE_BYTES>,
    pub has_cover: bool,
    pub chapter_total: u16,
    pub first_resource: String<PATH_BYTES>,
    pub first_offset: u32,
    pub first_chapter_label: String<LABEL_BYTES>,
}

#[derive(Debug, Clone)]
pub struct SdBookDbManifest<
    const MAX_BOOKS: usize,
    const NAME_BYTES: usize,
    const TITLE_BYTES: usize,
    const PATH_BYTES: usize,
    const LABEL_BYTES: usize,
> {
    pub entries: Vec<SdBookDbManifestEntry<NAME_BYTES, TITLE_BYTES, PATH_BYTES, LABEL_BYTES>, MAX_BOOKS>,
}

#[derive(Debug, Clone)]
pub struct SdBookDbParagraph<const PREVIEW_BYTES: usize> {
    pub ordinal: u16,
    pub offset_hint: u32,
    pub preview: String<PREVIEW_BYTES>,
}

#[derive(Debug, Clone)]
pub struct SdBookDbChapter<
    const PATH_BYTES: usize,
    const LABEL_BYTES: usize,
    const PARA_MAX: usize,
    const PREVIEW_BYTES: usize,
> {
    pub label: String<LABEL_BYTES>,
    pub resource: String<PATH_BYTES>,
    pub start_offset: u32,
    pub paragraph_total_hint: u16,
    pub paragraphs: Vec<SdBookDbParagraph<PREVIEW_BYTES>, PARA_MAX>,
}

#[derive(Debug, Clone)]
pub struct SdBookDbRecord<
    const NAME_BYTES: usize,
    const TITLE_BYTES: usize,
    const CHAPTER_MAX: usize,
    const PATH_BYTES: usize,
    const LABEL_BYTES: usize,
    const PARA_MAX: usize,
    const PREVIEW_BYTES: usize,
> {
    pub short_name: String<NAME_BYTES>,
    pub display_title: String<TITLE_BYTES>,
    pub has_cover: bool,
    pub chapters: Vec<SdBookDbChapter<PATH_BYTES, LABEL_BYTES, PARA_MAX, PREVIEW_BYTES>, CHAPTER_MAX>,
}

#[derive(Debug, Clone)]
pub struct SdBookDbProgressEntry<const NAME_BYTES: usize> {
    pub short_name: String<NAME_BYTES>,
    pub chapter_index: u16,
    pub paragraph_in_chapter: u16,
    pub word_index: u16,
}

#[derive(Debug, Clone)]
pub struct SdBookDbProgress<const MAX_BOOKS: usize, const NAME_BYTES: usize> {
    pub last_open_short_name: String<NAME_BYTES>,
    pub entries: Vec<SdBookDbProgressEntry<NAME_BYTES>, MAX_BOOKS>,
}

pub fn probe_and_load_book_db_manifest<
    BUS,
    CS,
    DELAY,
    const MAX_BOOKS: usize,
    const NAME_BYTES: usize,
    const TITLE_BYTES: usize,
    const PATH_BYTES: usize,
    const LABEL_BYTES: usize,
>(
    bus: &mut BUS,
    cs: &mut CS,
    delay: &mut DELAY,
    books_dir_name: &str,
) -> Result<
    Option<SdBookDbManifest<MAX_BOOKS, NAME_BYTES, TITLE_BYTES, PATH_BYTES, LABEL_BYTES>>,
    SdProbeError<BUS::Error, CS::Error>,
>
where
    BUS: SpiBus<u8>,
    CS: OutputPin,
    DELAY: DelayNs,
    BUS::Error: core::fmt::Debug,
    CS::Error: core::fmt::Debug,
{
    cs.set_high().map_err(SdProbeError::ChipSelect)?;
    let preclock = [0xFFu8; 10];
    bus.write(&preclock).map_err(SdProbeError::Spi)?;

    let spi_device = ExclusiveSpiDevice::new(bus, cs);
    let mut delay_ref = DelayRef(delay);
    let sd_card = SdCard::new(spi_device, &mut delay_ref);
    let _ = sd_card.num_bytes().map_err(SdProbeError::Card)?;

    let mut volume_mgr = VolumeManager::new(sd_card, FixedTimeSource);
    let mut volume = volume_mgr
        .open_volume(VolumeIdx(0))
        .map_err(SdProbeError::Filesystem)?;
    let mut root_dir = volume.open_root_dir().map_err(SdProbeError::Filesystem)?;
    let mut books_dir = match root_dir.open_dir(books_dir_name) {
        Ok(dir) => dir,
        Err(embedded_sdmmc::Error::NotFound) => return Ok(None),
        Err(err) => return Err(SdProbeError::Filesystem(err)),
    };
    let mut file = match books_dir.open_file_in_dir(SD_BOOK_DB_MANIFEST_FILE, Mode::ReadOnly) {
        Ok(file) => file,
        Err(embedded_sdmmc::Error::NotFound) => return Ok(None),
        Err(err) => return Err(SdProbeError::Filesystem(err)),
    };

    let mut manifest = SdBookDbManifest {
        entries: Vec::new(),
    };
    let parsed = read_manifest_payload::<
        _,
        _,
        _,
        _,
        _,
        MAX_BOOKS,
        NAME_BYTES,
        TITLE_BYTES,
        PATH_BYTES,
        LABEL_BYTES,
    >(&mut file, &mut manifest)
    .map_err(SdProbeError::Filesystem)?;

    file.close().map_err(SdProbeError::Filesystem)?;
    books_dir.close().map_err(SdProbeError::Filesystem)?;
    root_dir.close().map_err(SdProbeError::Filesystem)?;
    volume.close().map_err(SdProbeError::Filesystem)?;

    if parsed { Ok(Some(manifest)) } else { Ok(None) }
}

pub fn probe_and_save_book_db_manifest<
    BUS,
    CS,
    DELAY,
    const MAX_BOOKS: usize,
    const NAME_BYTES: usize,
    const TITLE_BYTES: usize,
    const PATH_BYTES: usize,
    const LABEL_BYTES: usize,
>(
    bus: &mut BUS,
    cs: &mut CS,
    delay: &mut DELAY,
    books_dir_name: &str,
    manifest: &SdBookDbManifest<MAX_BOOKS, NAME_BYTES, TITLE_BYTES, PATH_BYTES, LABEL_BYTES>,
) -> Result<(), SdProbeError<BUS::Error, CS::Error>>
where
    BUS: SpiBus<u8>,
    CS: OutputPin,
    DELAY: DelayNs,
    BUS::Error: core::fmt::Debug,
    CS::Error: core::fmt::Debug,
{
    cs.set_high().map_err(SdProbeError::ChipSelect)?;
    let preclock = [0xFFu8; 10];
    bus.write(&preclock).map_err(SdProbeError::Spi)?;

    let spi_device = ExclusiveSpiDevice::new(bus, cs);
    let mut delay_ref = DelayRef(delay);
    let sd_card = SdCard::new(spi_device, &mut delay_ref);
    let _ = sd_card.num_bytes().map_err(SdProbeError::Card)?;

    let mut volume_mgr = VolumeManager::new(sd_card, FixedTimeSource);
    let mut volume = volume_mgr
        .open_volume(VolumeIdx(0))
        .map_err(SdProbeError::Filesystem)?;
    let mut root_dir = volume.open_root_dir().map_err(SdProbeError::Filesystem)?;
    let mut books_dir = match root_dir.open_dir(books_dir_name) {
        Ok(dir) => dir,
        Err(embedded_sdmmc::Error::NotFound) => return Ok(()),
        Err(err) => return Err(SdProbeError::Filesystem(err)),
    };
    let mut file = books_dir
        .open_file_in_dir(SD_BOOK_DB_MANIFEST_FILE, Mode::ReadWriteCreateOrTruncate)
        .map_err(SdProbeError::Filesystem)?;

    write_manifest_payload(&mut file, manifest).map_err(SdProbeError::Filesystem)?;

    file.close().map_err(SdProbeError::Filesystem)?;
    books_dir.close().map_err(SdProbeError::Filesystem)?;
    root_dir.close().map_err(SdProbeError::Filesystem)?;
    volume.close().map_err(SdProbeError::Filesystem)?;
    Ok(())
}

pub fn probe_and_build_book_db_record<
    BUS,
    CS,
    DELAY,
    const NAME_BYTES: usize,
    const TITLE_BYTES: usize,
    const CHAPTER_MAX: usize,
    const PATH_BYTES: usize,
    const LABEL_BYTES: usize,
    const PARA_MAX: usize,
    const PREVIEW_BYTES: usize,
>(
    bus: &mut BUS,
    cs: &mut CS,
    delay: &mut DELAY,
    books_dir_name: &str,
    short_name: &str,
    display_title: &str,
    has_cover: bool,
) -> Result<
    Option<
        SdBookDbRecord<
            NAME_BYTES,
            TITLE_BYTES,
            CHAPTER_MAX,
            PATH_BYTES,
            LABEL_BYTES,
            PARA_MAX,
            PREVIEW_BYTES,
        >,
    >,
    SdProbeError<BUS::Error, CS::Error>,
>
where
    BUS: SpiBus<u8>,
    CS: OutputPin,
    DELAY: DelayNs,
    BUS::Error: core::fmt::Debug,
    CS::Error: core::fmt::Debug,
{
    let mut record = SdBookDbRecord {
        short_name: bounded_string::<NAME_BYTES>(short_name),
        display_title: bounded_string::<TITLE_BYTES>(display_title),
        has_cover,
        chapters: Vec::new(),
    };

    let mut chunk = [0u8; BOOK_DB_CHUNK_BYTES];
    let chapter0 = probe_and_read_epub_text_chunk_at_chapter::<_, _, _, PATH_BYTES>(
        bus,
        cs,
        delay,
        books_dir_name,
        short_name,
        0,
        &mut chunk,
    )?;
    if !matches!(chapter0.status, SdEpubTextChunkStatus::ReadOk) {
        return Ok(None);
    }

    let chapter_total = chapter0.chapter_total.max(1).min(CHAPTER_MAX as u16);
    for chapter_index in 0..chapter_total {
        let probe = probe_and_read_epub_text_chunk_at_chapter::<_, _, _, PATH_BYTES>(
            bus,
            cs,
            delay,
            books_dir_name,
            short_name,
            chapter_index,
            &mut chunk,
        )?;
        if !matches!(probe.status, SdEpubTextChunkStatus::ReadOk) {
            continue;
        }

        let mut label = String::<LABEL_BYTES>::new();
        if probe.chapter_label.is_empty() {
            let _ = label.push_str("Chapter ");
            push_u16_ascii(&mut label, chapter_index.saturating_add(1));
        } else {
            copy_string_into(&probe.chapter_label, &mut label);
        }

        let paragraphs = index_paragraphs_from_chunk::<PARA_MAX, PREVIEW_BYTES, PATH_BYTES>(
            &chunk[..probe.bytes_read.min(chunk.len())],
            probe.end_of_resource,
            probe.text_resource.as_str(),
        );
        let paragraph_total_hint = paragraphs
            .len()
            .clamp(0, u16::MAX as usize) as u16;

        let mut resource = String::<PATH_BYTES>::new();
        copy_string_into(&probe.text_resource, &mut resource);

        let chapter = SdBookDbChapter {
            label,
            resource,
            start_offset: probe.start_offset,
            paragraph_total_hint,
            paragraphs,
        };

        if record.chapters.push(chapter).is_err() {
            break;
        }
    }

    if record.chapters.is_empty() {
        return Ok(None);
    }
    Ok(Some(record))
}

pub fn probe_and_save_book_db_record<
    BUS,
    CS,
    DELAY,
    const NAME_BYTES: usize,
    const TITLE_BYTES: usize,
    const CHAPTER_MAX: usize,
    const PATH_BYTES: usize,
    const LABEL_BYTES: usize,
    const PARA_MAX: usize,
    const PREVIEW_BYTES: usize,
>(
    bus: &mut BUS,
    cs: &mut CS,
    delay: &mut DELAY,
    books_dir_name: &str,
    record: &SdBookDbRecord<
        NAME_BYTES,
        TITLE_BYTES,
        CHAPTER_MAX,
        PATH_BYTES,
        LABEL_BYTES,
        PARA_MAX,
        PREVIEW_BYTES,
    >,
) -> Result<(), SdProbeError<BUS::Error, CS::Error>>
where
    BUS: SpiBus<u8>,
    CS: OutputPin,
    DELAY: DelayNs,
    BUS::Error: core::fmt::Debug,
    CS::Error: core::fmt::Debug,
{
    cs.set_high().map_err(SdProbeError::ChipSelect)?;
    let preclock = [0xFFu8; 10];
    bus.write(&preclock).map_err(SdProbeError::Spi)?;

    let spi_device = ExclusiveSpiDevice::new(bus, cs);
    let mut delay_ref = DelayRef(delay);
    let sd_card = SdCard::new(spi_device, &mut delay_ref);
    let _ = sd_card.num_bytes().map_err(SdProbeError::Card)?;

    let mut volume_mgr = VolumeManager::new(sd_card, FixedTimeSource);
    let mut volume = volume_mgr
        .open_volume(VolumeIdx(0))
        .map_err(SdProbeError::Filesystem)?;
    let mut root_dir = volume.open_root_dir().map_err(SdProbeError::Filesystem)?;
    let mut books_dir = match root_dir.open_dir(books_dir_name) {
        Ok(dir) => dir,
        Err(embedded_sdmmc::Error::NotFound) => return Ok(()),
        Err(err) => return Err(SdProbeError::Filesystem(err)),
    };
    let file_name = db_file_name_for_short_name(record.short_name.as_str());
    let mut file = books_dir
        .open_file_in_dir(file_name.as_str(), Mode::ReadWriteCreateOrTruncate)
        .map_err(SdProbeError::Filesystem)?;

    write_record_payload(&mut file, record).map_err(SdProbeError::Filesystem)?;

    file.close().map_err(SdProbeError::Filesystem)?;
    books_dir.close().map_err(SdProbeError::Filesystem)?;
    root_dir.close().map_err(SdProbeError::Filesystem)?;
    volume.close().map_err(SdProbeError::Filesystem)?;
    Ok(())
}

pub fn probe_and_load_book_db_progress<
    BUS,
    CS,
    DELAY,
    const MAX_BOOKS: usize,
    const NAME_BYTES: usize,
>(
    bus: &mut BUS,
    cs: &mut CS,
    delay: &mut DELAY,
    books_dir_name: &str,
) -> Result<Option<SdBookDbProgress<MAX_BOOKS, NAME_BYTES>>, SdProbeError<BUS::Error, CS::Error>>
where
    BUS: SpiBus<u8>,
    CS: OutputPin,
    DELAY: DelayNs,
    BUS::Error: core::fmt::Debug,
    CS::Error: core::fmt::Debug,
{
    cs.set_high().map_err(SdProbeError::ChipSelect)?;
    let preclock = [0xFFu8; 10];
    bus.write(&preclock).map_err(SdProbeError::Spi)?;

    let spi_device = ExclusiveSpiDevice::new(bus, cs);
    let mut delay_ref = DelayRef(delay);
    let sd_card = SdCard::new(spi_device, &mut delay_ref);
    let _ = sd_card.num_bytes().map_err(SdProbeError::Card)?;

    let mut volume_mgr = VolumeManager::new(sd_card, FixedTimeSource);
    let mut volume = volume_mgr
        .open_volume(VolumeIdx(0))
        .map_err(SdProbeError::Filesystem)?;
    let mut root_dir = volume.open_root_dir().map_err(SdProbeError::Filesystem)?;
    let mut books_dir = match root_dir.open_dir(books_dir_name) {
        Ok(dir) => dir,
        Err(embedded_sdmmc::Error::NotFound) => return Ok(None),
        Err(err) => return Err(SdProbeError::Filesystem(err)),
    };
    let mut file = match books_dir.open_file_in_dir(SD_BOOK_DB_PROGRESS_FILE, Mode::ReadOnly) {
        Ok(file) => file,
        Err(embedded_sdmmc::Error::NotFound) => return Ok(None),
        Err(err) => return Err(SdProbeError::Filesystem(err)),
    };

    let mut progress = SdBookDbProgress {
        last_open_short_name: String::new(),
        entries: Vec::new(),
    };
    let parsed = read_progress_payload(&mut file, &mut progress).map_err(SdProbeError::Filesystem)?;
    file.close().map_err(SdProbeError::Filesystem)?;
    books_dir.close().map_err(SdProbeError::Filesystem)?;
    root_dir.close().map_err(SdProbeError::Filesystem)?;
    volume.close().map_err(SdProbeError::Filesystem)?;
    if parsed { Ok(Some(progress)) } else { Ok(None) }
}

pub fn probe_and_upsert_book_db_progress<
    BUS,
    CS,
    DELAY,
    const MAX_BOOKS: usize,
    const NAME_BYTES: usize,
>(
    bus: &mut BUS,
    cs: &mut CS,
    delay: &mut DELAY,
    books_dir_name: &str,
    short_name: &str,
    resume: ResumeState,
) -> Result<(), SdProbeError<BUS::Error, CS::Error>>
where
    BUS: SpiBus<u8>,
    CS: OutputPin,
    DELAY: DelayNs,
    BUS::Error: core::fmt::Debug,
    CS::Error: core::fmt::Debug,
{
    let mut progress = probe_and_load_book_db_progress::<_, _, _, MAX_BOOKS, NAME_BYTES>(
        bus,
        cs,
        delay,
        books_dir_name,
    )?
    .unwrap_or(SdBookDbProgress {
        last_open_short_name: String::new(),
        entries: Vec::new(),
    });

    progress.last_open_short_name.clear();
    push_str_bounded(&mut progress.last_open_short_name, short_name);

    let mut updated = false;
    for entry in progress.entries.iter_mut() {
        if entry.short_name.as_str().eq_ignore_ascii_case(short_name) {
            entry.chapter_index = resume.chapter_index;
            entry.paragraph_in_chapter = resume.paragraph_in_chapter;
            entry.word_index = resume.word_index.max(1);
            updated = true;
            break;
        }
    }

    if !updated && !progress.entries.is_full() {
        let mut name = String::<NAME_BYTES>::new();
        push_str_bounded(&mut name, short_name);
        let _ = progress.entries.push(SdBookDbProgressEntry {
            short_name: name,
            chapter_index: resume.chapter_index,
            paragraph_in_chapter: resume.paragraph_in_chapter,
            word_index: resume.word_index.max(1),
        });
    }

    save_progress_payload(
        bus,
        cs,
        delay,
        books_dir_name,
        &progress,
    )
}

fn save_progress_payload<
    BUS,
    CS,
    DELAY,
    const MAX_BOOKS: usize,
    const NAME_BYTES: usize,
>(
    bus: &mut BUS,
    cs: &mut CS,
    delay: &mut DELAY,
    books_dir_name: &str,
    progress: &SdBookDbProgress<MAX_BOOKS, NAME_BYTES>,
) -> Result<(), SdProbeError<BUS::Error, CS::Error>>
where
    BUS: SpiBus<u8>,
    CS: OutputPin,
    DELAY: DelayNs,
    BUS::Error: core::fmt::Debug,
    CS::Error: core::fmt::Debug,
{
    cs.set_high().map_err(SdProbeError::ChipSelect)?;
    let preclock = [0xFFu8; 10];
    bus.write(&preclock).map_err(SdProbeError::Spi)?;

    let spi_device = ExclusiveSpiDevice::new(bus, cs);
    let mut delay_ref = DelayRef(delay);
    let sd_card = SdCard::new(spi_device, &mut delay_ref);
    let _ = sd_card.num_bytes().map_err(SdProbeError::Card)?;

    let mut volume_mgr = VolumeManager::new(sd_card, FixedTimeSource);
    let mut volume = volume_mgr
        .open_volume(VolumeIdx(0))
        .map_err(SdProbeError::Filesystem)?;
    let mut root_dir = volume.open_root_dir().map_err(SdProbeError::Filesystem)?;
    let mut books_dir = match root_dir.open_dir(books_dir_name) {
        Ok(dir) => dir,
        Err(embedded_sdmmc::Error::NotFound) => return Ok(()),
        Err(err) => return Err(SdProbeError::Filesystem(err)),
    };
    let mut file = books_dir
        .open_file_in_dir(SD_BOOK_DB_PROGRESS_FILE, Mode::ReadWriteCreateOrTruncate)
        .map_err(SdProbeError::Filesystem)?;
    write_progress_payload(&mut file, progress).map_err(SdProbeError::Filesystem)?;
    file.close().map_err(SdProbeError::Filesystem)?;
    books_dir.close().map_err(SdProbeError::Filesystem)?;
    root_dir.close().map_err(SdProbeError::Filesystem)?;
    volume.close().map_err(SdProbeError::Filesystem)?;
    Ok(())
}

fn index_paragraphs_from_chunk<const PARA_MAX: usize, const PREVIEW_BYTES: usize, const PATH_BYTES: usize>(
    chunk: &[u8],
    end_of_resource: bool,
    resource_path: &str,
) -> Vec<SdBookDbParagraph<PREVIEW_BYTES>, PARA_MAX> {
    let mut out = Vec::<SdBookDbParagraph<PREVIEW_BYTES>, PARA_MAX>::new();
    if chunk.is_empty() {
        return out;
    }

    let mut parser = SdCatalogSource::new();
    let _ = parser.set_catalog_entries_from_iter(core::iter::once(("Index", false)));
    if parser.select_text(0).is_err() {
        return out;
    }
    if parser
        .set_catalog_text_chunk_from_bytes(0, chunk, end_of_resource, resource_path)
        .is_err()
    {
        return out;
    }

    let total = parser.paragraph_total();
    for paragraph_idx in 0..total {
        if out.is_full() {
            break;
        }
        let Some(raw) = parser.paragraph_preview(paragraph_idx) else {
            continue;
        };
        let text = raw.trim();
        if text.is_empty() {
            continue;
        }
        let mut preview = String::<PREVIEW_BYTES>::new();
        push_str_bounded(&mut preview, text);
        if preview.is_empty() {
            continue;
        }
        if out
            .last()
            .is_some_and(|entry| entry.preview.as_str() == preview.as_str())
        {
            continue;
        }
        let _ = out.push(SdBookDbParagraph {
            ordinal: paragraph_idx,
            offset_hint: paragraph_idx as u32,
            preview,
        });
    }

    out
}

fn write_manifest_payload<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
    const MAX_BOOKS: usize,
    const NAME_BYTES: usize,
    const TITLE_BYTES: usize,
    const PATH_BYTES: usize,
    const LABEL_BYTES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    manifest: &SdBookDbManifest<MAX_BOOKS, NAME_BYTES, TITLE_BYTES, PATH_BYTES, LABEL_BYTES>,
) -> Result<(), embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    write_all(file, &[0u8; BOOK_DB_HEADER_BYTES])?;
    let mut payload_len = 0u32;
    let mut hash = checksum_init();

    write_u16_payload(file, &mut payload_len, &mut hash, manifest.entries.len() as u16)?;
    for entry in manifest.entries.iter() {
        write_string_payload(file, &mut payload_len, &mut hash, entry.short_name.as_str())?;
        write_string_payload(file, &mut payload_len, &mut hash, entry.display_title.as_str())?;
        write_u8_payload(
            file,
            &mut payload_len,
            &mut hash,
            if entry.has_cover { 1 } else { 0 },
        )?;
        write_u16_payload(file, &mut payload_len, &mut hash, entry.chapter_total.max(1))?;
        write_string_payload(file, &mut payload_len, &mut hash, entry.first_resource.as_str())?;
        write_u32_payload(file, &mut payload_len, &mut hash, entry.first_offset)?;
        write_string_payload(
            file,
            &mut payload_len,
            &mut hash,
            entry.first_chapter_label.as_str(),
        )?;
    }

    write_header(
        file,
        BOOK_DB_KIND_MANIFEST,
        payload_len,
        checksum_finish(hash),
    )?;
    Ok(())
}

fn read_manifest_payload<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
    const MAX_BOOKS: usize,
    const NAME_BYTES: usize,
    const TITLE_BYTES: usize,
    const PATH_BYTES: usize,
    const LABEL_BYTES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    out: &mut SdBookDbManifest<MAX_BOOKS, NAME_BYTES, TITLE_BYTES, PATH_BYTES, LABEL_BYTES>,
) -> Result<bool, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let Some((payload_len, expected_checksum)) = read_header(file, BOOK_DB_KIND_MANIFEST)? else {
        return Ok(false);
    };
    if payload_len > BOOK_DB_MAX_MANIFEST_PAYLOAD_BYTES {
        return Ok(false);
    }

    let mut payload_read = 0u32;
    let mut hash = checksum_init();
    let count = read_u16_payload(file, payload_len, &mut payload_read, &mut hash)?;
    if count as usize > MAX_BOOKS {
        return Ok(false);
    }
    out.entries.clear();
    for _ in 0..count {
        let short_name = read_string_payload::<_, _, _, _, _, NAME_BYTES>(
            file,
            payload_len,
            &mut payload_read,
            &mut hash,
        )?;
        let display_title = read_string_payload::<_, _, _, _, _, TITLE_BYTES>(
            file,
            payload_len,
            &mut payload_read,
            &mut hash,
        )?;
        let has_cover = read_u8_payload(file, payload_len, &mut payload_read, &mut hash)? != 0;
        let chapter_total = read_u16_payload(file, payload_len, &mut payload_read, &mut hash)?;
        let first_resource = read_string_payload::<_, _, _, _, _, PATH_BYTES>(
            file,
            payload_len,
            &mut payload_read,
            &mut hash,
        )?;
        let first_offset = read_u32_payload(file, payload_len, &mut payload_read, &mut hash)?;
        let first_chapter_label = read_string_payload::<_, _, _, _, _, LABEL_BYTES>(
            file,
            payload_len,
            &mut payload_read,
            &mut hash,
        )?;
        if !out.entries.is_full() {
            let _ = out.entries.push(SdBookDbManifestEntry {
                short_name,
                display_title,
                has_cover,
                chapter_total: chapter_total.max(1),
                first_resource,
                first_offset,
                first_chapter_label,
            });
        }
    }

    if payload_read != payload_len || checksum_finish(hash) != expected_checksum {
        return Ok(false);
    }
    Ok(true)
}

fn write_record_payload<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
    const NAME_BYTES: usize,
    const TITLE_BYTES: usize,
    const CHAPTER_MAX: usize,
    const PATH_BYTES: usize,
    const LABEL_BYTES: usize,
    const PARA_MAX: usize,
    const PREVIEW_BYTES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    record: &SdBookDbRecord<
        NAME_BYTES,
        TITLE_BYTES,
        CHAPTER_MAX,
        PATH_BYTES,
        LABEL_BYTES,
        PARA_MAX,
        PREVIEW_BYTES,
    >,
) -> Result<(), embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    write_all(file, &[0u8; BOOK_DB_HEADER_BYTES])?;
    let mut payload_len = 0u32;
    let mut hash = checksum_init();
    write_string_payload(file, &mut payload_len, &mut hash, record.short_name.as_str())?;
    write_string_payload(
        file,
        &mut payload_len,
        &mut hash,
        record.display_title.as_str(),
    )?;
    write_u8_payload(
        file,
        &mut payload_len,
        &mut hash,
        if record.has_cover { 1 } else { 0 },
    )?;
    write_u16_payload(file, &mut payload_len, &mut hash, record.chapters.len() as u16)?;
    for chapter in record.chapters.iter() {
        write_string_payload(file, &mut payload_len, &mut hash, chapter.label.as_str())?;
        write_string_payload(file, &mut payload_len, &mut hash, chapter.resource.as_str())?;
        write_u32_payload(file, &mut payload_len, &mut hash, chapter.start_offset)?;
        write_u16_payload(
            file,
            &mut payload_len,
            &mut hash,
            chapter.paragraph_total_hint,
        )?;
        write_u16_payload(
            file,
            &mut payload_len,
            &mut hash,
            chapter.paragraphs.len() as u16,
        )?;
        for paragraph in chapter.paragraphs.iter() {
            write_u16_payload(file, &mut payload_len, &mut hash, paragraph.ordinal)?;
            write_u32_payload(file, &mut payload_len, &mut hash, paragraph.offset_hint)?;
            write_string_payload(file, &mut payload_len, &mut hash, paragraph.preview.as_str())?;
        }
    }
    write_header(file, BOOK_DB_KIND_BOOK, payload_len, checksum_finish(hash))?;
    Ok(())
}

fn write_progress_payload<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
    const MAX_BOOKS: usize,
    const NAME_BYTES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    progress: &SdBookDbProgress<MAX_BOOKS, NAME_BYTES>,
) -> Result<(), embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    write_all(file, &[0u8; BOOK_DB_HEADER_BYTES])?;
    let mut payload_len = 0u32;
    let mut hash = checksum_init();
    write_string_payload(
        file,
        &mut payload_len,
        &mut hash,
        progress.last_open_short_name.as_str(),
    )?;
    write_u16_payload(file, &mut payload_len, &mut hash, progress.entries.len() as u16)?;
    for entry in progress.entries.iter() {
        write_string_payload(file, &mut payload_len, &mut hash, entry.short_name.as_str())?;
        write_u16_payload(file, &mut payload_len, &mut hash, entry.chapter_index)?;
        write_u16_payload(file, &mut payload_len, &mut hash, entry.paragraph_in_chapter)?;
        write_u16_payload(file, &mut payload_len, &mut hash, entry.word_index.max(1))?;
    }
    write_header(
        file,
        BOOK_DB_KIND_PROGRESS,
        payload_len,
        checksum_finish(hash),
    )?;
    Ok(())
}

fn read_progress_payload<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
    const MAX_BOOKS: usize,
    const NAME_BYTES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    out: &mut SdBookDbProgress<MAX_BOOKS, NAME_BYTES>,
) -> Result<bool, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let Some((payload_len, expected_checksum)) = read_header(file, BOOK_DB_KIND_PROGRESS)? else {
        return Ok(false);
    };
    if payload_len > BOOK_DB_MAX_PROGRESS_PAYLOAD_BYTES {
        return Ok(false);
    }
    let mut payload_read = 0u32;
    let mut hash = checksum_init();
    out.last_open_short_name = read_string_payload::<_, _, _, _, _, NAME_BYTES>(
        file,
        payload_len,
        &mut payload_read,
        &mut hash,
    )?;
    let count = read_u16_payload(file, payload_len, &mut payload_read, &mut hash)?;
    if count as usize > MAX_BOOKS {
        return Ok(false);
    }
    out.entries.clear();
    for _ in 0..count {
        let short_name = read_string_payload::<_, _, _, _, _, NAME_BYTES>(
            file,
            payload_len,
            &mut payload_read,
            &mut hash,
        )?;
        let chapter_index = read_u16_payload(file, payload_len, &mut payload_read, &mut hash)?;
        let paragraph_in_chapter =
            read_u16_payload(file, payload_len, &mut payload_read, &mut hash)?;
        let word_index = read_u16_payload(file, payload_len, &mut payload_read, &mut hash)?;
        if !out.entries.is_full() {
            let _ = out.entries.push(SdBookDbProgressEntry {
                short_name,
                chapter_index,
                paragraph_in_chapter,
                word_index: word_index.max(1),
            });
        }
    }

    if payload_read != payload_len || checksum_finish(hash) != expected_checksum {
        return Ok(false);
    }
    Ok(true)
}

fn db_file_name_for_short_name(short_name: &str) -> String<BOOK_DB_FILE_NAME_BYTES> {
    let mut out = String::<BOOK_DB_FILE_NAME_BYTES>::new();
    let stem = short_name.split('.').next().unwrap_or(short_name);
    for byte in stem.as_bytes() {
        if !byte.is_ascii_alphanumeric() {
            continue;
        }
        if out.len() >= 8 {
            break;
        }
        let _ = out.push((*byte as char).to_ascii_uppercase());
    }
    if out.is_empty() {
        let _ = out.push_str("BOOK");
    }
    let _ = out.push_str(".RDB");
    out
}

fn copy_string_into<const SRC: usize, const DST: usize>(src: &String<SRC>, dst: &mut String<DST>) {
    dst.clear();
    for ch in src.chars() {
        if dst.push(ch).is_err() {
            break;
        }
    }
}

fn bounded_string<const N: usize>(input: &str) -> String<N> {
    let mut out = String::<N>::new();
    push_str_bounded(&mut out, input);
    out
}

fn push_str_bounded<const N: usize>(out: &mut String<N>, value: &str) {
    for ch in value.chars() {
        if out.push(ch).is_err() {
            break;
        }
    }
}

fn push_u16_ascii<const N: usize>(out: &mut String<N>, mut value: u16) {
    if value == 0 {
        let _ = out.push('0');
        return;
    }

    let mut tmp = [0u8; 5];
    let mut len = 0usize;
    while value > 0 && len < tmp.len() {
        tmp[len] = b'0' + (value % 10) as u8;
        value /= 10;
        len += 1;
    }
    for idx in (0..len).rev() {
        let _ = out.push(tmp[idx] as char);
    }
}

fn checksum_init() -> u32 {
    0x811C9DC5
}

fn checksum_update(mut hash: u32, bytes: &[u8]) -> u32 {
    for byte in bytes {
        hash ^= *byte as u32;
        hash = hash.wrapping_mul(16777619);
    }
    hash
}

fn checksum_finish(hash: u32) -> u32 {
    hash
}

fn write_header<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    kind: u8,
    payload_len: u32,
    checksum: u32,
) -> Result<(), embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let mut header = [0u8; BOOK_DB_HEADER_BYTES];
    header[0..4].copy_from_slice(&BOOK_DB_MAGIC);
    header[4] = BOOK_DB_VERSION;
    header[5] = kind;
    header[6..8].copy_from_slice(&0u16.to_le_bytes());
    header[8..12].copy_from_slice(&payload_len.to_le_bytes());
    header[12..16].copy_from_slice(&checksum.to_le_bytes());
    file.seek_from_start(0)?;
    write_all(file, &header)
}

fn read_header<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    expected_kind: u8,
) -> Result<Option<(u32, u32)>, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    file.seek_from_start(0)?;
    let mut header = [0u8; BOOK_DB_HEADER_BYTES];
    let read_now = file.read(&mut header)?;
    if read_now < BOOK_DB_HEADER_BYTES {
        return Ok(None);
    }
    if header[0..4] != BOOK_DB_MAGIC {
        return Ok(None);
    }
    if header[4] != BOOK_DB_VERSION || header[5] != expected_kind {
        return Ok(None);
    }
    let payload_len = u32::from_le_bytes([header[8], header[9], header[10], header[11]]);
    let checksum = u32::from_le_bytes([header[12], header[13], header[14], header[15]]);
    Ok(Some((payload_len, checksum)))
}

fn write_all<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    bytes: &[u8],
) -> Result<(), embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    file.write(bytes)?;
    Ok(())
}

fn read_exact<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    out: &mut [u8],
) -> Result<bool, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let mut cursor = 0usize;
    while cursor < out.len() {
        let read_now = file.read(&mut out[cursor..])?;
        if read_now == 0 {
            return Ok(false);
        }
        cursor = cursor.saturating_add(read_now);
    }
    Ok(true)
}

fn write_u8_payload<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    payload_len: &mut u32,
    hash: &mut u32,
    value: u8,
) -> Result<(), embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    write_all(file, &[value])?;
    *payload_len = payload_len.saturating_add(1);
    *hash = checksum_update(*hash, &[value]);
    Ok(())
}

fn write_u16_payload<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    payload_len: &mut u32,
    hash: &mut u32,
    value: u16,
) -> Result<(), embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let bytes = value.to_le_bytes();
    write_all(file, &bytes)?;
    *payload_len = payload_len.saturating_add(2);
    *hash = checksum_update(*hash, &bytes);
    Ok(())
}

fn write_u32_payload<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    payload_len: &mut u32,
    hash: &mut u32,
    value: u32,
) -> Result<(), embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let bytes = value.to_le_bytes();
    write_all(file, &bytes)?;
    *payload_len = payload_len.saturating_add(4);
    *hash = checksum_update(*hash, &bytes);
    Ok(())
}

fn write_string_payload<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    payload_len: &mut u32,
    hash: &mut u32,
    value: &str,
) -> Result<(), embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let bytes = value.as_bytes();
    let len = bytes.len().min(u16::MAX as usize) as u16;
    write_u16_payload(file, payload_len, hash, len)?;
    write_all(file, &bytes[..len as usize])?;
    *payload_len = payload_len.saturating_add(len as u32);
    *hash = checksum_update(*hash, &bytes[..len as usize]);
    Ok(())
}

fn read_u8_payload<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    payload_len: u32,
    payload_read: &mut u32,
    hash: &mut u32,
) -> Result<u8, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    if payload_read.saturating_add(1) > payload_len {
        return Ok(0);
    }
    let mut bytes = [0u8; 1];
    if !read_exact(file, &mut bytes)? {
        return Ok(0);
    }
    *payload_read = payload_read.saturating_add(1);
    *hash = checksum_update(*hash, &bytes);
    Ok(bytes[0])
}

fn read_u16_payload<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    payload_len: u32,
    payload_read: &mut u32,
    hash: &mut u32,
) -> Result<u16, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    if payload_read.saturating_add(2) > payload_len {
        return Ok(0);
    }
    let mut bytes = [0u8; 2];
    if !read_exact(file, &mut bytes)? {
        return Ok(0);
    }
    *payload_read = payload_read.saturating_add(2);
    *hash = checksum_update(*hash, &bytes);
    Ok(u16::from_le_bytes(bytes))
}

fn read_u32_payload<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    payload_len: u32,
    payload_read: &mut u32,
    hash: &mut u32,
) -> Result<u32, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    if payload_read.saturating_add(4) > payload_len {
        return Ok(0);
    }
    let mut bytes = [0u8; 4];
    if !read_exact(file, &mut bytes)? {
        return Ok(0);
    }
    *payload_read = payload_read.saturating_add(4);
    *hash = checksum_update(*hash, &bytes);
    Ok(u32::from_le_bytes(bytes))
}

fn read_string_payload<
    D,
    T,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const MAX_VOLUMES: usize,
    const OUT_BYTES: usize,
>(
    file: &mut embedded_sdmmc::File<'_, D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    payload_len: u32,
    payload_read: &mut u32,
    hash: &mut u32,
) -> Result<String<OUT_BYTES>, embedded_sdmmc::Error<D::Error>>
where
    D: embedded_sdmmc::BlockDevice,
    T: TimeSource,
{
    let len = read_u16_payload(file, payload_len, payload_read, hash)? as usize;
    if payload_read.saturating_add(len as u32) > payload_len {
        return Ok(String::new());
    }
    let mut bytes = [0u8; 256];
    let mut remaining = len;
    let mut out = String::<OUT_BYTES>::new();
    while remaining > 0 {
        let chunk = remaining.min(bytes.len());
        if !read_exact(file, &mut bytes[..chunk])? {
            break;
        }
        *payload_read = payload_read.saturating_add(chunk as u32);
        *hash = checksum_update(*hash, &bytes[..chunk]);
        let str_chunk = str::from_utf8(&bytes[..chunk]).unwrap_or("");
        for ch in str_chunk.chars() {
            if out.push(ch).is_err() {
                break;
            }
        }
        remaining -= chunk;
    }
    Ok(out)
}
