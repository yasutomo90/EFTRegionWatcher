use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetFocus, SetFocus, VK_ESCAPE, VK_RETURN,
};
mod style;
use eft_region_watcher::{
    config::{self, Config},
    eft::{
        self, ConnectionState,
        watcher::{LogEvent, LogWatcher},
    },
    geo::{GeoCache, IpWhoIs, Location},
    platform::{self, OwnedHandle, wide},
    update::{self, Release},
};
use std::{
    io, mem,
    net::IpAddr,
    path::PathBuf,
    ptr,
    sync::{
        Arc,
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::{Com::*, DataExchange::*, LibraryLoader::*, Memory::*, Threading::*},
    UI::{Controls::*, Shell::*, WindowsAndMessaging::*},
};
const WAKE: u32 = WM_APP + 1;
const TRAY: u32 = WM_APP + 2;
const COMMAND: u32 = WM_APP + 3;
const OPEN: usize = 101;
const COPY: usize = 102;
const CHECK: usize = 103;
const SETTINGS: usize = 104;
const EXIT: usize = 105;
const SELECT: usize = 106;
const SCROLL_UP: usize = 107;
const SCROLL_DOWN: usize = 108;
/// Newest last. Recorded on the watcher thread, read while painting.
type History = Arc<std::sync::Mutex<Vec<(eft::ServerEndpoint, u64)>>>;
const HISTORY_ROWS: usize = 5;
const HISTORY_MAX: usize = 200;
const EXPIRE: usize = 1;
const CLOSE: usize = 1101;
enum Event {
    Log(u64, LogEvent),
    Geo(IpAddr, Result<Location, String>),
    Checked(bool, Result<Release, String>),
    InstallError(String),
    ExitForUpdate,
}
enum Job {
    Geo(IpAddr),
    Check(bool, String),
    Download(Release, String, Vec<OwnedHandle>),
}
struct App {
    hwnd: HWND,
    buttons: Vec<HWND>,
    settings_open: bool,
    config: Config,
    dir: PathBuf,
    state: ConnectionState,
    /// None marks a lookup that failed; the worker backs off before retrying.
    locations: std::collections::HashMap<IpAddr, Option<Location>>,
    detected: u64,
    history: History,
    offset: usize,
    log_tab: bool,
    focus_visible: bool,
    warning: String,
    watcher: Option<LogWatcher>,
    generation: u64,
    tx: Inbox,
    jobs: Option<SyncSender<Job>>,
    busy: bool,
    checking: bool,
    geo_inflight: bool,
    stop: bool,
    tray: NOTIFYICONDATAW,
}
#[derive(Clone, Default)]
struct Inbox(Arc<std::sync::Mutex<std::collections::VecDeque<Event>>>);
impl Inbox {
    fn pop(&self) -> Option<Event> {
        self.0.lock().ok()?.pop_front()
    }
    fn push(&self, event: Event) {
        if let Ok(mut q) = self.0.lock() {
            q.retain(|old|!matches!((&event,old),(Event::Log(a,LogEvent::Endpoint{..}),Event::Log(b,LogEvent::Endpoint{..})) if a==b));
            if q.len() >= 64
                && let Some(i) = q
                    .iter()
                    .position(|e| matches!(e, Event::Log(..) | Event::Geo(..)))
            {
                q.remove(i);
            }
            q.push_back(event);
        }
    }
}
fn post(tx: &Inbox, hwnd: usize, event: Event) {
    tx.push(event);
    unsafe {
        PostMessageW(hwnd as HWND, WAKE, 0, 0);
    }
}
fn worker(
    rx: Receiver<Job>,
    tx: Inbox,
    hwnd: usize,
    dir: PathBuf,
    cancel: Arc<OwnedHandle>,
    dedicated_update: bool,
) {
    let mut installers = Vec::new();
    if !dedicated_update {
        update::cleanup_old_stages(&dir);
    }
    let mut cache = GeoCache::load(dir.join("geo_cache.json"));
    while let Ok(job) = rx.recv() {
        if platform::wait(&cancel, 0).unwrap_or(true) {
            break;
        }
        match job {
            Job::Geo(ip) => {
                let r = cache
                    .resolve(ip, &IpWhoIs, eft_region_watcher::now())
                    .map_err(|e| e.to_string());
                post(&tx, hwnd, Event::Geo(ip, r));
            }
            Job::Check(manual, repository) => post(
                &tx,
                hwnd,
                Event::Checked(
                    manual,
                    update::check(&repository).map_err(|e| e.to_string()),
                ),
            ),
            Job::Download(release, repository, games) => {
                if !dedicated_update {
                    let (one, receiver) = mpsc::sync_channel(1);
                    let _ = one.send(Job::Download(release, repository, games));
                    drop(one);
                    let tx = tx.clone();
                    let dir = dir.clone();
                    let cancel = cancel.clone();
                    installers.push(thread::spawn(move || {
                        worker(receiver, tx, hwnd, dir, cancel, true)
                    }));
                    continue;
                }
                let result = (|| -> io::Result<()> {
                    for game in games {
                        let handles = [cancel.0, game.0];
                        let wait =
                            unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE) };
                        if wait == WAIT_OBJECT_0 {
                            return Err(io::Error::other("更新をキャンセルしました"));
                        }
                        if wait != WAIT_OBJECT_0 + 1 {
                            return Err(io::Error::last_os_error());
                        }
                    }
                    let staging = update::stage(&release, &repository, &dir)?;
                    if platform::wait(&cancel, 0)? {
                        return Err(io::Error::other("更新をキャンセルしました"));
                    }
                    let exe = std::env::current_exe()?;
                    let target = exe
                        .parent()
                        .ok_or_else(|| io::Error::other("アプリのフォルダがありません"))?;
                    if exe.file_name().is_none_or(|n| n != update::APP) {
                        return Err(io::Error::other(
                            "アプリの名前を EFTRegionWatcher.exe に戻してから更新してください。",
                        ));
                    }
                    let ready = staging.join("ready");
                    let mut child = std::process::Command::new(staging.join(update::UPDATER))
                        .arg(std::process::id().to_string())
                        .arg(target)
                        .arg(&ready)
                        .current_dir(&staging)
                        .spawn()?;
                    for _ in 0..100 {
                        if ready.is_file() {
                            return Ok(());
                        }
                        if child.try_wait()?.is_some() {
                            return Err(io::Error::other("アップデーターを開始できませんでした"));
                        }
                        thread::sleep(std::time::Duration::from_millis(100));
                    }
                    Err(io::Error::other(
                        "アップデーターの応答がありません。現在のアプリは終了しません。",
                    ))
                })();
                match result {
                    Ok(()) => {
                        post(&tx, hwnd, Event::ExitForUpdate);
                        break;
                    }
                    Err(e) => post(&tx, hwnd, Event::InstallError(e.to_string())),
                }
            }
        }
    }
    for installer in installers {
        let _ = installer.join();
    }
}
pub fn run() -> io::Result<()> {
    unsafe {
        let mutex = CreateMutexW(
            ptr::null(),
            0,
            wide("Local\\EFTRegionWatcher.SingleInstance").as_ptr(),
        );
        if mutex.is_null() {
            return Err(io::Error::last_os_error());
        }
        let already = GetLastError() == ERROR_ALREADY_EXISTS;
        let _mutex = OwnedHandle(mutex);
        if already {
            let existing = FindWindowW(wide("EFTRegionWatcher.Window").as_ptr(), ptr::null());
            if !existing.is_null() {
                ShowWindow(existing, SW_SHOW);
                SetForegroundWindow(existing);
            }
            return Ok(());
        }
        let hr = CoInitializeEx(ptr::null(), COINIT_APARTMENTTHREADED as u32);
        if hr < 0 {
            return Err(io::Error::other("Windows UI の初期化に失敗しました"));
        }
        let controls = INITCOMMONCONTROLSEX {
            dwSize: mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_STANDARD_CLASSES,
        };
        InitCommonControlsEx(&controls);
        let instance = GetModuleHandleW(ptr::null());
        let class = wide("EFTRegionWatcher.Window");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
            hIcon: app_icon(instance, false),
            hbrBackground: ptr::null_mut(),
            ..mem::zeroed()
        };
        if RegisterClassW(&wc) == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut bounds = RECT {
            left: 0,
            top: 0,
            right: style::px(style::WIDTH),
            bottom: style::px(style::HEIGHT),
        };
        AdjustWindowRectEx(
            &mut bounds,
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN,
            0,
            0,
        );
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            wide("EFTRegionWatcher").as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            bounds.right - bounds.left,
            bounds.bottom - bounds.top,
            ptr::null_mut(),
            ptr::null_mut(),
            instance,
            ptr::null(),
        );
        if hwnd.is_null() {
            return Err(io::Error::last_os_error());
        }
        // The caption and Alt-Tab pick their own sizes; give each the frame drawn for it.
        SendMessageW(
            hwnd,
            WM_SETICON,
            ICON_SMALL as usize,
            app_icon(instance, true) as isize,
        );
        SendMessageW(
            hwnd,
            WM_SETICON,
            ICON_BIG as usize,
            app_icon(instance, false) as isize,
        );
        let dir = config::data_dir();
        std::fs::create_dir_all(&dir)?;
        let (cfg, warning) = match Config::load(&dir) {
            Ok(c) => (c, String::new()),
            Err(e) => {
                config::log(&dir, "WARN", &format!("設定読み込み: {e}"));
                (
                    Config::default(),
                    "設定ファイルを読み取れません。設定をご確認ください。".into(),
                )
            }
        };
        let tx = Inbox::default();
        let rx = tx.clone();
        let (jobs, jobs_rx) = mpsc::sync_channel(8);
        let cancel = CreateEventW(ptr::null(), 1, 0, ptr::null());
        if cancel.is_null() {
            return Err(io::Error::last_os_error());
        }
        let cancel = Arc::new(OwnedHandle(cancel));
        let worker_cancel = cancel.clone();
        let worker_tx = tx.clone();
        let worker_dir = dir.clone();
        let hwnd_value = hwnd as usize;
        let network = thread::spawn(move || {
            worker(
                jobs_rx,
                worker_tx,
                hwnd_value,
                worker_dir,
                worker_cancel,
                false,
            )
        });
        let mut tray: NOTIFYICONDATAW = mem::zeroed();
        tray.cbSize = mem::size_of_val(&tray) as u32;
        tray.hWnd = hwnd;
        tray.uID = 1;
        tray.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        tray.uCallbackMessage = TRAY;
        tray.hIcon = app_icon(instance, true);
        copy_wide(&mut tray.szTip, "EFTRegionWatcher — 待機中");
        if Shell_NotifyIconW(NIM_ADD, &tray) == 0 {
            ShowWindow(hwnd, SW_SHOW);
            MessageBoxW(
                hwnd,
                wide("トレイに表示できませんでした。ウィンドウから操作できます。").as_ptr(),
                wide("EFTRegionWatcher").as_ptr(),
                MB_OK | MB_ICONWARNING,
            );
        }
        let mut app = App {
            hwnd,
            buttons: Vec::new(),
            settings_open: false,
            config: cfg,
            dir,
            state: ConnectionState::Waiting,
            locations: Default::default(),
            detected: 0,
            history: History::default(),
            offset: 0,
            log_tab: false,
            focus_visible: false,
            warning,
            watcher: None,
            generation: 0,
            tx,
            jobs: Some(jobs),
            busy: false,
            checking: false,
            geo_inflight: false,
            stop: false,
            tray,
        };
        style::apply_theme(hwnd, app.config.dark_mode);
        app.rebuild_controls();
        app.start_watcher();
        app.render();
        ShowWindow(hwnd, SW_SHOW);
        if app.watcher.is_none() {
            PostMessageW(hwnd, WM_COMMAND, SELECT, 0);
        }
        if app.config.update_due(eft_region_watcher::now())
            && !app.config.update.repository.is_empty()
        {
            app.check(false);
        }
        let taskbar = RegisterWindowMessageW(wide("TaskbarCreated").as_ptr());
        let mut msg: MSG = mem::zeroed();
        while !app.stop {
            let status = GetMessageW(&mut msg, ptr::null_mut(), 0, 0);
            if status <= 0 {
                break;
            }
            // Focus rings belong to keyboard navigation; a click should not leave one behind.
            let keyboard = match msg.message {
                WM_KEYDOWN | WM_SYSKEYDOWN => Some(true),
                WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN => Some(false),
                _ => None,
            };
            if let Some(keyboard) = keyboard
                && app.focus_visible != keyboard
            {
                app.focus_visible = keyboard;
                app.render();
            }
            if msg.message == WM_KEYDOWN && msg.wParam == VK_ESCAPE as usize && app.settings_open {
                app.command(style::BACK);
            } else if msg.message == WM_KEYDOWN && msg.wParam == VK_RETURN as usize {
                let focused = GetFocus();
                if app.buttons.contains(&focused) {
                    app.command(GetDlgCtrlID(focused) as usize);
                }
            } else if msg.message == WM_COMMAND || msg.message == COMMAND {
                app.command(msg.wParam & 0xffff);
            } else if msg.message == WM_TIMER && msg.wParam == EXPIRE {
                KillTimer(hwnd, EXPIRE);
                app.state.mark_last_seen();
                app.render();
            } else if msg.message == taskbar {
                Shell_NotifyIconW(NIM_ADD, &app.tray);
            } else {
                if IsDialogMessageW(hwnd, &msg) == 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            while let Some(event) = rx.pop() {
                app.event(event);
                if app.stop {
                    break;
                }
            }
        }
        SetEvent(cancel.0);
        // Disconnect event receivers before joining: workers cannot block shutdown on a full queue.
        drop(rx);
        app.watcher.take();
        app.jobs.take();
        let _ = network.join();
        KillTimer(hwnd, EXPIRE);
        Shell_NotifyIconW(NIM_DELETE, &app.tray);
        DestroyWindow(hwnd);
        CoUninitialize();
        Ok(())
    }
}
/// Icon resource 1 (assets/icon.ico) at the size Windows asks for; falls back to the shell icon.
unsafe fn app_icon(instance: HMODULE, small: bool) -> HICON {
    unsafe {
        let metric = if small { SM_CXSMICON } else { SM_CXICON };
        let size = GetSystemMetrics(metric);
        let icon = LoadImageW(
            instance,
            ptr::without_provenance(1), // MAKEINTRESOURCEW(1)
            IMAGE_ICON,
            size,
            size,
            LR_DEFAULTCOLOR,
        );
        if icon.is_null() {
            LoadIconW(ptr::null_mut(), IDI_APPLICATION)
        } else {
            icon as HICON
        }
    }
}
// Mirrors the native control creation API.
#[allow(clippy::too_many_arguments)]
unsafe fn control(
    parent: HWND,
    class: &str,
    text: &str,
    style: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    id: usize,
) -> HWND {
    unsafe {
        let hwnd = CreateWindowExW(
            0,
            wide(class).as_ptr(),
            wide(text).as_ptr(),
            WS_CHILD | WS_VISIBLE | if class == "BUTTON" { WS_TABSTOP } else { 0 } | style,
            x,
            y,
            w,
            h,
            parent,
            id as HMENU,
            GetModuleHandleW(ptr::null()),
            ptr::null(),
        );
        SendMessageW(
            hwnd,
            WM_SETFONT,
            GetStockObject(DEFAULT_GUI_FONT) as usize,
            1,
        );
        hwnd
    }
}
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_ERASEBKGND => 1,
            WM_PAINT => {
                style::paint(hwnd);
                0
            }
            WM_DRAWITEM => {
                if l != 0 {
                    style::button(&*(l as *const DRAWITEMSTRUCT));
                }
                1
            }
            WM_COMMAND => {
                PostMessageW(hwnd, COMMAND, w, l);
                0
            }
            WM_MOUSEWHEEL => {
                let down = ((w >> 16) as u16 as i16) < 0;
                PostMessageW(hwnd, COMMAND, if down { SCROLL_DOWN } else { SCROLL_UP }, 0);
                0
            }
            WM_CLOSE => {
                PostMessageW(hwnd, COMMAND, CLOSE, 0);
                0
            }
            TRAY => {
                match l as u32 {
                    WM_LBUTTONUP | WM_LBUTTONDBLCLK => {
                        ShowWindow(hwnd, SW_SHOW);
                        SetForegroundWindow(hwnd);
                    }
                    WM_RBUTTONUP | WM_CONTEXTMENU => {
                        let menu = CreatePopupMenu();
                        for (id, text) in [
                            (OPEN, "開く"),
                            (OPEN, "現在のサーバー"),
                            (COPY, "IP をコピー"),
                            (CHECK, "更新を確認"),
                            (SETTINGS, "設定"),
                            (EXIT, "終了"),
                        ] {
                            AppendMenuW(menu, MF_STRING, id, wide(text).as_ptr());
                        }
                        let mut p = POINT { x: 0, y: 0 };
                        GetCursorPos(&mut p);
                        SetForegroundWindow(hwnd);
                        let chosen = TrackPopupMenu(
                            menu,
                            TPM_RETURNCMD | TPM_RIGHTBUTTON,
                            p.x,
                            p.y,
                            0,
                            hwnd,
                            ptr::null(),
                        );
                        DestroyMenu(menu);
                        if chosen > 0 {
                            PostMessageW(hwnd, WM_COMMAND, chosen as usize, 0);
                        }
                        PostMessageW(hwnd, WM_NULL, 0, 0);
                    }
                    _ => {}
                }
                0
            }
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}
impl App {
    fn save(&mut self) {
        if let Err(e) = self.config.save(&self.dir) {
            self.warning = "設定を保存できません。フォルダのアクセス権をご確認ください。".into();
            config::log(&self.dir, "WARN", &format!("設定保存: {e}"));
        }
    }
    fn start_watcher(&mut self) {
        self.watcher.take();
        self.generation += 1;
        self.state.mark_last_seen();
        let Some(root) = eft::finder::find(self.config.eft_log_path.as_deref()) else {
            self.warning = "EFT のログフォルダが見つかりません。設定から選択してください。".into();
            return;
        };
        self.config.eft_log_path = Some(root.clone());
        self.save();
        let tx = self.tx.clone();
        let hwnd = self.hwnd as usize;
        let generation = self.generation;
        if let Ok(mut h) = self.history.lock() {
            h.clear();
        }
        self.offset = 0;
        let history = self.history.clone();
        // The inbox collapses queued endpoint events, so history is recorded before posting.
        match LogWatcher::start(root, move |e| {
            if let LogEvent::Endpoint {
                endpoint,
                detected_at,
                ..
            } = &e
                && let Ok(mut h) = history.lock()
                && h.last().is_none_or(|(last, _)| last != endpoint)
            {
                if h.len() >= HISTORY_MAX {
                    h.remove(0);
                }
                h.push((endpoint.clone(), *detected_at));
            }
            post(&tx, hwnd, Event::Log(generation, e))
        }) {
            Ok(w) => {
                self.watcher = Some(w);
                self.warning.clear();
            }
            Err(e) => {
                self.warning = format!("ログを監視できません: {e}");
            }
        }
    }
    fn rebuild_controls(&mut self) {
        unsafe {
            for handle in self.buttons.drain(..) {
                DestroyWindow(handle);
            }
            for (id, label, x, y, w, h) in style::button_specs(self.settings_open) {
                self.buttons.push(control(
                    self.hwnd,
                    "BUTTON",
                    label,
                    BS_OWNERDRAW as u32,
                    style::px(x),
                    style::px(y),
                    style::px(w),
                    style::px(h),
                    id,
                ));
            }
            if let Some(button) = self.buttons.first() {
                SetFocus(*button);
            }
        }
    }
    fn render(&mut self) {
        let connected = matches!(self.state, ConnectionState::Connected(_));
        let has_endpoint = self.state.endpoint().is_some();
        let endpoint = self
            .state
            .endpoint()
            .map(ToString::to_string)
            .unwrap_or_else(|| "まだ検出されていません".into());
        let current = self
            .state
            .endpoint()
            .and_then(|e| self.locations.get(&e.ip))
            .and_then(Option::as_ref)
            .cloned();
        let region = current.as_ref().map(Location::label).unwrap_or_else(|| {
            if has_endpoint {
                "地域を取得できません".into()
            } else {
                "接続先を待っています".into()
            }
        });
        // The ASN and organization are noise for the reader; only guidance goes here.
        let provider = if current.is_some() {
            String::new()
        } else if has_endpoint {
            "接続先の IP はそのまま利用できます。".into()
        } else {
            "EFT の接続ログを検出すると、推定地域を表示します。".into()
        };
        let time = if self.detected == 0 {
            String::new()
        } else {
            format!(
                "{}  {}",
                if connected { "検出" } else { "最終記録" },
                local_time(self.detected)
            )
        };
        let entries = self.history.lock().map(|h| h.clone()).unwrap_or_default();
        let total = entries.len();
        self.offset = self.offset.min(total.saturating_sub(HISTORY_ROWS));
        let history = entries
            .iter()
            .rev()
            .skip(self.offset)
            .take(HISTORY_ROWS)
            .map(|(e, at)| {
                let region = match self.locations.get(&e.ip) {
                    Some(Some(l)) => l.label(),
                    Some(None) => "地域を取得できません".into(),
                    None => "地域を取得しています…".into(),
                };
                (e.to_string(), region, local_time(*at))
            })
            .collect();
        self.request_geo();
        let view = style::View {
            dark: self.config.dark_mode,
            settings: self.settings_open,
            log_tab: self.log_tab,
            focus: self.focus_visible,
            connected,
            has_endpoint,
            watching: self.watcher.is_some(),
            region,
            provider,
            endpoint: endpoint.clone(),
            time,
            history,
            history_total: total,
            history_offset: self.offset,
            notice: self.warning.clone(),
            path: self
                .config
                .eft_log_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "ログフォルダを選択してください".into()),
            resident: self.config.minimize_to_tray_on_close,
            notifications: self.config.connection_notifications,
            auto_update: self.config.update.enabled,
        };
        if style::snapshot().dark != view.dark {
            style::apply_theme(self.hwnd, view.dark);
        }
        style::set_view(view);
        unsafe {
            for &button in &self.buttons {
                let id = GetDlgCtrlID(button) as usize;
                let toggle = match id {
                    style::DARK => Some(("ダークモード", self.config.dark_mode)),
                    style::RESIDENT => Some((
                        "閉じてもトレイに残す",
                        self.config.minimize_to_tray_on_close,
                    )),
                    style::NOTIFY => Some(("接続通知", self.config.connection_notifications)),
                    style::AUTO_UPDATE => Some(("自動更新チェック", self.config.update.enabled)),
                    _ => None,
                };
                if let Some((label, on)) = toggle {
                    SetWindowTextW(
                        button,
                        wide(&format!("{label}: {}", if on { "オン" } else { "オフ" })).as_ptr(),
                    );
                }
                InvalidateRect(button, ptr::null(), 0);
            }
            InvalidateRect(self.hwnd, ptr::null(), 0);
            copy_wide(
                &mut self.tray.szTip,
                &format!(
                    "EFTRegionWatcher\n{}\n{endpoint}",
                    if connected {
                        "接続を検出"
                    } else {
                        "待機中"
                    }
                ),
            );
            self.tray.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            Shell_NotifyIconW(NIM_MODIFY, &self.tray);
        }
    }
    fn command(&mut self, id: usize) {
        match id {
            OPEN => unsafe {
                ShowWindow(self.hwnd, SW_SHOW);
                SetForegroundWindow(self.hwnd);
            },
            COPY => self.copy(),
            CHECK => self.check(true),
            SETTINGS => self.settings(),
            style::BACK => {
                self.settings_open = false;
                self.rebuild_controls();
                self.render();
            }
            style::DARK => {
                self.config.dark_mode = !self.config.dark_mode;
                self.save();
                self.render();
            }
            style::RESIDENT => {
                self.config.minimize_to_tray_on_close = !self.config.minimize_to_tray_on_close;
                self.save();
                self.render();
            }
            style::NOTIFY => {
                self.config.connection_notifications = !self.config.connection_notifications;
                self.save();
                self.render();
            }
            style::AUTO_UPDATE => {
                self.config.update.enabled = !self.config.update.enabled;
                self.save();
                self.render();
            }
            SELECT => self.select(),
            style::TAB_NOW | style::TAB_LOG => {
                self.log_tab = id == style::TAB_LOG;
                self.render();
            }
            SCROLL_UP | SCROLL_DOWN => {
                let total = self.history.lock().map(|h| h.len()).unwrap_or(0);
                let next = page(self.offset, total, id == SCROLL_DOWN);
                if next != self.offset {
                    self.offset = next;
                    self.render();
                }
            }
            EXIT => self.stop = true,
            CLOSE => {
                if self.config.minimize_to_tray_on_close {
                    unsafe {
                        ShowWindow(self.hwnd, SW_HIDE);
                    }
                } else {
                    self.stop = true;
                }
            }
            _ => {}
        }
    }
    /// Resolves one address at a time: the current endpoint first, then the visible history rows.
    fn request_geo(&mut self) {
        if self.geo_inflight {
            return;
        }
        let visible = self
            .history
            .lock()
            .map(|h| {
                h.iter()
                    .rev()
                    .skip(self.offset)
                    .take(HISTORY_ROWS)
                    .map(|(e, _)| e.ip)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let Some(ip) = self
            .state
            .endpoint()
            .map(|e| e.ip)
            .into_iter()
            .chain(visible)
            .find(|ip| !self.locations.contains_key(ip))
        else {
            return;
        };
        self.geo_inflight = self
            .jobs
            .as_ref()
            .is_some_and(|j| j.try_send(Job::Geo(ip)).is_ok());
    }
    fn select(&mut self) {
        if let Some(path) = folder(self.hwnd) {
            self.config.eft_log_path = Some(path);
            self.start_watcher();
            self.render();
        }
    }
    fn settings(&mut self) {
        self.settings_open = true;
        self.rebuild_controls();
        self.render();
    }
    fn check(&mut self, manual: bool) {
        if self.checking || self.busy {
            return;
        }
        if !update::valid_repository(&self.config.update.repository) {
            if manual {
                dialog(
                    self.hwnd,
                    "更新先が未設定です",
                    "正式な公開リリースでは更新先が自動設定されます。",
                    &[(1, "閉じる")],
                    1,
                );
            }
            return;
        }
        if self.jobs.as_ref().is_some_and(|j| {
            j.try_send(Job::Check(manual, self.config.update.repository.clone()))
                .is_ok()
        }) {
            self.checking = true;
            self.config.update.last_check = eft_region_watcher::now();
            self.save();
            if manual {
                self.warning = "更新を確認しています…".into();
                self.render();
            }
        }
    }
    fn event(&mut self, event: Event) {
        match event {
            Event::Log(generation, event) => {
                if generation != self.generation {
                    return;
                }
                match event {
                    LogEvent::Endpoint {
                        endpoint,
                        historical,
                        detected_at,
                    } => {
                        let same = self.state.endpoint() == Some(&endpoint);
                        // A live line is a fresh chance to resolve an address that failed before.
                        if !historical && matches!(self.locations.get(&endpoint.ip), Some(None)) {
                            self.locations.remove(&endpoint.ip);
                        }
                        self.state = if historical {
                            ConnectionState::LastSeen(endpoint.clone())
                        } else {
                            ConnectionState::Connected(endpoint.clone())
                        };
                        if !same || !historical {
                            self.detected = detected_at;
                        }
                        self.warning.clear();
                        if !historical {
                            unsafe {
                                SetTimer(self.hwnd, EXPIRE, 300000, None);
                            }
                        }
                        if !same {
                            self.offset = 0;
                            self.request_geo();
                            if !historical && self.config.connection_notifications {
                                self.notify(&format!("サーバーを検出しました\n{endpoint}"));
                            }
                        }
                        self.render();
                    }
                    LogEvent::Unavailable(e) => {
                        config::log(&self.dir, "WARN", &e);
                        self.warning = e;
                        self.state.mark_last_seen();
                        self.render();
                    }
                }
            }
            Event::Geo(ip, result) => {
                self.geo_inflight = false;
                match result {
                    Ok(l) => {
                        self.locations.insert(ip, Some(l));
                    }
                    Err(e) => {
                        self.locations.insert(ip, None);
                        if self.state.endpoint().is_some_and(|e| e.ip == ip) {
                            self.warning =
                                "推定地域を取得できません。IP はそのまま利用できます。".into();
                        }
                        config::log(&self.dir, "WARN", &format!("GeoIP: {e}"));
                    }
                }
                // ponytail: unbounded map, capped by the watcher's own history limit.
                self.render();
            }
            Event::Checked(manual, result) => {
                self.checking = false;
                self.warning.clear();
                self.render();
                match result {
                    Ok(release) => match update::available(
                        env!("CARGO_PKG_VERSION"),
                        &release,
                        &self.config.update.skipped_version,
                    ) {
                        Ok(true) => {
                            let content = format!(
                                "現在: v{}\n最新: {}\n\n変更内容:\n{}",
                                env!("CARGO_PKG_VERSION"),
                                release.tag_name,
                                update::release_notes(&release)
                            );
                            match dialog(
                                self.hwnd,
                                "新しいバージョンがあります",
                                &content,
                                &[
                                    (301, "更新する"),
                                    (302, "あとで"),
                                    (303, "このバージョンをスキップ"),
                                ],
                                302,
                            ) {
                                301 => self.download(release),
                                303 => {
                                    self.config.update.skipped_version =
                                        release.tag_name.trim_start_matches('v').into();
                                    self.save();
                                }
                                _ => {}
                            }
                        }
                        Ok(false) => {
                            if manual {
                                dialog(
                                    self.hwnd,
                                    "更新確認",
                                    "利用可能な更新はありません（スキップ済みのバージョンを除きます）。",
                                    &[(1, "閉じる")],
                                    1,
                                );
                            }
                        }
                        Err(e) => self.update_error(manual, &e.to_string()),
                    },
                    Err(e) => self.update_error(manual, &e),
                }
            }
            Event::InstallError(e) => {
                self.busy = false;
                self.warning.clear();
                self.update_error(true, &e);
                self.render();
            }
            Event::ExitForUpdate => self.stop = true,
        }
    }
    fn download(&mut self, release: Release) {
        if self.busy {
            return;
        }
        let mut games = match platform::game_processes() {
            Ok(p) => p,
            Err(e) => {
                self.update_error(true,&format!("EFT の実行状況を確認できません。ゲームを終了してから再試行してください。\n{e}"));
                return;
            }
        };
        if !games.is_empty() {
            match dialog(
                self.hwnd,
                "Escape from Tarkov が起動中です",
                "ゲームを終了してからの更新をおすすめします。ゲームを終了させる操作は行いません。",
                &[
                    (401, "EFT 終了後に更新（推奨）"),
                    (402, "今すぐ更新"),
                    (403, "キャンセル"),
                ],
                401,
            ) {
                401 => {}
                402 => games.clear(),
                _ => return,
            }
        }
        let waiting = !games.is_empty();
        if self.jobs.as_ref().is_some_and(|j| {
            j.try_send(Job::Download(
                release,
                self.config.update.repository.clone(),
                games,
            ))
            .is_ok()
        }) {
            self.busy = true;
            self.warning = if waiting {
                "EFT の終了を待っています。アプリを終了するとキャンセルします。"
            } else {
                "更新をダウンロードしています…"
            }
            .into();
            self.render();
        }
    }
    fn update_error(&self, manual: bool, message: &str) {
        config::log(&self.dir, "WARN", &format!("更新: {message}"));
        if manual {
            dialog(
                self.hwnd,
                "更新できませんでした",
                &format!("現在のバージョンは引き続き利用できます。\n\n{message}"),
                &[(1, "閉じる")],
                1,
            );
        }
    }
    fn notify(&mut self, message: &str) {
        unsafe {
            self.tray.uFlags = NIF_INFO;
            copy_wide(&mut self.tray.szInfoTitle, "EFTRegionWatcher");
            copy_wide(&mut self.tray.szInfo, message);
            self.tray.dwInfoFlags = NIIF_INFO | NIIF_NOSOUND;
            Shell_NotifyIconW(NIM_MODIFY, &self.tray);
        }
    }
    fn copy(&self) {
        if let Some(e) = self.state.endpoint()
            && let Err(error) = clipboard(self.hwnd, &e.ip.to_string())
        {
            self.update_error(true, &format!("IP をコピーできませんでした: {error}"));
        }
    }
}
/// One page of history per wheel notch, clamped to the last full page.
fn page(offset: usize, total: usize, down: bool) -> usize {
    if down {
        (offset + HISTORY_ROWS).min(total.saturating_sub(HISTORY_ROWS))
    } else {
        offset.saturating_sub(HISTORY_ROWS)
    }
}
fn copy_wide<const N: usize>(dest: &mut [u16; N], text: &str) {
    dest.fill(0);
    for (i, c) in text.encode_utf16().take(N - 1).enumerate() {
        dest[i] = c;
    }
}
fn dialog(hwnd: HWND, title: &str, content: &str, buttons: &[(i32, &str)], default: i32) -> i32 {
    style::modal(hwnd, title, content, buttons, default)
}
fn folder(hwnd: HWND) -> Option<PathBuf> {
    unsafe {
        let title = wide("Escape from Tarkov の Logs フォルダを選んでください");
        let mut info: BROWSEINFOW = mem::zeroed();
        info.hwndOwner = hwnd;
        info.lpszTitle = title.as_ptr();
        info.ulFlags = BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE;
        let pidl = SHBrowseForFolderW(&info);
        if pidl.is_null() {
            return None;
        }
        let mut buf = [0u16; 32768];
        let ok = SHGetPathFromIDListEx(pidl, buf.as_mut_ptr(), buf.len() as u32, 0);
        CoTaskMemFree(pidl.cast());
        if ok == 0 {
            return None;
        }
        let end = buf.iter().position(|c| *c == 0)?;
        use std::os::windows::ffi::OsStringExt;
        Some(std::ffi::OsString::from_wide(&buf[..end]).into())
    }
}
fn clipboard(hwnd: HWND, text: &str) -> io::Result<()> {
    unsafe {
        if OpenClipboard(hwnd) == 0 {
            return Err(io::Error::last_os_error());
        }
        let result = (|| {
            let text = wide(text);
            let h = GlobalAlloc(GMEM_MOVEABLE, text.len() * 2);
            if h.is_null() {
                return Err(io::Error::last_os_error());
            }
            let data = GlobalLock(h);
            if data.is_null() {
                GlobalFree(h);
                return Err(io::Error::last_os_error());
            }
            ptr::copy_nonoverlapping(text.as_ptr(), data.cast(), text.len());
            GlobalUnlock(h);
            if EmptyClipboard() == 0 || SetClipboardData(13, h).is_null() {
                GlobalFree(h);
                return Err(io::Error::last_os_error());
            }
            Ok(())
        })();
        CloseClipboard();
        result
    }
}
fn local_time(unix: u64) -> String {
    unsafe {
        let ticks = (unix + 11644473600) * 10_000_000;
        let utc = FILETIME {
            dwLowDateTime: ticks as u32,
            dwHighDateTime: (ticks >> 32) as u32,
        };
        let mut local: FILETIME = mem::zeroed();
        let mut time: SYSTEMTIME = mem::zeroed();
        if windows_sys::Win32::Storage::FileSystem::FileTimeToLocalFileTime(&utc, &mut local) != 0
            && windows_sys::Win32::System::Time::FileTimeToSystemTime(&local, &mut time) != 0
        {
            format!(
                "{:04}/{:02}/{:02} {:02}:{:02}:{:02}",
                time.wYear, time.wMonth, time.wDay, time.wHour, time.wMinute, time.wSecond
            )
        } else {
            unix.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_button_commands_reach_application_queue() {
        unsafe {
            let class = wide("EFTRegionWatcher.DispatchTest");
            let instance = GetModuleHandleW(ptr::null());
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                ..mem::zeroed()
            };
            assert_ne!(RegisterClassW(&wc), 0);
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                class.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                ptr::null_mut(),
                instance,
                ptr::null(),
            );
            assert!(!hwnd.is_null());
            SendMessageW(hwnd, WM_COMMAND, COPY, 0);
            let mut msg: MSG = mem::zeroed();
            assert_ne!(PeekMessageW(&mut msg, hwnd, COMMAND, COMMAND, PM_REMOVE), 0);
            assert_eq!(msg.wParam, COPY);
            SendMessageW(hwnd, WM_CLOSE, 0, 0);
            assert_ne!(PeekMessageW(&mut msg, hwnd, COMMAND, COMMAND, PM_REMOVE), 0);
            assert_eq!(msg.wParam, CLOSE);
            DestroyWindow(hwnd);
            UnregisterClassW(class.as_ptr(), instance);
        }
    }
    #[test]
    fn modal_backlog_keeps_latest_endpoint() {
        let inbox = Inbox::default();
        for port in 1..1000 {
            inbox.push(Event::Log(
                1,
                LogEvent::Endpoint {
                    endpoint: eft::ServerEndpoint {
                        ip: "203.0.113.10".parse().unwrap(),
                        port: Some(port),
                    },
                    historical: false,
                    detected_at: 1,
                },
            ));
        }
        assert_eq!(inbox.0.lock().unwrap().len(), 1);
        match inbox.pop().unwrap() {
            Event::Log(_, LogEvent::Endpoint { endpoint, .. }) => {
                assert_eq!(endpoint.port, Some(999))
            }
            _ => panic!("wrong event"),
        }
    }

    #[test]
    fn history_pages_by_five_and_clamps() {
        assert_eq!(page(0, 12, true), 5);
        assert_eq!(page(5, 12, true), 7);
        assert_eq!(page(7, 12, true), 7);
        assert_eq!(page(7, 12, false), 2);
        assert_eq!(page(2, 12, false), 0);
        assert_eq!(page(0, 3, true), 0);
    }
    fn test_app(dir: PathBuf) -> App {
        App {
            hwnd: ptr::null_mut(),
            buttons: Vec::new(),
            settings_open: false,
            config: Config::default(),
            dir,
            state: ConnectionState::Waiting,
            locations: Default::default(),
            detected: 0,
            history: History::default(),
            offset: 0,
            log_tab: false,
            focus_visible: false,
            warning: String::new(),
            watcher: None,
            generation: 0,
            tx: Inbox::default(),
            jobs: None,
            busy: false,
            checking: false,
            geo_inflight: false,
            stop: false,
            tray: unsafe { mem::zeroed() },
        }
    }
    #[test]
    fn close_respects_residency_and_exit_always_exits() {
        let mut app = test_app(std::env::temp_dir());
        app.command(CLOSE); // default: closing exits
        assert!(app.stop);
        app.stop = false;
        app.config.minimize_to_tray_on_close = true;
        app.command(CLOSE);
        assert!(!app.stop);
        app.command(EXIT);
        assert!(app.stop);
    }
}
