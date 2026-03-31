use ::services::{
    backend_sync::NoopBackendSyncService, formatter::NoopFormatterService, power::NoopPowerService,
    provisioning::NoopProvisioningService, storage::NoopStorageService, wifi::NoopWifiService,
};

use crate::{input::PlatformInputService, sleep::PlatformSleepService};

pub struct PlatformServices<'d> {
    pub wifi: NoopWifiService,
    pub backend_sync: NoopBackendSyncService,
    pub formatter: NoopFormatterService,
    pub storage: NoopStorageService,
    pub provisioning: NoopProvisioningService,
    pub input: PlatformInputService<'d>,
    pub power: NoopPowerService,
    pub sleep: PlatformSleepService,
}

impl<'d> PlatformServices<'d> {
    pub fn new(input: PlatformInputService<'d>) -> Self {
        Self {
            wifi: NoopWifiService,
            backend_sync: NoopBackendSyncService,
            formatter: NoopFormatterService,
            storage: NoopStorageService,
            provisioning: NoopProvisioningService,
            input,
            power: NoopPowerService,
            sleep: PlatformSleepService::new(),
        }
    }
}
