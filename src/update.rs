use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs, io,
    path::{Path, PathBuf},
};
/// The helper may only run from a staging directory under `updates/`, with its readiness file
/// directly inside that same directory.
///
/// `staging` and `allowed` are canonicalized by the caller; `ready` is not, because the app passes
/// the path it built from the data directory and the file does not exist yet. Comparing the two
/// forms directly never matches on Windows, where canonicalization adds the `\\?\` prefix, so the
/// parent directory is canonicalized here instead.
pub fn staging_ok(staging: &Path, allowed: &Path, ready: &Path) -> bool {
    staging.starts_with(allowed)
        && staging != allowed
        && ready.file_name() == Some(std::ffi::OsStr::new("ready"))
        && ready
            .parent()
            .and_then(|p| p.canonicalize().ok())
            .is_some_and(|p| p == staging)
}
pub const APP: &str = "EFTRegionWatcher.exe";
pub const UPDATER: &str = "EFTRegionWatcher.Updater.exe";
pub const SUMS: &str = "SHA256SUMS.txt";
#[derive(Clone, Debug, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Release {
    pub tag_name: String,
    pub draft: bool,
    pub prerelease: bool,
    #[serde(default)]
    pub body: Option<String>,
    pub assets: Vec<Asset>,
}
pub fn valid_repository(s: &str) -> bool {
    let parts: Vec<_> = s.split('/').collect();
    parts.len() == 2
        && parts.iter().all(|p| {
            !p.is_empty()
                && *p != "."
                && *p != ".."
                && p.len() <= 100
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        })
}
pub fn version(s: &str) -> io::Result<Version> {
    Version::parse(s.strip_prefix('v').unwrap_or(s)).map_err(io::Error::other)
}
pub fn available(installed: &str, release: &Release, skipped: &str) -> io::Result<bool> {
    let latest = version(&release.tag_name)?;
    let installed = version(installed)?;
    Ok(!release.draft
        && !release.prerelease
        && latest.pre.is_empty()
        && latest > installed
        && version(skipped).ok().as_ref() != Some(&latest))
}
pub fn parse_release(bytes: &[u8]) -> io::Result<Release> {
    let r: Release = serde_json::from_slice(bytes).map_err(io::Error::other)?;
    version(&r.tag_name)?;
    Ok(r)
}
pub fn release_notes(r: &Release) -> String {
    r.body
        .as_deref()
        .unwrap_or("変更内容はありません。")
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .take(1600)
        .collect()
}
#[cfg(windows)]
pub fn check(repository: &str) -> io::Result<Release> {
    if !valid_repository(repository) {
        return Err(io::Error::other(
            "更新先が未設定です。正式なリリース版をご利用ください。",
        ));
    }
    parse_release(&crate::net::get(
        &format!("https://api.github.com/repos/{repository}/releases/latest"),
        512 * 1024,
    )?)
}
pub fn asset_url<'a>(r: &'a Release, repository: &str, name: &str) -> io::Result<&'a str> {
    if !valid_repository(repository) || !matches!(name, APP | UPDATER | SUMS) {
        return Err(io::Error::other("不正な更新メタデータ"));
    }
    let v = version(&r.tag_name)?;
    if !v.pre.is_empty() || !v.build.is_empty() {
        return Err(io::Error::other("安定版のバージョンではありません"));
    }
    let expected = format!(
        "https://github.com/{repository}/releases/download/{}/{name}",
        r.tag_name
    );
    let mut matches = r.assets.iter().filter(|a| a.name == name);
    let a = matches
        .next()
        .ok_or_else(|| io::Error::other(format!("更新ファイルが見つかりません: {name}")))?;
    if matches.next().is_some() || a.browser_download_url != expected {
        return Err(io::Error::other("更新 URL が公開リポジトリと一致しません"));
    }
    Ok(&a.browser_download_url)
}
pub fn verify(bytes: &[u8], sums: &str, name: &str) -> io::Result<()> {
    let mut found = None;
    for line in sums.lines() {
        let parts: Vec<_> = line.split_whitespace().collect();
        if parts.len() == 2 && parts[1].trim_start_matches('*') == name {
            if found.is_some() {
                return Err(io::Error::other("重複した SHA-256"));
            }
            found = Some(parts[0]);
        }
    }
    let expected = found.ok_or_else(|| io::Error::other("SHA-256 がありません"))?;
    if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(io::Error::other("不正な SHA-256"));
    }
    let actual = format!("{:x}", Sha256::digest(bytes));
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(io::Error::other(
            "更新ファイルの整合性を確認できませんでした。現在のバージョンを引き続き利用できます。",
        ));
    }
    Ok(())
}
pub fn verify_pe(bytes: &[u8]) -> io::Result<()> {
    if bytes.len() < 64 || &bytes[..2] != b"MZ" {
        return Err(io::Error::other("Windows 実行ファイルではありません"));
    }
    let offset = u32::from_le_bytes(bytes[60..64].try_into().map_err(io::Error::other)?) as usize;
    if offset > bytes.len().saturating_sub(6)
        || &bytes[offset..offset + 4] != b"PE\0\0"
        || bytes[offset + 4..offset + 6] != [0x64, 0x86]
    {
        return Err(io::Error::other("Windows x64 実行ファイルではありません"));
    }
    Ok(())
}
#[cfg(windows)]
pub fn stage(r: &Release, repository: &str, dir: &Path) -> io::Result<PathBuf> {
    // Validate every URL before downloading any assets.
    let sums_url = asset_url(r, repository, SUMS)?;
    let app_url = asset_url(r, repository, APP)?;
    let updater_url = asset_url(r, repository, UPDATER)?;
    let sums = crate::net::get(sums_url, 8192)?;
    let sums_text = std::str::from_utf8(&sums).map_err(io::Error::other)?;
    let staging_root = dir.join("updates");
    fs::create_dir_all(&staging_root)?;
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let staging = staging_root.join(format!("stage-{}-{unique}", std::process::id()));
    fs::create_dir(&staging)?;
    let result = (|| {
        for (name, url) in [(APP, app_url), (UPDATER, updater_url)] {
            let bytes = crate::net::get(url, 40 * 1024 * 1024)?;
            verify(&bytes, sums_text, name)?;
            verify_pe(&bytes)?;
            crate::config::atomic_write(&staging.join(name), &bytes)?;
        }
        crate::config::atomic_write(&staging.join(SUMS), &sums)?;
        Ok(staging.clone())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}
/// Transaction with local backups. The running helper lives in the staging directory.
/// On any replacement/launch failure, restore both old binaries.
pub fn install(
    staging: &Path,
    target: &Path,
    launch: impl FnOnce(&Path) -> io::Result<()>,
) -> io::Result<()> {
    if !target.is_absolute() || !target.is_dir() || !target.join(APP).is_file() {
        return Err(io::Error::other("インストール先が不正です"));
    }
    if staging.canonicalize()? == target.canonicalize()? {
        return Err(io::Error::other("更新元と更新先が同じです"));
    }
    let sums = fs::read_to_string(staging.join(SUMS))?;
    for name in [APP, UPDATER] {
        let b = fs::read(staging.join(name))?;
        verify(&b, &sums, name)?;
        verify_pe(&b)?;
    }
    let mut backups = Vec::new();
    let mut installed = Vec::new();
    let result = (|| {
        for name in [APP, UPDATER] {
            let dest = target.join(name);
            let backup = target.join(format!("{name}.previous"));
            let incoming = target.join(format!("{name}.incoming"));
            if backup.exists() || incoming.exists() {
                return Err(io::Error::other(
                    "前回の更新ファイルが残っています。現在のアプリを終了し、.previous / .incoming ファイルをご確認ください。",
                ));
            }
            fs::copy(staging.join(name), &incoming)?;
            if dest.exists() {
                if let Err(e) = fs::rename(&dest, &backup) {
                    let _ = fs::remove_file(&incoming);
                    return Err(e);
                }
                backups.push((dest.clone(), backup));
            }
            if let Err(e) = fs::rename(&incoming, &dest) {
                let _ = fs::remove_file(&incoming);
                return Err(e);
            }
            installed.push(dest);
        }
        launch(&target.join(APP))
    })();
    if let Err(error) = result {
        let mut rollback_errors = Vec::new();
        for p in installed.iter().rev() {
            if let Err(e) = fs::remove_file(p) {
                rollback_errors.push(e.to_string());
            }
        }
        for (dest, backup) in backups.iter().rev() {
            if let Err(e) = fs::rename(backup, dest) {
                rollback_errors.push(e.to_string());
            }
        }
        if !rollback_errors.is_empty() {
            return Err(io::Error::other(format!(
                "{error}; 復元に失敗しました。.previous ファイルを保存してください: {}",
                rollback_errors.join("; ")
            )));
        }
        return Err(error);
    }
    for (_, backup) in backups {
        let _ = fs::remove_file(backup);
    }
    Ok(())
}
/// Remove expired staging directories without following links outside our data directory.
pub fn cleanup_old_stages(dir: &Path) {
    let root = dir.join("updates");
    let Ok(entries) = fs::read_dir(&root) else {
        return;
    };
    for entry in entries.flatten().take(100) {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if !kind.is_dir()
            || kind.is_symlink()
            || !entry.file_name().to_string_lossy().starts_with("stage-")
        {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                continue;
            }
        }
        if metadata
            .modified()
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age.as_secs() > 7 * 86400)
        {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn release(tag: &str) -> Release {
        Release {
            tag_name: tag.into(),
            draft: false,
            prerelease: false,
            body: None,
            assets: vec![],
        }
    }
    #[test]
    fn staging_guard_accepts_the_path_the_app_actually_passes() {
        let allowed = std::env::temp_dir().join(format!("eft-updates-{}", std::process::id()));
        let staging = allowed.join("stage-1");
        fs::create_dir_all(&staging).unwrap();
        let allowed_c = allowed.canonicalize().unwrap();
        let staging_c = staging.canonicalize().unwrap();
        // The app builds `ready` from the data directory, so it arrives uncanonicalized.
        assert!(staging_ok(&staging_c, &allowed_c, &staging.join("ready")));
        assert!(staging_ok(&staging_c, &allowed_c, &staging_c.join("ready")));
        assert!(!staging_ok(&staging_c, &allowed_c, &staging.join("other")));
        assert!(!staging_ok(&staging_c, &allowed_c, &allowed.join("ready")));
        assert!(!staging_ok(&allowed_c, &allowed_c, &allowed.join("ready")));
        fs::remove_dir_all(&allowed).unwrap();
    }
    #[test]
    fn version_rules() {
        let mut r = release("v1.2.0");
        assert!(!available("1.2.0", &r, "").unwrap());
        assert!(available("1.1.0", &r, "").unwrap());
        assert!(!available("2.0.0", &r, "").unwrap());
        assert!(!available("1.0.0", &r, "1.2.0").unwrap());
        assert!(available("1.0.0", &r, "1.1.0").unwrap());
        r.draft = true;
        assert!(!available("1.0.0", &r, "").unwrap());
        r.draft = false;
        r.prerelease = true;
        assert!(!available("1.0.0", &r, "").unwrap());
        r.prerelease = false;
        r.tag_name = "v1.3.0-beta.1".into();
        assert!(!available("1.0.0", &r, "").unwrap());
    }
    #[test]
    fn malformed() {
        for b in [
            b"{}".as_slice(),
            b"no",
            br#"{"tag_name":"oops","draft":false,"prerelease":false,"assets":[]}"#,
        ] {
            assert!(parse_release(b).is_err());
        }
    }
    #[test]
    fn checksum() {
        let sums = format!("{:x}  {APP}\n", Sha256::digest(b"good"));
        verify(b"good", &sums, APP).unwrap();
        assert!(verify(b"bad", &sums, APP).is_err());
        assert!(verify(b"good", &(sums.clone() + &sums), APP).is_err());
        assert!(verify(b"good", "invalid  EFTRegionWatcher.exe", APP).is_err());
    }
    #[test]
    fn urls() {
        let mut r = release("v1.2.0");
        r.assets.push(Asset {
            name: APP.into(),
            browser_download_url: format!(
                "https://github.com/owner/repo/releases/download/v1.2.0/{APP}"
            ),
        });
        assert!(asset_url(&r, "owner/repo", APP).is_ok());
        r.assets[0].browser_download_url = "https://evil.invalid/app.exe".into();
        assert!(asset_url(&r, "owner/repo", APP).is_err());
        assert!(!valid_repository("../repo"));
    }
    fn fake_pe() -> Vec<u8> {
        let mut b = vec![0; 128];
        b[..2].copy_from_slice(b"MZ");
        b[60..64].copy_from_slice(&64u32.to_le_bytes());
        b[64..70].copy_from_slice(b"PE\0\0\x64\x86");
        b
    }
    #[test]
    fn failed_launch_restores_both_files() {
        let root = std::env::temp_dir().join(format!("eft-install-{}", std::process::id()));
        let s = root.join("stage");
        let t = root.join("target");
        fs::create_dir_all(&s).unwrap();
        fs::create_dir_all(&t).unwrap();
        let b = fake_pe();
        let mut sums = String::new();
        for n in [APP, UPDATER] {
            fs::write(s.join(n), &b).unwrap();
            fs::write(t.join(n), b"old").unwrap();
            sums += &format!("{:x}  {n}\n", Sha256::digest(&b));
        }
        fs::write(s.join(SUMS), sums).unwrap();
        assert!(
            install(&s, &t, |_| Err(io::Error::other(
                "simulated launch failure"
            )))
            .is_err()
        );
        for n in [APP, UPDATER] {
            assert_eq!(fs::read(t.join(n)).unwrap(), b"old");
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn corrupt_download_preserves_installation() {
        let root = std::env::temp_dir().join(format!("eft-corrupt-{}", std::process::id()));
        let staging = root.join("stage");
        let target = root.join("target");
        fs::create_dir_all(&staging).unwrap();
        fs::create_dir_all(&target).unwrap();
        let good = fake_pe();
        fs::write(target.join(APP), b"old application").unwrap();
        fs::write(target.join(UPDATER), b"old helper").unwrap();
        fs::write(staging.join(APP), b"corrupted download").unwrap();
        fs::write(
            staging.join(SUMS),
            format!("{:x}  {APP}\n", Sha256::digest(&good)),
        )
        .unwrap();
        assert!(install(&staging, &target, |_| panic!("must not launch")).is_err());
        assert_eq!(fs::read(target.join(APP)).unwrap(), b"old application");
        assert_eq!(fs::read(target.join(UPDATER)).unwrap(), b"old helper");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn successful_transaction_replaces_both() {
        let root = std::env::temp_dir().join(format!("eft-success-{}", std::process::id()));
        let staging = root.join("stage");
        let target = root.join("target");
        fs::create_dir_all(&staging).unwrap();
        fs::create_dir_all(&target).unwrap();
        let bytes = fake_pe();
        let mut sums = String::new();
        for name in [APP, UPDATER] {
            fs::write(staging.join(name), &bytes).unwrap();
            fs::write(target.join(name), b"old").unwrap();
            sums += &format!("{:x}  {name}\n", Sha256::digest(&bytes));
        }
        fs::write(staging.join(SUMS), sums).unwrap();
        install(&staging, &target, |exe| {
            assert_eq!(exe, target.join(APP));
            Ok(())
        })
        .unwrap();
        for name in [APP, UPDATER] {
            assert_eq!(fs::read(target.join(name)).unwrap(), bytes);
            assert!(!target.join(format!("{name}.previous")).exists());
        }
        fs::remove_dir_all(root).unwrap();
    }
}
