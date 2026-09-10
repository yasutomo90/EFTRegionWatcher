use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub eft_log_path: Option<PathBuf>,
    pub connection_notifications: bool,
    pub minimize_to_tray_on_close: bool,
    pub dark_mode: bool,
    pub update: UpdateConfig,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateConfig {
    pub enabled: bool,
    pub check_interval_hours: u64,
    pub skipped_version: String,
    pub last_check: u64,
    pub repository: String,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            eft_log_path: None,
            connection_notifications: false,
            minimize_to_tray_on_close: false,
            dark_mode: true,
            update: UpdateConfig::default(),
        }
    }
}
impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            check_interval_hours: 24,
            skipped_version: String::new(),
            last_check: 0,
            repository: option_env!("EFT_RELEASE_REPOSITORY").unwrap_or("").into(),
        }
    }
}
pub fn data_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("EFTRegionWatcher")
}
impl Config {
    pub fn load(dir: &Path) -> io::Result<Self> {
        match fs::read_to_string(dir.join("config.toml")) {
            Ok(s) => {
                let mut config: Self = toml::from_str(&s).map_err(io::Error::other)?;
                if config.update.repository.is_empty() {
                    config.update.repository =
                        option_env!("EFT_RELEASE_REPOSITORY").unwrap_or("").into();
                }
                Ok(config)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
        }
    }
    pub fn save(&self, dir: &Path) -> io::Result<()> {
        atomic_write(
            &dir.join("config.toml"),
            toml::to_string_pretty(self)
                .map_err(io::Error::other)?
                .as_bytes(),
        )
    }
    pub fn update_due(&self, now: u64) -> bool {
        self.update.enabled
            && (self.update.last_check == 0
                || now.saturating_sub(self.update.last_check)
                    >= self.update.check_interval_hours.clamp(1, 168) * 3600)
    }
}
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        };
        let a: Vec<u16> = tmp.as_os_str().encode_wide().chain(Some(0)).collect();
        let b: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        if unsafe {
            MoveFileExW(
                a.as_ptr(),
                b.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
    }
    #[cfg(not(windows))]
    fs::rename(tmp, path)?;
    Ok(())
}
pub fn log(dir: &Path, level: &str, message: &str) {
    use std::io::Write;
    let folder = dir.join("logs");
    let _ = fs::create_dir_all(&folder);
    let p = folder.join("app.log");
    if fs::metadata(&p)
        .map(|m| m.len() > 256 * 1024)
        .unwrap_or(false)
    {
        let _ = fs::remove_file(folder.join("app.old.log"));
        let _ = fs::rename(&p, folder.join("app.old.log"));
    }
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(p) {
        let _ = writeln!(
            f,
            "{} {level} {}",
            crate::now(),
            message.chars().take(1000).collect::<String>()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn closing_exits_unless_the_setting_says_otherwise() {
        let config: Config = toml::from_str("connection_notifications = false").unwrap();
        assert!(!config.minimize_to_tray_on_close);
        assert!(!Config::default().minimize_to_tray_on_close);
    }
    #[test]
    fn update_schedule_and_settings_roundtrip() {
        let dir = std::env::temp_dir().join(format!("eft-config-{}", std::process::id()));
        let mut c = Config::default();
        assert!(c.update_due(100));
        c.update.last_check = 100;
        assert!(!c.update_due(101));
        assert!(c.update_due(86500));
        c.update.enabled = false;
        assert!(!c.update_due(999999));
        c.eft_log_path = Some(PathBuf::from(r"D:\Games\EFT\Logs"));
        c.connection_notifications = true;
        c.minimize_to_tray_on_close = false;
        c.dark_mode = false;
        c.save(&dir).unwrap();
        let loaded = Config::load(&dir).unwrap();
        assert_eq!(loaded.eft_log_path, c.eft_log_path);
        assert!(loaded.connection_notifications);
        assert!(!loaded.minimize_to_tray_on_close);
        assert!(!loaded.dark_mode);
        assert!(!loaded.update.enabled);
        fs::remove_dir_all(dir).unwrap();
    }
}
