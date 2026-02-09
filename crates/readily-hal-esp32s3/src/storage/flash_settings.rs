use embedded_storage::{ReadStorage, Storage};
use esp_bootloader_esp_idf::partitions::{
    DataPartitionSubType, PARTITION_TABLE_MAX_LEN, PartitionType, read_partition_table,
};
use esp_rom_sys::rom::spiflash::{
    ESP_ROM_SPIFLASH_RESULT_OK, esp_rom_spiflash_erase_sector, esp_rom_spiflash_read,
    esp_rom_spiflash_unlock, esp_rom_spiflash_write,
};
use readily_core::{
    render::{FontFamily, FontSize, VisualStyle},
    settings::{PersistedSettings, SettingsStore},
};

const FLASH_SECTOR_SIZE: u32 = 4096;
const DEFAULT_FLASH_CAPACITY_BYTES: usize = 16 * 1024 * 1024;

const SETTINGS_MAGIC: u32 = 0x3153_4452; // "RDS1"
const SETTINGS_VERSION: u8 = 1;
const SETTINGS_RECORD_LEN: usize = 16;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum FlashSettingsError {
    PartitionTable,
    SettingsPartitionMissing,
    PartitionTooSmall,
    FlashOpFailed(i32),
    Corrupted,
    Unsupported,
}

#[derive(Debug)]
struct RawFlash;

impl RawFlash {
    fn new() -> Result<Self, FlashSettingsError> {
        let rc = unsafe { esp_rom_spiflash_unlock() };
        if rc != ESP_ROM_SPIFLASH_RESULT_OK {
            return Err(FlashSettingsError::FlashOpFailed(rc));
        }
        Ok(Self)
    }

    fn erase_sector(&mut self, sector_addr: u32) -> Result<(), FlashSettingsError> {
        if !sector_addr.is_multiple_of(FLASH_SECTOR_SIZE) {
            return Err(FlashSettingsError::Unsupported);
        }

        let sector = sector_addr / FLASH_SECTOR_SIZE;
        let rc = unsafe { esp_rom_spiflash_erase_sector(sector) };
        if rc != ESP_ROM_SPIFLASH_RESULT_OK {
            return Err(FlashSettingsError::FlashOpFailed(rc));
        }
        Ok(())
    }

    fn read_word(&mut self, addr: u32) -> Result<u32, FlashSettingsError> {
        if !addr.is_multiple_of(4) {
            return Err(FlashSettingsError::Unsupported);
        }

        let mut word = 0u32;
        let rc = unsafe { esp_rom_spiflash_read(addr, &mut word as *mut u32 as *const u32, 4) };
        if rc != ESP_ROM_SPIFLASH_RESULT_OK {
            return Err(FlashSettingsError::FlashOpFailed(rc));
        }
        Ok(word)
    }

    fn write_word(&mut self, addr: u32, word: u32) -> Result<(), FlashSettingsError> {
        if !addr.is_multiple_of(4) {
            return Err(FlashSettingsError::Unsupported);
        }

        let rc = unsafe { esp_rom_spiflash_write(addr, &word as *const u32, 4) };
        if rc != ESP_ROM_SPIFLASH_RESULT_OK {
            return Err(FlashSettingsError::FlashOpFailed(rc));
        }
        Ok(())
    }

    fn read_bytes(&mut self, addr: u32, out: &mut [u8]) -> Result<(), FlashSettingsError> {
        if out.is_empty() {
            return Ok(());
        }

        let mut written = 0usize;
        let start = addr & !0b11;
        let end = (addr + out.len() as u32 + 3) & !0b11;

        for word_addr in (start..end).step_by(4) {
            let word = self.read_word(word_addr)?;
            let bytes = word.to_le_bytes();

            let base = word_addr as i64 - addr as i64;
            for (i, b) in bytes.iter().enumerate() {
                let dst = base + i as i64;
                if dst < 0 {
                    continue;
                }
                let dst = dst as usize;
                if dst >= out.len() {
                    break;
                }
                out[dst] = *b;
                written += 1;
            }
        }

        if written == out.len() {
            Ok(())
        } else {
            Err(FlashSettingsError::Corrupted)
        }
    }

    fn write_erased_bytes(&mut self, addr: u32, data: &[u8]) -> Result<(), FlashSettingsError> {
        if data.is_empty() {
            return Ok(());
        }

        let start = addr & !0b11;
        let end = (addr + data.len() as u32 + 3) & !0b11;

        for word_addr in (start..end).step_by(4) {
            let mut bytes = [0xFFu8; 4];
            let base = word_addr as i64 - addr as i64;
            for (i, slot) in bytes.iter_mut().enumerate() {
                let src = base + i as i64;
                if src < 0 {
                    continue;
                }
                let src = src as usize;
                if src >= data.len() {
                    break;
                }
                *slot = data[src];
            }

            self.write_word(word_addr, u32::from_le_bytes(bytes))?;
        }

        Ok(())
    }
}

