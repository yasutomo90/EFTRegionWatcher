use std::path::{Path, PathBuf};
pub fn find(configured: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = configured
        && p.is_dir()
    {
        return Some(p.into());
    }
    let mut candidates = vec![
        PathBuf::from(r"C:\Battlestate Games\EFT\Logs"),
        PathBuf::from(r"C:\Battlestate Games\Escape from Tarkov\Logs"),
    ];
    for key in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(p) = std::env::var_os(key) {
            candidates.push(PathBuf::from(p).join(r"Battlestate Games\EFT\Logs"));
        }
    }
    #[cfg(windows)]
    candidates.extend(registry_paths());
    candidates.into_iter().find(|p| p.is_dir())
}
#[cfg(windows)]
fn registry_paths() -> Vec<PathBuf> {
    use windows_sys::Win32::System::Registry::*;
    let mut result = Vec::new();
    for root in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        for key in [
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\EscapeFromTarkov",
            r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\EscapeFromTarkov",
        ] {
            let key: Vec<u16> = key.encode_utf16().chain(Some(0)).collect();
            let name: Vec<u16> = "InstallLocation".encode_utf16().chain(Some(0)).collect();
            let mut buf = [0u16; 2048];
            let mut size = (buf.len() * 2) as u32;
            if unsafe {
                RegGetValueW(
                    root,
                    key.as_ptr(),
                    name.as_ptr(),
                    RRF_RT_REG_SZ,
                    std::ptr::null_mut(),
                    buf.as_mut_ptr().cast(),
                    &mut size,
                )
            } == 0
            {
                let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
                result.push(PathBuf::from(String::from_utf16_lossy(&buf[..end])).join("Logs"));
            }
        }
    }
    result
}
