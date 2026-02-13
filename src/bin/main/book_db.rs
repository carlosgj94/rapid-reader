use core::fmt::Debug;

use embedded_hal::{delay::DelayNs, digital::OutputPin, spi::SpiBus};
use heapless::{String as HeaplessString, Vec as HeaplessVec};
use log::info;
use readily_core::{
    content::{
        TextCatalog,
        sd_catalog::{SD_CATALOG_TITLE_BYTES, SdCatalogSource},
    },
    settings::ResumeState,
};
use readily_hal_esp32s3::storage::sd_spi::{
    SdBookDbManifest, SdBookDbManifestEntry, SdBookDbProgress, SdProbeError,
    probe_and_load_book_db_manifest, probe_and_load_book_db_progress,
    probe_and_save_book_db_manifest, probe_and_upsert_book_db_progress,
};

use super::{
    SD_BOOKS_DIR, SD_SCAN_MAX_EPUBS, SD_SCAN_NAME_BYTES, SD_TEXT_PATH_BYTES, SdBookStreamState,
};

const DB_TITLE_BYTES: usize = SD_SCAN_NAME_BYTES;

type BookManifest = SdBookDbManifest<
    SD_SCAN_MAX_EPUBS,
    SD_SCAN_NAME_BYTES,
    DB_TITLE_BYTES,
    SD_TEXT_PATH_BYTES,
    SD_CATALOG_TITLE_BYTES,
>;
type ManifestEntry = SdBookDbManifestEntry<
    SD_SCAN_NAME_BYTES,
    DB_TITLE_BYTES,
    SD_TEXT_PATH_BYTES,
    SD_CATALOG_TITLE_BYTES,
>;
type ProgressState = SdBookDbProgress<SD_SCAN_MAX_EPUBS, SD_SCAN_NAME_BYTES>;

pub(super) fn try_load_catalog_from_db<BUS, CS, DELAY>(
    content: &mut SdCatalogSource,
    sd_stream_states: &mut HeaplessVec<SdBookStreamState, SD_SCAN_MAX_EPUBS>,
    sd_spi: &mut BUS,
    sd_cs: &mut CS,
    sd_delay: &mut DELAY,
) -> Result<bool, SdProbeError<BUS::Error, CS::Error>>
where
    BUS: SpiBus<u8>,
    CS: OutputPin,
    DELAY: DelayNs,
    BUS::Error: Debug,
    CS::Error: Debug,
{
    let Some(manifest) = probe_and_load_book_db_manifest::<
        _,
        _,
        _,
        SD_SCAN_MAX_EPUBS,
        SD_SCAN_NAME_BYTES,
        DB_TITLE_BYTES,
        SD_TEXT_PATH_BYTES,
        SD_CATALOG_TITLE_BYTES,
    >(sd_spi, sd_cs, sd_delay, SD_BOOKS_DIR)?
    else {
        return Ok(false);
    };

    if manifest
        .entries
        .iter()
        .any(|entry| entry.first_resource.as_str().trim().is_empty())
    {
        info!("sd-db: manifest invalid (missing first_resource); fallback to SD scan");
        return Ok(false);
    }

    let load = content.set_catalog_entries_from_iter(
        manifest
            .entries
            .iter()
            .map(|entry| (entry.display_title.as_str(), entry.has_cover)),
    );
    if load.loaded == 0 {
        return Ok(false);
    }

    sd_stream_states.clear();
    for (index, entry) in manifest.entries.iter().enumerate() {
        let idx = index as u16;
        let _ = content.set_catalog_text_chunk_from_bytes(
            idx,
            b"",
            false,
            entry.first_resource.as_str(),
        );
        let _ = content.set_catalog_stream_chapter_metadata(
            idx,
            0,
            entry.chapter_total.max(1),
            Some(entry.first_chapter_label.as_str()),
        );

        let mut state = SdBookStreamState {
            short_name: HeaplessString::new(),
            text_resource: HeaplessString::new(),
            next_offset: entry.first_offset,
            end_of_resource: false,
            ready: !entry.first_resource.is_empty(),
        };
        copy_heapless_string(&entry.short_name, &mut state.short_name);
        copy_heapless_string(&entry.first_resource, &mut state.text_resource);
        if sd_stream_states.push(state).is_err() {
            break;
        }
    }

    info!(
        "sd-db: loaded manifest books={} stream_states={}",
        load.loaded,
        sd_stream_states.len()
    );
    Ok(true)
}

