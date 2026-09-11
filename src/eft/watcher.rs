use super::{ServerEndpoint, parser::Lines};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, SystemTime},
};
#[derive(Debug)]
pub enum LogEvent {
    Endpoint {
        endpoint: ServerEndpoint,
        historical: bool,
        detected_at: u64,
    },
    Unavailable(String),
}
enum Message {
    Change(notify::Result<Event>),
    Poll,
    Stop,
}
pub struct LogWatcher {
    tx: SyncSender<Message>,
    worker: Option<JoinHandle<()>>,
}
impl Drop for LogWatcher {
    fn drop(&mut self) {
        let _ = self.tx.send(Message::Stop);
        if let Some(t) = self.worker.take() {
            let _ = t.join();
        }
    }
}
impl LogWatcher {
    pub fn start(root: PathBuf, send: impl Fn(LogEvent) + Send + 'static) -> io::Result<Self> {
        if !root.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "ログフォルダが見つかりません",
            ));
        }
        let (tx, rx) = mpsc::sync_channel(128);
        let tx2 = tx.clone();
        let overflow = Arc::new(AtomicBool::new(false));
        let overflow2 = overflow.clone();
        let mut watcher = RecommendedWatcher::new(
            move |e| {
                if tx2.try_send(Message::Change(e)).is_err() {
                    overflow2.store(true, Ordering::Relaxed);
                }
            },
            notify::Config::default(),
        )
        .map_err(io::Error::other)?;
        watcher
            .watch(&root, RecursiveMode::Recursive)
            .map_err(io::Error::other)?;
        let worker = thread::spawn(move || {
            let _watcher = watcher;
            let mut tails: HashMap<PathBuf, Tail> = HashMap::new();
            let mut pending: HashMap<PathBuf, u8> = HashMap::new();
            let mut historical_pending = HashSet::new();
            let mut paths = log_files(&root);
            paths.sort_by_key(|p| {
                fs::metadata(p)
                    .and_then(|m| m.modified())
                    .unwrap_or(SystemTime::UNIX_EPOCH)
            });
            for p in paths {
                let tail = tails.entry(p.clone()).or_default();
                match tail.read(&p, true) {
                    Ok(events) => {
                        let detected_at = modified(&p);
                        for endpoint in events {
                            send(LogEvent::Endpoint {
                                endpoint,
                                historical: true,
                                detected_at,
                            });
                        }
                    }
                    Err(_) => {
                        historical_pending.insert(p.clone());
                        pending.insert(p, 0);
                    }
                }
            }
            loop {
                let message = match rx.recv_timeout(Duration::from_millis(500)) {
                    Ok(m) => Some(m),
                    Err(mpsc::RecvTimeoutError::Timeout) => Some(Message::Poll),
                    Err(_) => None,
                };
                let mut changed = HashSet::new();
                match message {
                    None | Some(Message::Stop) => break,
                    // Windows defers change notifications for a file the game keeps open, so
                    // notify alone can stay silent for a whole raid. Rescan on every tick.
                    Some(Message::Poll) => changed.extend(log_files(&root)),
                    Some(Message::Change(Err(e))) => {
                        send(LogEvent::Unavailable(format!("ログ監視エラー: {e}")));
                        changed.extend(log_files(&root));
                    }
                    Some(Message::Change(Ok(event))) => {
                        for p in event.paths {
                            if p.is_dir() {
                                changed.extend(log_files(&p));
                            } else if is_log(&p) {
                                changed.insert(p);
                            }
                        }
                    }
                }
                if overflow.swap(false, Ordering::Relaxed) {
                    changed.extend(log_files(&root));
                }
                changed.extend(pending.keys().cloned());
                if !root.is_dir() {
                    send(LogEvent::Unavailable(
                        "ログフォルダがなくなりました。設定から選び直してください。".into(),
                    ));
                    break;
                }
                let mut changed: Vec<_> = changed.into_iter().collect();
                changed.sort_by_key(|p| {
                    fs::metadata(p)
                        .and_then(|m| m.modified())
                        .unwrap_or(SystemTime::UNIX_EPOCH)
                });
                for p in changed {
                    if !p.exists() {
                        tails.remove(&p);
                        pending.remove(&p);
                        historical_pending.remove(&p);
                        continue;
                    }
                    if !tails.contains_key(&p) && tails.len() >= 1024 {
                        tails.retain(|p, _| p.exists());
                        if tails.len() >= 1024
                            && let Some(old) = tails
                                .keys()
                                .min_by_key(|p| fs::metadata(p).and_then(|m| m.modified()).ok())
                                .cloned()
                        {
                            tails.remove(&old);
                        }
                    }
                    let historical = historical_pending.contains(&p);
                    match tails.entry(p.clone()).or_default().read(&p, historical) {
                        Ok(events) => {
                            pending.remove(&p);
                            historical_pending.remove(&p);
                            for endpoint in events {
                                send(LogEvent::Endpoint {
                                    endpoint,
                                    historical,
                                    detected_at: if historical {
                                        modified(&p)
                                    } else {
                                        crate::now()
                                    },
                                });
                            }
                        }
                        Err(e) => {
                            let count = pending.entry(p.clone()).or_default();
                            *count += 1;
                            if *count >= 4 {
                                pending.remove(&p);
                                send(LogEvent::Unavailable(format!(
                                    "ログを読み取れません。次の変更時に再試行します: {e}"
                                )));
                            }
                        }
                    }
                }
            }
        });
        Ok(Self {
            tx,
            worker: Some(worker),
        })
    }
}
fn modified(p: &Path) -> u64 {
    fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
fn is_log(p: &Path) -> bool {
    p.extension().is_some_and(|s| s.eq_ignore_ascii_case("log"))
}
fn log_files(root: &Path) -> Vec<PathBuf> {
    let mut result = Vec::new();
    let mut dirs = vec![(root.to_path_buf(), 0)];
    let mut visited = 0;
    while let Some((dir, depth)) = dirs.pop() {
        visited += 1;
        if visited > 2048 {
            break;
        }
        if let Ok(entries) = fs::read_dir(dir) {
            for e in entries.flatten() {
                if result.len() >= 1024 {
                    return result;
                }
                let Ok(kind) = e.file_type() else {
                    continue;
                };
                let p = e.path();
                if kind.is_dir() && depth < 3 {
                    dirs.push((p, depth + 1));
                } else if kind.is_file() && is_log(&p) {
                    result.push(p);
                }
            }
        }
    }
    result
}
#[derive(Default)]
pub struct Tail {
    offset: u64,
    identity: Option<(u64, u64)>,
    lines: Lines,
    anchor: Vec<u8>,
}
impl Tail {
    pub fn read(&mut self, path: &Path, initial: bool) -> io::Result<Vec<ServerEndpoint>> {
        let mut f = File::open(path)?;
        let metadata = f.metadata()?;
        let identity = file_identity(&f, &metadata)?;
        let replaced = self.identity.is_some_and(|i| i != identity);
        let mut reset = replaced || metadata.len() < self.offset;
        if !reset && !self.anchor.is_empty() {
            f.seek(SeekFrom::Start(self.offset - self.anchor.len() as u64))?;
            let mut old = vec![0; self.anchor.len()];
            f.read_exact(&mut old)?;
            reset = old != self.anchor;
        }
        if reset {
            self.offset = 0;
            self.lines = Lines::default();
            self.anchor.clear();
        }
        self.identity = Some(identity);
        if initial && metadata.len() > 65536 {
            self.offset = metadata.len() - 65536;
            self.lines.discard_partial();
        }
        f.seek(SeekFrom::Start(self.offset))?;
        let mut result = Vec::new();
        let mut buffer = [0u8; 16384];
        // Snapshot length bounds work even if another process keeps appending.
        let end = metadata.len();
        while self.offset < end {
            let limit = ((end - self.offset) as usize).min(buffer.len());
            let n = f.read(&mut buffer[..limit])?;
            if n == 0 {
                break;
            }
            self.offset += n as u64;
            for endpoint in self.lines.feed(&buffer[..n]) {
                if result.last() == Some(&endpoint) {
                    continue;
                }
                // ponytail: newest 64 per read; the history list shows far fewer.
                if result.len() >= 64 {
                    result.remove(0);
                }
                result.push(endpoint);
            }
        }
        let n = self.offset.min(32) as usize;
        self.anchor.resize(n, 0);
        f.seek(SeekFrom::Start(self.offset - n as u64))?;
        f.read_exact(&mut self.anchor)?;
        Ok(result)
    }
}
#[cfg(windows)]
fn file_identity(f: &File, _: &fs::Metadata) -> io::Result<(u64, u64)> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::*;
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(f.as_raw_handle(), &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((
        info.dwVolumeSerialNumber as u64,
        ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
    ))
}
#[cfg(not(windows))]
fn file_identity(_: &File, m: &fs::Metadata) -> io::Result<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    Ok((m.dev(), m.ino()))
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn append_truncate_replace_and_partial() {
        let d = std::env::temp_dir().join(format!("eft-tail-{}", std::process::id()));
        fs::create_dir_all(&d).unwrap();
        let p = d.join("test.log");
        fs::write(&p, b"Connect (address: 1.2.3.4:12)").unwrap();
        let mut t = Tail::default();
        assert!(t.read(&p, false).unwrap().is_empty());
        File::options()
            .append(true)
            .open(&p)
            .unwrap()
            .write_all(b"\n")
            .unwrap();
        assert_eq!(t.read(&p, false).unwrap().len(), 1);
        assert!(t.read(&p, false).unwrap().is_empty());
        fs::write(&p, b"Connect (address: 5.6.7.8:13)\n").unwrap();
        assert_eq!(t.read(&p, false).unwrap()[0].port, Some(13));
        fs::remove_file(&p).unwrap();
        fs::write(&p, b"Connect (address: 9.8.7.6:14)\n").unwrap();
        assert_eq!(t.read(&p, false).unwrap()[0].port, Some(14));
        fs::remove_dir_all(d).unwrap();
    }
    #[test]
    fn native_notifications_new_session() {
        let d = std::env::temp_dir().join(format!("eft-watch-{}", std::process::id()));
        fs::create_dir_all(&d).unwrap();
        let (tx, rx) = mpsc::channel();
        let watcher = LogWatcher::start(d.clone(), move |e| {
            let _ = tx.send(e);
        })
        .unwrap();
        let session = d.join("session");
        fs::create_dir_all(&session).unwrap();
        fs::write(
            session.join("application.log"),
            b"Connect (address: 1.2.3.4:42)\n",
        )
        .unwrap();
        let event = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(event, LogEvent::Endpoint { .. }));
        drop(watcher);
        fs::remove_dir_all(d).unwrap();
    }
    /// 連続レイド: 既存のログに追記されたら、通知の有無に関わらず拾えること。
    #[test]
    fn appended_line_is_detected() {
        let d = std::env::temp_dir().join(format!("eft-append-{}", std::process::id()));
        fs::create_dir_all(&d).unwrap();
        let log = d.join("application.log");
        fs::write(&log, b"Connect (address: 1.2.3.4:42)\n").unwrap();
        let (tx, rx) = mpsc::channel();
        let watcher = LogWatcher::start(d.clone(), move |e| {
            let _ = tx.send(e);
        })
        .unwrap();
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            LogEvent::Endpoint {
                historical: true,
                ..
            }
        ));
        let mut f = File::options().append(true).open(&log).unwrap();
        f.write_all(b"Connect (address: 5.6.7.8:43)\n").unwrap();
        f.flush().unwrap();
        let event = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        match event {
            LogEvent::Endpoint {
                endpoint,
                historical,
                ..
            } => {
                assert_eq!(endpoint.port, Some(43));
                assert!(!historical);
            }
            e => panic!("{e:?}"),
        }
        drop(watcher);
        fs::remove_dir_all(d).unwrap();
    }
}
