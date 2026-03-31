use ::domain::sleep::{DEFAULT_INACTIVITY_TIMEOUT_MS, SleepConfig, SleepModel, WakeReason};
use ::services::sleep::SleepService;
use esp_hal::{
    gpio::RtcPinWithResistors,
    rtc_cntl::{
        Rtc,
        sleep::{Ext0WakeupSource, WakeupLevel},
    },
};
use log::info;

#[derive(Debug, Default)]
pub struct PlatformSleepService {
    model: SleepModel,
}

impl PlatformSleepService {
    pub const fn new() -> Self {
        Self {
            model: SleepModel::new(SleepConfig::new(DEFAULT_INACTIVITY_TIMEOUT_MS)),
        }
    }

    pub fn hydrate_from_boot(&mut self, woke_from_deep_sleep: bool, now_ms: u64) {
        let reason = if woke_from_deep_sleep {
            WakeReason::ExternalButton
        } else {
            WakeReason::ColdBoot
        };
        self.mark_woke(reason, now_ms);
    }

    pub fn configure_inactivity_timeout(&mut self, inactivity_timeout_ms: u64) {
        self.model.config.inactivity_timeout_ms = inactivity_timeout_ms;
    }
}

impl SleepService for PlatformSleepService {
    fn model(&self) -> &SleepModel {
        &self.model
    }

    fn model_mut(&mut self) -> &mut SleepModel {
        &mut self.model
    }
}

pub fn enter_deep_sleep_with_button<P>(
    sleep: &mut PlatformSleepService,
    rtc: &mut Rtc<'_>,
    wake_pin: P,
) -> !
where
    P: RtcPinWithResistors,
{
    sleep.mark_deep_sleeping();
    wake_pin.rtcio_pullup(true);
    wake_pin.rtcio_pulldown(false);
    info!(
        "entering deep sleep after {} ms inactivity, wake gpio={}",
        sleep.model().config.inactivity_timeout_ms,
        wake_pin.number()
    );
    let ext0 = Ext0WakeupSource::new(wake_pin, WakeupLevel::Low);
    rtc.sleep_deep(&[&ext0]);
}
