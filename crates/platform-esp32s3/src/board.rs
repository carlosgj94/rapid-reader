pub const DISPLAY_CLK_GPIO: u8 = 13;
pub const DISPLAY_DI_GPIO: u8 = 14;
pub const DISPLAY_CS_GPIO: u8 = 15;
pub const DISPLAY_DISP_GPIO: u8 = 2;
pub const DISPLAY_EMD_GPIO: u8 = 9;

pub const SD_CS_GPIO: u8 = 8;
pub const SD_SCK_GPIO: u8 = 4;
pub const SD_MOSI_GPIO: u8 = 40;
pub const SD_MISO_GPIO: u8 = 41;

pub const ENCODER_CLK_GPIO: u8 = 10;
pub const ENCODER_DT_GPIO: u8 = 11;
pub const ENCODER_SW_GPIO: u8 = 12;
pub const SLEEP_WAKE_GPIO: u8 = ENCODER_SW_GPIO;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct BoardConfig {
    pub display_clk_gpio: u8,
    pub display_di_gpio: u8,
    pub display_cs_gpio: u8,
    pub display_disp_gpio: u8,
    pub display_emd_gpio: u8,
    pub sd_cs_gpio: u8,
    pub sd_sck_gpio: u8,
    pub sd_mosi_gpio: u8,
    pub sd_miso_gpio: u8,
    pub encoder_clk_gpio: u8,
    pub encoder_dt_gpio: u8,
    pub encoder_sw_gpio: u8,
    pub sleep_wake_gpio: u8,
}

impl BoardConfig {
    pub const fn new() -> Self {
        Self {
            display_clk_gpio: DISPLAY_CLK_GPIO,
            display_di_gpio: DISPLAY_DI_GPIO,
            display_cs_gpio: DISPLAY_CS_GPIO,
            display_disp_gpio: DISPLAY_DISP_GPIO,
            display_emd_gpio: DISPLAY_EMD_GPIO,
            sd_cs_gpio: SD_CS_GPIO,
            sd_sck_gpio: SD_SCK_GPIO,
            sd_mosi_gpio: SD_MOSI_GPIO,
            sd_miso_gpio: SD_MISO_GPIO,
            encoder_clk_gpio: ENCODER_CLK_GPIO,
            encoder_dt_gpio: ENCODER_DT_GPIO,
            encoder_sw_gpio: ENCODER_SW_GPIO,
            sleep_wake_gpio: SLEEP_WAKE_GPIO,
        }
    }
}

impl Default for BoardConfig {
    fn default() -> Self {
        Self::new()
    }
}
