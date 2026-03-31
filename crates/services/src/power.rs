#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum PowerStatus {
    #[default]
    Unsupported,
    Ready,
}

pub trait PowerService {
    fn status(&self) -> PowerStatus;
}

#[derive(Debug, Default)]
pub struct NoopPowerService;

impl PowerService for NoopPowerService {
    fn status(&self) -> PowerStatus {
        PowerStatus::Unsupported
    }
}
