#![cfg_attr(not(test), no_std)]

//! LS027B7DH01 (2.7" 400x240 Sharp Memory LCD) driver primitives.

mod framebuffer;
pub mod protocol;

#[cfg(feature = "embedded-graphics")]
mod graphics;

pub use framebuffer::FrameBuffer;

use core::convert::TryFrom;

use embedded_hal::{
    digital::OutputPin,
    spi::{Operation, SpiDevice},
};

/// LCD inversion strategy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InversionMode {
    /// COM inversion is driven via dedicated `EXTCOMIN` pin toggling.
    ExtComInPin,
}

/// Driver configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Config {
    /// Expected SPI clock in Hz (documented for board glue).
    pub spi_hz: u32,
    /// EXTCOMIN target frequency in Hz.
    pub extcomin_hz: u8,
    /// Inversion strategy.
    pub inversion: InversionMode,
    /// M1 level embedded in SPI command words.
    pub m1_high: bool,
    /// Additional CS-active delay used for clear command hold time.
    pub clear_hold_ns: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            spi_hz: 1_000_000,
            extcomin_hz: 1,
            inversion: InversionMode::ExtComInPin,
            m1_high: false,
            clear_hold_ns: 220_000,
        }
    }
}

/// Driver errors.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Error<SpiErr, DispErr, ExtErr> {
    /// SPI transaction failed.
    Spi(SpiErr),
    /// DISP pin operation failed.
    Disp(DispErr),
    /// EXTCOMIN pin operation failed.
    ExtCom(ExtErr),
    /// Input parameters are outside supported bounds.
    InvalidInput,
}

pub type DriverResult<SpiErr, DispErr, ExtErr> = Result<(), Error<SpiErr, DispErr, ExtErr>>;

/// LS027B7DH01 driver.
#[derive(Debug)]
pub struct Ls027<SPI, DISP, EXTCOM> {
    spi: SPI,
    disp: DISP,
    extcom: EXTCOM,
    config: Config,
    extcom_high: bool,
}

impl<SPI, DISP, EXTCOM> Ls027<SPI, DISP, EXTCOM>
where
    SPI: SpiDevice<u8>,
    DISP: OutputPin,
    EXTCOM: OutputPin,
{
    /// Creates a new driver instance.
    pub fn new(spi: SPI, disp: DISP, extcom: EXTCOM, config: Config) -> Self {
        Self {
            spi,
            disp,
            extcom,
            config,
            extcom_high: false,
        }
    }

    /// Returns current configuration.
    pub fn config(&self) -> Config {
        self.config
    }

    /// Releases owned bus and pins.
    pub fn release(self) -> (SPI, DISP, EXTCOM) {
        (self.spi, self.disp, self.extcom)
    }

    /// Drives `DISP` high.
    pub fn enable_display(&mut self) -> DriverResult<SPI::Error, DISP::Error, EXTCOM::Error> {
        self.disp.set_high().map_err(Error::Disp)
    }

    /// Drives `DISP` low.
    pub fn disable_display(&mut self) -> DriverResult<SPI::Error, DISP::Error, EXTCOM::Error> {
        self.disp.set_low().map_err(Error::Disp)
    }

    /// Toggles the EXTCOMIN pin level.
    pub fn toggle_extcomin(&mut self) -> DriverResult<SPI::Error, DISP::Error, EXTCOM::Error> {
        self.extcom_high = !self.extcom_high;

        if self.extcom_high {
            self.extcom.set_high().map_err(Error::ExtCom)
        } else {
            self.extcom.set_low().map_err(Error::ExtCom)
        }
    }

    /// Issues all-clear command.
    pub fn clear_all(&mut self) -> DriverResult<SPI::Error, DISP::Error, EXTCOM::Error> {
        let packet = protocol::build_clear_packet(self.config.m1_high);
        let mut ops = [
            Operation::Write(&packet),
            Operation::DelayNs(self.config.clear_hold_ns),
        ];
        self.spi.transaction(&mut ops).map_err(Error::Spi)
    }

    /// Writes one line (1..=240).
    pub fn write_line(
        &mut self,
        line: u16,
        data: &[u8; protocol::LINE_BYTES],
    ) -> DriverResult<SPI::Error, DISP::Error, EXTCOM::Error> {
        let packet = protocol::build_write_line_packet(line, data, self.config.m1_high)
            .ok_or(Error::InvalidInput)?;

        self.spi.write(&packet).map_err(Error::Spi)
    }

    /// Flushes a full framebuffer.
    pub fn flush_full(
        &mut self,
        buffer: &[u8; protocol::BUFFER_SIZE],
    ) -> DriverResult<SPI::Error, DISP::Error, EXTCOM::Error> {
        for (i, line) in buffer.chunks_exact(protocol::LINE_BYTES).enumerate() {
            let line =
                <&[u8; protocol::LINE_BYTES]>::try_from(line).map_err(|_| Error::InvalidInput)?;
            self.write_line((i + 1) as u16, line)?;
        }

        Ok(())
    }
}
