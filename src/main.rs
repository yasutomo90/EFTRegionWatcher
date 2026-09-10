#![cfg_attr(windows, windows_subsystem = "windows")]
#[cfg(windows)]
mod ui;
#[cfg(windows)]
fn main() {
    if let Err(e) = ui::run() {
        eft_region_watcher::platform::error(&format!("起動できませんでした。\n{e}"));
    }
}
#[cfg(not(windows))]
fn main() {
    eprintln!("EFTRegionWatcher は Windows 11 x64 専用です。");
}
