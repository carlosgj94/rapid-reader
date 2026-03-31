#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use embedded_hal::{delay::DelayNs, digital::OutputPin, spi::SpiBus};
use esp_hal::{
    clock::CpuClock,
    delay::Delay,
    gpio::{Level, Output, OutputConfig, RtcPin},
    rtc_cntl::{SocResetReason, reset_reason, wakeup_cause},
    spi::master::Spi,
    system::Cpu,
    time::Rate,
    timer::timg::TimerGroup,
};
use log::{LevelFilter, info};
use ls027b7dh01::{
    FrameBuffer,
    protocol::{self, HEIGHT, LINE_BYTES, WIDTH},
};

const DISPLAY_SPI_HZ: u32 = 1_000_000;

const DISPLAY_CLK_GPIO: u8 = 13;
const DISPLAY_DI_GPIO: u8 = 14;
const DISPLAY_CS_GPIO: u8 = 15;
const DISPLAY_DISP_GPIO: u8 = 2;
const DISPLAY_EMD_GPIO: u8 = 9;

const SD_CS_GPIO: u8 = 8;
const SD_SCK_GPIO: u8 = 4;
const SD_MOSI_GPIO: u8 = 40;
const SD_MISO_GPIO: u8 = 41;

const ENCODER_CLK_GPIO: u8 = 10;
const ENCODER_DT_GPIO: u8 = 11;
const ENCODER_SW_GPIO: u8 = 12;

const CS_SETUP_NS: u32 = 3_000;
const CS_HOLD_NS: u32 = 1_000;
const CMD_WRITE: u8 = 0x80;
const VCOM_BIT: u8 = 0x40;
const HEARTBEAT_MS: u32 = 1_000;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum DisplayError<SpiErr, DispErr, EmdErr, CsErr> {
    Spi(SpiErr),
    Disp(DispErr),
    Emd(EmdErr),
    Cs(CsErr),
    Protocol,
}

type SharpDisplayResult<SpiErr, DispErr, EmdErr, CsErr> =
    Result<(), DisplayError<SpiErr, DispErr, EmdErr, CsErr>>;

struct SharpDisplay<SPI, DISP, EMD, CS> {
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
    fn new(spi: SPI, disp: DISP, emd: EMD, cs: CS) -> Self {
        Self {
            spi,
            disp,
            emd,
            cs,
            vcom_high: false,
        }
    }