pub(super) fn build_book_db_from_runtime<BUS, CS, DELAY>(
    content: &SdCatalogSource,
    sd_stream_states: &HeaplessVec<SdBookStreamState, SD_SCAN_MAX_EPUBS>,
    sd_spi: &mut BUS,
    sd_cs: &mut CS,
    sd_delay: &mut DELAY,
) where
    BUS: SpiBus<u8>,
    CS: OutputPin,
    DELAY: DelayNs,
    BUS::Error: Debug,
    CS::Error: Debug,
{
    let mut manifest = BookManifest {
        entries: HeaplessVec::new(),
    };

    for (index, state) in sd_stream_states.iter().enumerate() {
        let idx = index as u16;
        let title = content.title_at(idx).unwrap_or("Untitled");
        let has_cover = content.has_cover_at(idx);
        let mut entry = ManifestEntry {
            short_name: HeaplessString::new(),
            display_title: HeaplessString::new(),
            has_cover,
            chapter_total: 1,
            first_resource: HeaplessString::new(),
            first_offset: 0,
            first_chapter_label: HeaplessString::new(),
        };
        copy_heapless_string(&state.short_name, &mut entry.short_name);
        copy_str_into(title, &mut entry.display_title);
        copy_heapless_string(&state.text_resource, &mut entry.first_resource);
        let _ = entry.first_chapter_label.push_str("Section");
        let _ = manifest.entries.push(entry);
    }

    if manifest.entries.is_empty() {
        info!("sd-db: no manifest entries to save");
        return;
    }

    match probe_and_save_book_db_manifest(sd_spi, sd_cs, sd_delay, SD_BOOKS_DIR, &manifest) {
        Ok(()) => info!("sd-db: manifest saved entries={}", manifest.entries.len()),
        Err(err) => log_probe_error("sd-db: manifest save failed", &err),
    }
}

pub(super) fn load_resume_from_db<BUS, CS, DELAY>(
    sd_stream_states: &HeaplessVec<SdBookStreamState, SD_SCAN_MAX_EPUBS>,
    sd_spi: &mut BUS,
    sd_cs: &mut CS,
    sd_delay: &mut DELAY,
) -> Option<ResumeState>
where
    BUS: SpiBus<u8>,
    CS: OutputPin,
    DELAY: DelayNs,
    BUS::Error: Debug,
    CS::Error: Debug,
{
    let progress =
        match probe_and_load_book_db_progress::<_, _, _, SD_SCAN_MAX_EPUBS, SD_SCAN_NAME_BYTES>(
            sd_spi,
            sd_cs,
            sd_delay,
            SD_BOOKS_DIR,
        ) {
            Ok(Some(progress)) => progress,
            Ok(None) => return None,
            Err(err) => {
                log_probe_error("sd-db: load progress failed", &err);
                return None;
            }
        };

    map_progress_to_resume(sd_stream_states, &progress)
}

pub(super) fn save_resume_to_db<BUS, CS, DELAY>(
    resume: ResumeState,
    sd_stream_states: &HeaplessVec<SdBookStreamState, SD_SCAN_MAX_EPUBS>,
    sd_spi: &mut BUS,
    sd_cs: &mut CS,
    sd_delay: &mut DELAY,
) -> bool
where
    BUS: SpiBus<u8>,
    CS: OutputPin,
    DELAY: DelayNs,
    BUS::Error: Debug,
    CS::Error: Debug,
{
    let Some(stream_state) = sd_stream_states.get(resume.selected_book as usize) else {
        return false;
    };

    if let Err(err) =
        probe_and_upsert_book_db_progress::<_, _, _, SD_SCAN_MAX_EPUBS, SD_SCAN_NAME_BYTES>(
            sd_spi,
            sd_cs,
            sd_delay,
            SD_BOOKS_DIR,
            stream_state.short_name.as_str(),
            resume,
        )
    {
        log_probe_error("sd-db: save progress failed", &err);
        return false;
    }
    true
}

fn map_progress_to_resume(
    sd_stream_states: &HeaplessVec<SdBookStreamState, SD_SCAN_MAX_EPUBS>,
    progress: &ProgressState,
) -> Option<ResumeState> {
    if progress.entries.is_empty() {
        return None;
    }

    let preferred = if progress.last_open_short_name.is_empty() {
        None
    } else {
        progress.entries.iter().find(|entry| {
            entry
                .short_name
                .as_str()
                .eq_ignore_ascii_case(progress.last_open_short_name.as_str())
        })
    };
    let entry = preferred.unwrap_or_else(|| &progress.entries[0]);

    let selected_book = sd_stream_states
        .iter()
        .enumerate()
        .find_map(|(index, state)| {
            if state
                .short_name
                .as_str()
                .eq_ignore_ascii_case(entry.short_name.as_str())
            {
                Some(index as u16)
            } else {
                None
            }
        })?;

    Some(ResumeState {
        selected_book,
        chapter_index: entry.chapter_index,
        paragraph_in_chapter: entry.paragraph_in_chapter,
        word_index: entry.word_index.max(1),
    })
}

fn copy_heapless_string<const SRC: usize, const DST: usize>(
    src: &HeaplessString<SRC>,
    dst: &mut HeaplessString<DST>,
) {
    dst.clear();
    for ch in src.chars() {
        if dst.push(ch).is_err() {
            break;
        }
    }
}

fn copy_str_into<const DST: usize>(src: &str, dst: &mut HeaplessString<DST>) {
    dst.clear();
    for ch in src.chars() {
        if dst.push(ch).is_err() {
            break;
        }
    }
}

fn log_probe_error<BE, CE>(prefix: &str, err: &SdProbeError<BE, CE>)
where
    BE: Debug,
    CE: Debug,
{
    match err {
        SdProbeError::ChipSelect(e) => info!("{prefix} (chip-select): {:?}", e),
        SdProbeError::Spi(e) => info!("{prefix} (spi): {:?}", e),
        SdProbeError::Card(e) => info!("{prefix} (card): {:?}", e),
        SdProbeError::Filesystem(e) => info!("{prefix} (fs): {:?}", e),
    }
}