impl ReadStorage for RawFlash {
    type Error = FlashSettingsError;

    fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
        self.read_bytes(offset, bytes)
    }

    fn capacity(&self) -> usize {
        DEFAULT_FLASH_CAPACITY_BYTES
    }
}

impl Storage for RawFlash {
    fn write(&mut self, _offset: u32, _bytes: &[u8]) -> Result<(), Self::Error> {
        Err(FlashSettingsError::Unsupported)
    }
}

#[derive(Debug)]
pub struct FlashSettingsStore {
    flash: RawFlash,
    settings_sector_addr: u32,
}

impl FlashSettingsStore {
    pub fn new() -> Result<Self, FlashSettingsError> {
        let mut flash = RawFlash::new()?;

        let mut table_buf = [0u8; PARTITION_TABLE_MAX_LEN];
        let table = read_partition_table(&mut flash, &mut table_buf)
            .map_err(|_| FlashSettingsError::PartitionTable)?;

        let mut best_data_undefined: Option<(u32, u32)> = None;
        let mut fallback_nvs: Option<(u32, u32)> = None;

        for entry in table.iter() {
            if entry.is_read_only() {
                continue;
            }

            if entry.len() < FLASH_SECTOR_SIZE {
                continue;
            }

            match entry.partition_type() {
                PartitionType::Data(DataPartitionSubType::Undefined) => {
                    best_data_undefined = Some((entry.offset(), entry.len()));
                    break;
                }
                PartitionType::Data(DataPartitionSubType::Nvs) => {
                    if fallback_nvs.is_none() {
                        fallback_nvs = Some((entry.offset(), entry.len()));
                    }
                }
                _ => {}
            }
        }

        let (offset, len) = best_data_undefined
            .or(fallback_nvs)
            .ok_or(FlashSettingsError::SettingsPartitionMissing)?;

        if len < FLASH_SECTOR_SIZE {
            return Err(FlashSettingsError::PartitionTooSmall);
        }

        let settings_sector_addr = offset + len - FLASH_SECTOR_SIZE;
        Ok(Self {
            flash,
            settings_sector_addr,
        })
    }
}

impl SettingsStore for FlashSettingsStore {
    type Error = FlashSettingsError;

    fn load(&mut self) -> Result<Option<PersistedSettings>, Self::Error> {
        let mut buf = [0u8; SETTINGS_RECORD_LEN];
        self.flash.read_bytes(self.settings_sector_addr, &mut buf)?;

        if buf.iter().all(|b| *b == 0xFF) {
            return Ok(None);
        }

        let magic = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        if magic != SETTINGS_MAGIC {
            return Ok(None);
        }

        let version = buf[4];
        if version != SETTINGS_VERSION {
            return Ok(None);
        }

        let expected_checksum = u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]);
        let checksum = checksum32(&buf[..12]);
        if checksum != expected_checksum {
            return Err(FlashSettingsError::Corrupted);
        }

        let font_family = match buf[5] {
            0 => FontFamily::Serif,
            1 => FontFamily::Pixel,
            _ => return Err(FlashSettingsError::Corrupted),
        };

        let font_size = match buf[6] {
            0 => FontSize::Small,
            1 => FontSize::Medium,
            2 => FontSize::Large,
            _ => return Err(FlashSettingsError::Corrupted),
        };

        let inverted = (buf[7] & 0x01) != 0;
        let wpm = u16::from_le_bytes([buf[8], buf[9]]);

        Ok(Some(PersistedSettings::new(
            wpm,
            VisualStyle {
                font_family,
                font_size,
                inverted,
            },
        )))
    }

    fn save(&mut self, settings: &PersistedSettings) -> Result<(), Self::Error> {
        let mut buf = [0xFFu8; SETTINGS_RECORD_LEN];
        buf[0..4].copy_from_slice(&SETTINGS_MAGIC.to_le_bytes());
        buf[4] = SETTINGS_VERSION;
        buf[5] = match settings.style.font_family {
            FontFamily::Serif => 0,
            FontFamily::Pixel => 1,
        };
        buf[6] = match settings.style.font_size {
            FontSize::Small => 0,
            FontSize::Medium => 1,
            FontSize::Large => 2,
        };
        buf[7] = if settings.style.inverted { 1 } else { 0 };
        buf[8..10].copy_from_slice(&settings.wpm.to_le_bytes());
        buf[10] = 0;
        buf[11] = 0;
        let checksum = checksum32(&buf[..12]);
        buf[12..16].copy_from_slice(&checksum.to_le_bytes());

        self.flash.erase_sector(self.settings_sector_addr)?;
        self.flash
            .write_erased_bytes(self.settings_sector_addr, &buf)?;
        Ok(())
    }
}

fn checksum32(bytes: &[u8]) -> u32 {
    let mut hash = 0x811C9DC5u32;
    for b in bytes {
        hash ^= *b as u32;
        hash = hash.wrapping_mul(16777619);
    }
    hash
}
