#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    esp_println::println!("panic: {}", info);
    loop {
        core::hint::spin_loop();
    }
}

esp_bootloader_esp_idf::esp_app_desc!();

#[cfg(feature = "firmware-info-logs")]
const FIRMWARE_LOG_LEVEL: log::LevelFilter = log::LevelFilter::Info;
#[cfg(not(feature = "firmware-info-logs"))]
const FIRMWARE_LOG_LEVEL: log::LevelFilter = log::LevelFilter::Warn;

#[esp_rtos::main]
async fn main(spawner: embassy_executor::Spawner) -> ! {
    esp_println::logger::init_logger(FIRMWARE_LOG_LEVEL);
    esp_println::println!("boot: motif minimal firmware");
    platform_esp32s3::bootstrap::run_minimal(spawner).await
}
