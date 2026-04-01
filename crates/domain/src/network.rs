#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum NetworkStatus {
    #[default]
    Offline,
    Connecting,
    Online,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct NetworkState {
    pub status: NetworkStatus,
}

impl NetworkState {
    pub const fn offline() -> Self {
        Self {
            status: NetworkStatus::Offline,
        }
    }

    pub const fn online() -> Self {
        Self {
            status: NetworkStatus::Online,
        }
    }
}

impl Default for NetworkState {
    fn default() -> Self {
        Self::online()
    }
}
