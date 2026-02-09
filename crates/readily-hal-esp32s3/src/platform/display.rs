use embedded_hal::{delay::DelayNs, digital::OutputPin, spi::SpiBus};
use ls027b7dh01::{
    FrameBuffer,
    protocol::{self, HEIGHT, LINE_BYTES},
};

const CS_SETUP_NS: u32 = 3_000;
const CS_HOLD_NS: u32 = 1_000;

const CMD_WRITE: u8 = 0x80;
const VCOM_BIT: u8 = 0x40;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum DisplayError<SpiErr, DispErr, EmdErr, CsErr> {
    Spi(SpiErr),
    Disp(DispErr),
    Emd(EmdErr),
    Cs(CsErr),
    Protocol,
}

pub type SharpDisplayResult<SpiErr, DispErr, EmdErr, CsErr> =
    Result<(), DisplayError<SpiErr, DispErr, EmdErr, CsErr>>;

/// Minimal board-level display adapter for LS027B7DH01.
#[derive(Debug)]
pub struct SharpDisplay<SPI, DISP, EMD, CS> {
    spi: SPI,
    disp: DISP,
    emd: EMD,
    cs: CS,
    vcom_high: bool,
}

impl<SPI, DISP, EMD, CS> SharpDisplay<SPI, DISP, EMD, CS>
where
    SPI: SpiBus<u8>,
    DISP: OutputPin,
    EMD: OutputPin,
    CS: OutputPin,
{
    pub fn new(spi: SPI, disp: DISP, emd: EMD, cs: CS) -> Self {
        Self {
            spi,
            disp,
            emd,
            cs,
            vcom_high: false,
        }
    }

    /// Puts the panel in serial M1 mode and enables display output.
    pub fn initialize<D>(
        &mut self,
        delay: &mut D,
    ) -> SharpDisplayResult<SPI::Error, DISP::Error, EMD::Error, CS::Error>
    where
        D: DelayNs,
    {
        self.disp.set_high().map_err(DisplayError::Disp)?;
        self.emd.set_low().map_err(DisplayError::Emd)?;
        self.cs.set_low().map_err(DisplayError::Cs)?;

        delay.delay_us(60);
        Ok(())
    }

    /// Send all-clear command and hold CS as required.
    pub fn clear_all<D>(
        &mut self,
        delay: &mut D,
    ) -> SharpDisplayResult<SPI::Error, DISP::Error, EMD::Error, CS::Error>
    where
        D: DelayNs,
    {
        self.vcom_high = !self.vcom_high;

        self.cs.set_high().map_err(DisplayError::Cs)?;
        delay.delay_ns(CS_SETUP_NS);

        let packet = protocol::build_clear_packet(self.vcom_high);
        self.spi.write(&packet).map_err(DisplayError::Spi)?;
        self.spi.flush().map_err(DisplayError::Spi)?;

        delay.delay_ns(220_000);
        self.cs.set_low().map_err(DisplayError::Cs)?;

        Ok(())
    }

    /// Flush a full framebuffer in a single CS-high transaction.
    pub fn flush_frame<D>(
        &mut self,
        frame: &FrameBuffer,
        delay: &mut D,
    ) -> SharpDisplayResult<SPI::Error, DISP::Error, EMD::Error, CS::Error>
    where
        D: DelayNs,
    {
        self.vcom_high = !self.vcom_high;

        self.cs.set_high().map_err(DisplayError::Cs)?;
        delay.delay_ns(CS_SETUP_NS);

        let command = CMD_WRITE | if self.vcom_high { VCOM_BIT } else { 0x00 };
        self.spi.write(&[command]).map_err(DisplayError::Spi)?;

        // [address][50 data bytes][dummy]
        let mut packet = [0u8; LINE_BYTES + 2];
        packet[LINE_BYTES + 1] = 0x00;

        let bytes = frame.bytes();

        for line in 1..=HEIGHT as u16 {
            packet[0] = protocol::encode_line_address(line).ok_or(DisplayError::Protocol)?;

            let start = (line as usize - 1) * LINE_BYTES;
            let end = start + LINE_BYTES;
            packet[1..1 + LINE_BYTES].copy_from_slice(&bytes[start..end]);

            self.spi.write(&packet).map_err(DisplayError::Spi)?;
        }

        // Frame trailer byte.
        self.spi.write(&[0x00]).map_err(DisplayError::Spi)?;
        self.spi.flush().map_err(DisplayError::Spi)?;

        delay.delay_ns(CS_HOLD_NS);
        self.cs.set_low().map_err(DisplayError::Cs)?;

        Ok(())
    }
}
