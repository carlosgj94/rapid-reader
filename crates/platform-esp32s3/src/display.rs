use embedded_hal::{delay::DelayNs, digital::OutputPin, spi::SpiBus};
use ls027b7dh01::{
    DirtyRows, FrameBuffer,
    protocol::{self, HEIGHT, LINE_BYTES},
};

const CS_SETUP_NS: u32 = 3_000;
const CS_HOLD_NS: u32 = 1_000;
const CLEAR_HOLD_NS: u32 = 220_000;
pub const HEARTBEAT_INTERVAL_MS: u64 = 500;
const FULL_FRAME_BYTES: usize = 1 + (HEIGHT * (LINE_BYTES + 2)) + 1;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum DisplayError<SpiErr, DispErr, EmdErr, CsErr> {
    Spi(SpiErr),
    Disp(DispErr),
    Emd(EmdErr),
    Cs(CsErr),
    Protocol,
}

pub type DisplayResult<SpiErr, DispErr, EmdErr, CsErr> =
    Result<(), DisplayError<SpiErr, DispErr, EmdErr, CsErr>>;
pub type DisplayPresentResult<SpiErr, DispErr, EmdErr, CsErr> =
    Result<DisplayPresentStats, DisplayError<SpiErr, DispErr, EmdErr, CsErr>>;

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub struct DisplayPresentStats {
    pub dirty_rows: u16,
    pub bytes_sent: usize,
    pub full_refresh: bool,
}

pub fn diff_dirty_rows(committed: &FrameBuffer, working: &FrameBuffer) -> DirtyRows {
    let mut dirty_rows = DirtyRows::new();

    for row in 0..HEIGHT {
        if committed.row(row) != working.row(row) {
            let _ = dirty_rows.mark_row(row);
        }
    }

    dirty_rows
}

pub struct PlatformDisplay<SPI, DISP, EMD, CS> {
    spi: SPI,
    disp: DISP,
    emd: EMD,
    cs: CS,
    vcom_high: bool,
}

