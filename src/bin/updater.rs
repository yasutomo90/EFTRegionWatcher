#![cfg_attr(windows, windows_subsystem = "windows")]
#[cfg(windows)]
fn main() {
    if let Err(e) = run() {
        eft_region_watcher::platform::error(&format!(
            "更新できませんでした。旧バージョンを再起動してください。\n\n{e}"
        ));
    }
}
#[cfg(not(windows))]
fn main() {
    eprintln!("Windows 11 x64 が必要です。");
}
#[cfg(windows)]
fn run() -> std::io::Result<()> {
    use eft_region_watcher::{platform, update};
    use std::{io, path::PathBuf};
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() != 4 {
        return Err(io::Error::other(
            "このプログラムはメインアプリから更新時に起動します。",
        ));
    }
    let pid = args[1]
        .to_str()
        .and_then(|s| s.parse::<u32>().ok())
        .filter(|p| *p != 0)
        .ok_or_else(|| io::Error::other("不正なプロセス ID"))?;
    let target = PathBuf::from(&args[2]).canonicalize()?;
    let ready = PathBuf::from(&args[3]);
    let current = std::env::current_exe()?.canonicalize()?;
    let staging = current
        .parent()
        .ok_or_else(|| io::Error::other("更新元がありません"))?;
    let allowed = eft_region_watcher::config::data_dir()
        .join("updates")
        .canonicalize()?;
    if !update::staging_ok(staging, &allowed, &ready) {
        return Err(io::Error::other("不正な更新作業フォルダ"));
    }
    let parent = platform::process_handle(pid)?;
    if platform::executable_of(&parent)?.canonicalize()?
        != target.join(update::APP).canonicalize()?
    {
        return Err(io::Error::other("更新対象のアプリが一致しません"));
    }
    // Verify again in the helper before acknowledging readiness.
    let sums = std::fs::read_to_string(staging.join(update::SUMS))?;
    for name in [update::APP, update::UPDATER] {
        let bytes = std::fs::read(staging.join(name))?;
        update::verify(&bytes, &sums, name)?;
        update::verify_pe(&bytes)?;
    }
    std::fs::write(&ready, b"ready")?;
    if !platform::wait(&parent, 30000)? {
        return Err(io::Error::other("アプリの終了を待機できませんでした。"));
    }
    update::install(staging, &target, |p| platform::start(p).map(|_| ()))?;
    // The currently running helper cannot delete itself; cleanup happens on a later app start.
    Ok(())
}
