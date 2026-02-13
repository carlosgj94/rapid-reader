use embedded_hal::{digital::OutputPin, spi::SpiBus};
use esp_hal::{
    gpio::RtcPin,
    peripherals::{GPIO2, GPIO12, LPWR},
    rtc_cntl::{
        Rtc,
        sleep::{RtcioWakeupSource, WakeupLevel},
    },
};
use readily_hal_esp32s3::platform::display::SharpDisplay;

pub(super) fn enter_deep_sleep<DSPI, DISP, EMD, DCS, SDBUS, SDCS>(
    display: &mut SharpDisplay<DSPI, DISP, EMD, DCS>,
    sd_spi: &mut SDBUS,
    sd_cs: &mut SDCS,
) -> !
where
    DSPI: SpiBus<u8>,
    DISP: OutputPin,
    EMD: OutputPin,
    DCS: OutputPin,
    SDBUS: SpiBus<u8>,
    SDCS: OutputPin,
{
    // Put display in a deterministic off state before entering deep sleep.
    let _ = display.disable_output();
    // Latch DISP low through deep sleep.
    let disp_hold = unsafe { GPIO2::steal() };
    disp_hold.rtcio_pad_hold(true);

    // Keep SD bus idle and CS deasserted so no transaction remains active.
    let _ = sd_spi.flush();
    let _ = sd_cs.set_high();

    let mut rtc = Rtc::new(unsafe { LPWR::steal() });
    let mut wake_sw = unsafe { GPIO12::steal() };
    let mut wake_pins: [(&mut dyn RtcPin, WakeupLevel); 1] = [(&mut wake_sw, WakeupLevel::Low)];
    let wake_source = RtcioWakeupSource::new(&mut wake_pins);

    rtc.sleep_deep(&[&wake_source]);
}