impl<SPI, DISP, EMD, CS> PlatformDisplay<SPI, DISP, EMD, CS>
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

    pub fn initialize<D>(
        &mut self,
        delay: &mut D,
    ) -> DisplayResult<SPI::Error, DISP::Error, EMD::Error, CS::Error>
    where
        D: DelayNs,
    {
        self.disp.set_high().map_err(DisplayError::Disp)?;
        self.emd.set_low().map_err(DisplayError::Emd)?;
        self.cs.set_low().map_err(DisplayError::Cs)?;
        delay.delay_us(60);
        Ok(())
    }

    pub fn clear_all<D>(
        &mut self,
        delay: &mut D,
    ) -> DisplayResult<SPI::Error, DISP::Error, EMD::Error, CS::Error>
    where
        D: DelayNs,
    {
        self.vcom_high = !self.vcom_high;
        self.cs.set_high().map_err(DisplayError::Cs)?;
        delay.delay_ns(CS_SETUP_NS);

        let packet = protocol::build_clear_packet(self.vcom_high);
        self.spi.write(&packet).map_err(DisplayError::Spi)?;
        self.spi.flush().map_err(DisplayError::Spi)?;

        delay.delay_ns(CLEAR_HOLD_NS);
        self.cs.set_low().map_err(DisplayError::Cs)?;
        Ok(())
    }

    pub fn heartbeat<D>(
        &mut self,
        delay: &mut D,
    ) -> DisplayResult<SPI::Error, DISP::Error, EMD::Error, CS::Error>
    where
        D: DelayNs,
    {
        self.vcom_high = !self.vcom_high;
        self.cs.set_high().map_err(DisplayError::Cs)?;
        delay.delay_ns(CS_SETUP_NS);

        let packet = protocol::build_display_mode_packet(self.vcom_high);
        self.spi.write(&packet).map_err(DisplayError::Spi)?;
        self.spi.flush().map_err(DisplayError::Spi)?;

        delay.delay_ns(CS_HOLD_NS);
        self.cs.set_low().map_err(DisplayError::Cs)?;
        Ok(())
    }

    pub fn present<D>(
        &mut self,
        committed: &mut FrameBuffer,
        working: &FrameBuffer,
        dirty_rows: &DirtyRows,
        delay: &mut D,
    ) -> DisplayPresentResult<SPI::Error, DISP::Error, EMD::Error, CS::Error>
    where
        D: DelayNs,
    {
        if dirty_rows.is_empty() {
            return Ok(DisplayPresentStats::default());
        }

        let dirty_count = dirty_rows.count();
        if dirty_rows.is_full_height() {
            self.flush_full_frame(working, delay)?;
            committed.copy_dirty_rows_from(working, dirty_rows);
            return Ok(DisplayPresentStats {
                dirty_rows: dirty_count,
                bytes_sent: FULL_FRAME_BYTES,
                full_refresh: true,
            });
        }

        self.flush_dirty_rows(working, dirty_rows, delay)?;
        committed.copy_dirty_rows_from(working, dirty_rows);
        Ok(DisplayPresentStats {
            dirty_rows: dirty_count,
            bytes_sent: 1 + dirty_count as usize * (LINE_BYTES + 2) + 1,
            full_refresh: false,
        })
    }

    pub fn disable_output(
        &mut self,
    ) -> DisplayResult<SPI::Error, DISP::Error, EMD::Error, CS::Error> {
        self.cs.set_low().map_err(DisplayError::Cs)?;
        self.emd.set_low().map_err(DisplayError::Emd)?;
        self.disp.set_low().map_err(DisplayError::Disp)?;
        Ok(())
    }

    pub fn enter_low_power<D>(
        &mut self,
        delay: &mut D,
    ) -> DisplayResult<SPI::Error, DISP::Error, EMD::Error, CS::Error>
    where
        D: DelayNs,
    {
        self.clear_all(delay)?;
        self.disable_output()
    }

    fn flush_full_frame<D>(
        &mut self,
        frame: &FrameBuffer,
        delay: &mut D,
    ) -> DisplayResult<SPI::Error, DISP::Error, EMD::Error, CS::Error>
    where
        D: DelayNs,
    {
        self.vcom_high = !self.vcom_high;
        self.cs.set_high().map_err(DisplayError::Cs)?;
        delay.delay_ns(CS_SETUP_NS);

        self.spi
            .write(&[protocol::build_write_command(self.vcom_high)])
            .map_err(DisplayError::Spi)?;

        let mut packet = [0u8; LINE_BYTES + 2];
        packet[LINE_BYTES + 1] = 0x00;

        for line in 1..=HEIGHT as u16 {
            packet[0] = protocol::encode_line_address(line).ok_or(DisplayError::Protocol)?;
            packet[1..1 + LINE_BYTES]
                .copy_from_slice(frame.row(line as usize - 1).ok_or(DisplayError::Protocol)?);
            self.spi.write(&packet).map_err(DisplayError::Spi)?;
        }

        self.finish_write_transaction(delay)
    }

    fn flush_dirty_rows<D>(
        &mut self,
        frame: &FrameBuffer,
        dirty_rows: &DirtyRows,
        delay: &mut D,
    ) -> DisplayResult<SPI::Error, DISP::Error, EMD::Error, CS::Error>
    where
        D: DelayNs,
    {
        self.vcom_high = !self.vcom_high;
        self.cs.set_high().map_err(DisplayError::Cs)?;
        delay.delay_ns(CS_SETUP_NS);

        self.spi
            .write(&[protocol::build_write_command(self.vcom_high)])
            .map_err(DisplayError::Spi)?;

        let mut packet = [0u8; LINE_BYTES + 2];
        packet[LINE_BYTES + 1] = 0x00;

        for row in dirty_rows.iter() {
            packet[0] =
                protocol::encode_line_address(row as u16 + 1).ok_or(DisplayError::Protocol)?;
            packet[1..1 + LINE_BYTES]
                .copy_from_slice(frame.row(row).ok_or(DisplayError::Protocol)?);
            self.spi.write(&packet).map_err(DisplayError::Spi)?;
        }

        self.finish_write_transaction(delay)
    }

    fn finish_write_transaction<D>(
        &mut self,
        delay: &mut D,
    ) -> DisplayResult<SPI::Error, DISP::Error, EMD::Error, CS::Error>
    where
        D: DelayNs,
    {
        self.spi.write(&[0x00]).map_err(DisplayError::Spi)?;
        self.spi.flush().map_err(DisplayError::Spi)?;
        delay.delay_ns(CS_HOLD_NS);
        self.cs.set_low().map_err(DisplayError::Cs)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::convert::Infallible;
    use std::{cell::RefCell, rc::Rc, vec::Vec};

    #[derive(Clone, Default)]
    struct MockPin {
        states: Rc<RefCell<Vec<bool>>>,
    }

    impl OutputPin for MockPin {
        type Error = Infallible;

        fn set_low(&mut self) -> Result<(), Self::Error> {
            self.states.borrow_mut().push(false);
            Ok(())
        }

        fn set_high(&mut self) -> Result<(), Self::Error> {
            self.states.borrow_mut().push(true);
            Ok(())
        }
    }

    #[derive(Clone, Default)]
    struct MockSpi {
        writes: Rc<RefCell<Vec<Vec<u8>>>>,
        flushed: Rc<RefCell<u8>>,
    }

    impl SpiBus<u8> for MockSpi {
        type Error = Infallible;

        fn read(&mut self, words: &mut [u8]) -> Result<(), Self::Error> {
            words.fill(0);
            Ok(())
        }

        fn write(&mut self, words: &[u8]) -> Result<(), Self::Error> {
            self.writes.borrow_mut().push(words.to_vec());
            Ok(())
        }

        fn transfer(&mut self, read: &mut [u8], write: &[u8]) -> Result<(), Self::Error> {
            let count = read.len().min(write.len());
            read[..count].copy_from_slice(&write[..count]);
            self.writes.borrow_mut().push(write.to_vec());
            Ok(())
        }

        fn transfer_in_place(&mut self, words: &mut [u8]) -> Result<(), Self::Error> {
            self.writes.borrow_mut().push(words.to_vec());
            Ok(())
        }

        fn flush(&mut self) -> Result<(), Self::Error> {
            *self.flushed.borrow_mut() += 1;
            Ok(())
        }
    }

    #[derive(Default)]
    struct MockDelay;

    impl DelayNs for MockDelay {
        fn delay_ns(&mut self, _ns: u32) {}
    }

    #[test]
    fn diff_dirty_rows_marks_only_changed_rows() {
        let committed = FrameBuffer::new();
        let mut working = FrameBuffer::new();
        working.fill_rect(0, 12, 16, 2, true);

        let dirty = diff_dirty_rows(&committed, &working);

        assert_eq!(dirty.count(), 2);
        assert!(dirty.is_dirty_row(12));
        assert!(dirty.is_dirty_row(13));
        assert!(!dirty.is_dirty_row(11));
    }

    #[test]
    fn partial_present_writes_only_dirty_rows_and_updates_committed() {
        let spi = MockSpi::default();
        let writes = spi.writes.clone();
        let mut display = PlatformDisplay::new(
            spi,
            MockPin::default(),
            MockPin::default(),
            MockPin::default(),
        );
        let mut delay = MockDelay;
        let mut committed = FrameBuffer::new();
        let mut working = FrameBuffer::new();
        working.fill_rect(0, 5, 16, 1, true);

        let dirty = diff_dirty_rows(&committed, &working);
        let stats = display
            .present(&mut committed, &working, &dirty, &mut delay)
            .unwrap();

        assert_eq!(stats.dirty_rows, 1);
        assert_eq!(stats.bytes_sent, 1 + (LINE_BYTES + 2) + 1);

        let writes = writes.borrow();
        assert_eq!(writes[0], vec![protocol::build_write_command(true)]);
        assert_eq!(writes[1][0], protocol::encode_line_address(6).unwrap());
        assert_eq!(writes[2], vec![0x00]);
        assert_eq!(committed.row(5), working.row(5));
        assert_eq!(committed.row(4).unwrap(), &[0u8; LINE_BYTES]);
    }

    #[test]
    fn heartbeat_emits_display_mode_packet() {
        let spi = MockSpi::default();
        let writes = spi.writes.clone();
        let mut display = PlatformDisplay::new(
            spi,
            MockPin::default(),
            MockPin::default(),
            MockPin::default(),
        );
        let mut delay = MockDelay;

        display.heartbeat(&mut delay).unwrap();

        let writes = writes.borrow();
        assert_eq!(
            writes.as_slice(),
            &[protocol::build_display_mode_packet(true).to_vec()]
        );
    }
}
