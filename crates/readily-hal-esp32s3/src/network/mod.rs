//! Wi-Fi/network connectivity state shared between async network workers and UI.

use core::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};

/// High-level connectivity state for UI + logs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ConnectivityState {
    Disconnected = 0,
    Connecting = 1,
    LinkUpNoIp = 2,
    Connected = 3,
    PingDegraded = 4,
}

impl ConnectivityState {
    fn from_raw(raw: u8) -> Self {
        match raw {
            1 => Self::Connecting,
            2 => Self::LinkUpNoIp,
            3 => Self::Connected,
            4 => Self::PingDegraded,
            _ => Self::Disconnected,
        }
    }
}

/// Wi-Fi credentials source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WifiConfig {
    pub ssid: &'static str,
    pub password: &'static str,
}

impl WifiConfig {
    pub const fn new(ssid: &'static str, password: &'static str) -> Self {
        Self { ssid, password }
    }
}

/// Immutable connectivity snapshot for renderer and board loop.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConnectivitySnapshot {
    pub state: ConnectivityState,
    pub link_up: bool,
    pub has_ipv4: bool,
    pub ping_ok: bool,
    pub revision: u32,
}

impl ConnectivitySnapshot {
    pub const fn disconnected() -> Self {
        Self {
            state: ConnectivityState::Disconnected,
            link_up: false,
            has_ipv4: false,
            ping_ok: false,
            revision: 0,
        }
    }

    /// Connection definition for the Wi-Fi icon: link + IPv4 config.
    pub const fn icon_connected(self) -> bool {
        self.link_up && self.has_ipv4
    }
}

/// Lock-free shared connectivity status.
#[derive(Debug)]
pub struct ConnectivityHandle {
    state: AtomicU8,
    link_up: AtomicBool,
    has_ipv4: AtomicBool,
    ping_ok: AtomicBool,
    revision: AtomicU32,
}

impl ConnectivityHandle {
    pub const fn new() -> Self {
        Self {
            state: AtomicU8::new(ConnectivityState::Disconnected as u8),
            link_up: AtomicBool::new(false),
            has_ipv4: AtomicBool::new(false),
            ping_ok: AtomicBool::new(false),
            revision: AtomicU32::new(0),
        }
    }

    pub fn snapshot(&self) -> ConnectivitySnapshot {
        ConnectivitySnapshot {
            state: ConnectivityState::from_raw(self.state.load(Ordering::Acquire)),
            link_up: self.link_up.load(Ordering::Acquire),
            has_ipv4: self.has_ipv4.load(Ordering::Acquire),
            ping_ok: self.ping_ok.load(Ordering::Acquire),
            revision: self.revision.load(Ordering::Acquire),
        }
    }

    pub fn mark_connecting(&self) {
        self.update_state(ConnectivityState::Connecting);
    }

    pub fn mark_disconnected(&self) {
        let mut changed = false;
        changed |= self.store_bool(&self.link_up, false);
        changed |= self.store_bool(&self.has_ipv4, false);
        changed |= self.store_bool(&self.ping_ok, false);
        changed |= self.store_state(ConnectivityState::Disconnected);
        if changed {
            self.bump_revision();
        }
    }

    pub fn update_link_ip(&self, link_up: bool, has_ipv4: bool) {
        let mut changed = false;
        changed |= self.store_bool(&self.link_up, link_up);
        changed |= self.store_bool(&self.has_ipv4, has_ipv4);

        let ping_ok = self.ping_ok.load(Ordering::Acquire);
        let next = Self::state_for(link_up, has_ipv4, ping_ok);
        changed |= self.store_state(next);

        if changed {
            self.bump_revision();
        }
    }

    pub fn update_ping(&self, ping_ok: bool) {
        let mut changed = false;
        changed |= self.store_bool(&self.ping_ok, ping_ok);

        let link_up = self.link_up.load(Ordering::Acquire);
        let has_ipv4 = self.has_ipv4.load(Ordering::Acquire);
        let next = Self::state_for(link_up, has_ipv4, ping_ok);
        changed |= self.store_state(next);

        if changed {
            self.bump_revision();
        }
    }

    fn update_state(&self, next: ConnectivityState) {
        if self.store_state(next) {
            self.bump_revision();
        }
    }

    fn state_for(link_up: bool, has_ipv4: bool, ping_ok: bool) -> ConnectivityState {
        if !link_up {
            ConnectivityState::Disconnected
        } else if !has_ipv4 {
            ConnectivityState::LinkUpNoIp
        } else if ping_ok {
            ConnectivityState::Connected
        } else {
            ConnectivityState::PingDegraded
        }
    }

    fn store_state(&self, next: ConnectivityState) -> bool {
        self.state.swap(next as u8, Ordering::AcqRel) != next as u8
    }

    fn store_bool(&self, cell: &AtomicBool, next: bool) -> bool {
        cell.swap(next, Ordering::AcqRel) != next
    }

    fn bump_revision(&self) {
        self.revision.fetch_add(1, Ordering::AcqRel);
    }
}

impl Default for ConnectivityHandle {
    fn default() -> Self {
        Self::new()
    }
}
