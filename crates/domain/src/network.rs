#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum NetworkStatus {
    Disabled,
    #[default]
    Offline,
    Connecting,
    Online,
    ProbeFailed,
}

impl NetworkStatus {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Disabled => "Disabled",
            Self::Offline => "Offline",
            Self::Connecting => "Connecting",
            Self::Online => "Online",
            Self::ProbeFailed => "Probe Failed",
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct NetworkState {
    pub status: NetworkStatus,
}

impl NetworkState {
    pub const fn disabled() -> Self {
        Self {
            status: NetworkStatus::Disabled,
        }
    }

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

    pub const fn connecting() -> Self {
        Self {
            status: NetworkStatus::Connecting,
        }
    }

    pub const fn probe_failed() -> Self {
        Self {
            status: NetworkStatus::ProbeFailed,
        }
    }
}

impl Default for NetworkState {
    fn default() -> Self {
        Self::offline()
    }
}
