use domain::reader::ReaderProgress;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub struct QueueScreenModel;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub struct ReaderScreenModel {
    pub progress: ReaderProgress,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub struct SettingsScreenModel;
