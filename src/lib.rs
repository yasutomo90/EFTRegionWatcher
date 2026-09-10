pub mod config;
pub mod eft;
pub mod geo;
#[cfg(windows)]
pub mod net;
#[cfg(windows)]
pub mod platform;
pub mod update;
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
