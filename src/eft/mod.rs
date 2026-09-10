pub mod finder;
pub mod parser;
pub mod watcher;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerEndpoint {
    pub ip: IpAddr,
    pub port: Option<u16>,
}
impl std::fmt::Display for ServerEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Port is parsed for validation only; the UI shows the address.
        write!(f, "{}", self.ip)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectionState {
    Waiting,
    Connected(ServerEndpoint),
    LastSeen(ServerEndpoint),
}
impl ConnectionState {
    pub fn endpoint(&self) -> Option<&ServerEndpoint> {
        match self {
            Self::Waiting => None,
            Self::Connected(e) | Self::LastSeen(e) => Some(e),
        }
    }
    pub fn mark_last_seen(&mut self) {
        if let Self::Connected(e) = self {
            *self = Self::LastSeen(e.clone());
        }
    }
}
