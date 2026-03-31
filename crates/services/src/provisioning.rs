use domain::provisioning::{ProvisioningSession, ProvisioningState, ProvisioningStatus};

pub trait ProvisioningService {
    fn state(&self) -> ProvisioningState;
    fn status(&self) -> ProvisioningStatus;
    fn start_session(&mut self) -> ProvisioningSession;
    fn cancel(&mut self) -> ProvisioningStatus;
}

#[derive(Debug, Default)]
pub struct NoopProvisioningService;

impl ProvisioningService for NoopProvisioningService {
    fn state(&self) -> ProvisioningState {
        ProvisioningState::Disabled
    }

    fn status(&self) -> ProvisioningStatus {
        ProvisioningStatus::new(ProvisioningState::Disabled)
    }

    fn start_session(&mut self) -> ProvisioningSession {
        ProvisioningSession::default()
    }

    fn cancel(&mut self) -> ProvisioningStatus {
        ProvisioningStatus::new(ProvisioningState::Disabled)
    }
}