    fn initialize<D>(
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

    fn clear_all<D>(
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

    fn flush_frame<D>(
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

        let mut packet = [0u8; LINE_BYTES + 2];
        packet[LINE_BYTES + 1] = 0x00;

        for line in 1..=HEIGHT as u16 {
            packet[0] = protocol::encode_line_address(line).ok_or(DisplayError::Protocol)?;
            let start = (line as usize - 1) * LINE_BYTES;
            let end = start + LINE_BYTES;
            packet[1..1 + LINE_BYTES].copy_from_slice(&frame.bytes()[start..end]);
            self.spi.write(&packet).map_err(DisplayError::Spi)?;
        }

        self.spi.write(&[0x00]).map_err(DisplayError::Spi)?;
        self.spi.flush().map_err(DisplayError::Spi)?;
        delay.delay_ns(CS_HOLD_NS);
        self.cs.set_low().map_err(DisplayError::Cs)?;
        Ok(())
    }

    fn disable_output(
        &mut self,
    ) -> SharpDisplayResult<SPI::Error, DISP::Error, EMD::Error, CS::Error> {
        self.cs.set_low().map_err(DisplayError::Cs)?;
        self.emd.set_low().map_err(DisplayError::Emd)?;
        self.disp.set_low().map_err(DisplayError::Disp)?;
        Ok(())
    }
}

fn draw_idle_frame(frame: &mut FrameBuffer, heartbeat_on: bool) {
    frame.clear(false);

    for x in 0..WIDTH {
        let _ = frame.set_pixel(x, 0, true);
        let _ = frame.set_pixel(x, HEIGHT - 1, true);
    }

    for y in 0..HEIGHT {
        let _ = frame.set_pixel(0, y, true);
        let _ = frame.set_pixel(WIDTH - 1, y, true);
    }

    for x in 32..(WIDTH - 32) {
        let _ = frame.set_pixel(x, HEIGHT / 2, true);
    }

    for y in 48..(HEIGHT - 48) {
        let _ = frame.set_pixel(WIDTH / 2, y, true);
    }

    let heartbeat_x = WIDTH / 2;
    let heartbeat_y = HEIGHT / 2;
    for dy in 0..8 {
        for dx in 0..8 {
            let _ = frame.set_pixel(heartbeat_x + dx - 4, heartbeat_y + dy - 4, heartbeat_on);
        }
    }
}

fn log_gpio_contract() {
    info!(
        "display gpio clk={} di={} cs={} disp={} emd={}",
        DISPLAY_CLK_GPIO, DISPLAY_DI_GPIO, DISPLAY_CS_GPIO, DISPLAY_DISP_GPIO, DISPLAY_EMD_GPIO
    );
    info!(
        "sd gpio cs={} sck={} mosi={} miso={}",
        SD_CS_GPIO, SD_SCK_GPIO, SD_MOSI_GPIO, SD_MISO_GPIO
    );
    info!(
        "encoder gpio clk={} dt={} sw={}",
        ENCODER_CLK_GPIO, ENCODER_DT_GPIO, ENCODER_SW_GPIO
    );
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    esp_println::println!("panic: {}", info);
    loop {
        core::hint::spin_loop();
    }
}

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(_spawner: embassy_executor::Spawner) -> ! {
    esp_println::logger::init_logger(LevelFilter::Info);
    esp_println::println!("boot: readily minimal firmware");

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    let boot_reset_reason = reset_reason(Cpu::ProCpu);
    let boot_wakeup_cause = wakeup_cause();
    let woke_from_deep_sleep = boot_reset_reason == Some(SocResetReason::CoreDeepSleep);
    info!(
        "boot reset_reason={:?} wakeup_cause={:?} wake={}",
        boot_reset_reason, boot_wakeup_cause, woke_from_deep_sleep
    );

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 65536);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    let disp_pin = peripherals.GPIO2;
    disp_pin.rtcio_pad_hold(false);
    let disp = Output::new(disp_pin, Level::Low, OutputConfig::default());
    let emd = Output::new(peripherals.GPIO9, Level::Low, OutputConfig::default());
    let cs = Output::new(peripherals.GPIO15, Level::Low, OutputConfig::default());

    let spi_config = esp_hal::spi::master::Config::default()
        .with_frequency(Rate::from_hz(DISPLAY_SPI_HZ))
        .with_mode(esp_hal::spi::Mode::_1);
    let spi = Spi::new(peripherals.SPI2, spi_config)
        .unwrap()
        .with_sck(peripherals.GPIO13)
        .with_mosi(peripherals.GPIO14);

    let mut delay = Delay::new();
    let mut display = SharpDisplay::new(spi, disp, emd, cs);

    if let Err(err) = display.initialize(&mut delay) {
        info!("display initialize failed: {:?}", err);
    }
    if let Err(err) = display.clear_all(&mut delay) {
        info!("display clear failed: {:?}", err);
    }

    log_gpio_contract();

    let mut frame = FrameBuffer::new();
    let mut heartbeat_on = true;
    draw_idle_frame(&mut frame, heartbeat_on);
    if let Err(err) = display.flush_frame(&frame, &mut delay) {
        info!("display initial flush failed: {:?}", err);
    } else {
        info!("display idle frame flushed");
    }

    loop {
        heartbeat_on = !heartbeat_on;
        draw_idle_frame(&mut frame, heartbeat_on);
        if let Err(err) = display.flush_frame(&frame, &mut delay) {
            info!("display heartbeat flush failed: {:?}", err);
            let _ = display.disable_output();
        }
        delay.delay_ms(HEARTBEAT_MS);
    }
}
