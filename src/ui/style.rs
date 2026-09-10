//! Event-driven GDI presentation. No rendering loop, animation, or GPU surface.
use super::*;
use std::cell::RefCell;
use windows_sys::Win32::Graphics::Dwm::*;
use windows_sys::Win32::Graphics::GdiPlus::*;
use windows_sys::Win32::UI::HiDpi::*;
pub const WIDTH: i32 = 480;
pub const HEIGHT: i32 = 592;
pub const BACK: usize = 210;
pub const DARK: usize = 211;
pub const RESIDENT: usize = 212;
pub const NOTIFY: usize = 213;
pub const AUTO_UPDATE: usize = 214;
pub const TAB_NOW: usize = 215;
pub const TAB_LOG: usize = 216;
#[derive(Clone)]
pub struct View {
    pub dark: bool,
    pub settings: bool,
    pub log_tab: bool,
    /// Focus rings show only while the keyboard is driving the UI.
    pub focus: bool,
    pub connected: bool,
    pub has_endpoint: bool,
    pub watching: bool,
    pub region: String,
    pub provider: String,
    pub endpoint: String,
    pub time: String,
    /// Newest first: (address, region, detected time). At most HISTORY_ROWS entries.
    pub history: Vec<(String, String, String)>,
    pub history_total: usize,
    pub history_offset: usize,
    pub notice: String,
    pub path: String,
    pub resident: bool,
    pub notifications: bool,
    pub auto_update: bool,
}
impl Default for View {
    fn default() -> Self {
        Self {
            dark: true,
            settings: false,
            log_tab: false,
            focus: false,
            connected: false,
            has_endpoint: false,
            watching: false,
            region: "接続先を待っています".into(),
            provider: "EFT の接続ログを検出すると、推定地域を表示します。".into(),
            endpoint: "まだ検出されていません".into(),
            time: String::new(),
            history: Vec::new(),
            history_total: 0,
            history_offset: 0,
            notice: String::new(),
            path: "ログフォルダを選択してください".into(),
            resident: false,
            notifications: false,
            auto_update: true,
        }
    }
}
thread_local! {static VIEW:RefCell<View>=RefCell::new(View::default());}
pub fn snapshot() -> View {
    VIEW.with(|v| v.borrow().clone())
}
pub fn set_view(view: View) {
    VIEW.with(|v| *v.borrow_mut() = view);
}
#[derive(Clone, Copy)]
struct Colors {
    bg: u32,
    card: u32,
    inset: u32,
    border: u32,
    text: u32,
    muted: u32,
    accent: u32,
    accent_text: u32,
    tint: u32,
}
fn rgb(r: u32, g: u32, b: u32) -> u32 {
    r | (g << 8) | (b << 16)
}
fn palette(dark: bool) -> Colors {
    if dark {
        Colors {
            bg: rgb(17, 21, 26),
            card: rgb(25, 31, 38),
            inset: rgb(32, 39, 47),
            border: rgb(47, 57, 67),
            text: rgb(235, 242, 247),
            muted: rgb(151, 166, 180),
            accent: rgb(122, 222, 184),
            accent_text: rgb(16, 43, 34),
            tint: rgb(28, 55, 46),
        }
    } else {
        Colors {
            bg: rgb(244, 247, 248),
            card: rgb(255, 255, 255),
            inset: rgb(237, 242, 244),
            border: rgb(221, 229, 233),
            text: rgb(26, 40, 50),
            muted: rgb(92, 112, 126),
            accent: rgb(23, 113, 84),
            accent_text: rgb(255, 255, 255),
            tint: rgb(227, 242, 234),
        }
    }
}
pub fn dpi() -> i32 {
    unsafe { GetDpiForSystem() as i32 }.max(96)
}
pub fn px(value: i32) -> i32 {
    value * dpi() / 96
}
pub fn apply_theme(hwnd: HWND, dark: bool) {
    unsafe {
        let value = if dark { 1i32 } else { 0 };
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
            (&value as *const i32).cast(),
            4,
        );
        let p = palette(dark);
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_CAPTION_COLOR as u32,
            (&p.bg as *const u32).cast(),
            4,
        );
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_TEXT_COLOR as u32,
            (&p.muted as *const u32).cast(),
            4,
        );
    }
}
fn rect(x: i32, y: i32, w: i32, h: i32) -> RECT {
    RECT {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    }
}
unsafe fn fill(dc: HDC, r: RECT, color: u32) {
    unsafe {
        let brush = CreateSolidBrush(color);
        FillRect(dc, &r, brush);
        DeleteObject(brush);
    }
}
/// GDI has no antialiasing, so curves go through GDI+; text stays on GDI for ClearType.
/// ponytail: started once per process and never shut down; the process exit reclaims it.
fn graphics(dc: HDC) -> *mut GpGraphics {
    thread_local! {static STARTED: bool = {
        let mut token = 0usize;
        let input = GdiplusStartupInput {
            GdiplusVersion: 1,
            ..Default::default()
        };
        unsafe { GdiplusStartup(&mut token, &input, ptr::null_mut()) == 0 }
    }}
    if !STARTED.with(|s| *s) {
        return ptr::null_mut();
    }
    let mut g = ptr::null_mut();
    unsafe {
        if GdipCreateFromHDC(dc, &mut g) != 0 {
            return ptr::null_mut();
        }
        GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
    }
    g
}
/// COLORREF (0x00BBGGRR) to GDI+ ARGB.
fn argb(color: u32) -> u32 {
    0xff00_0000 | ((color & 0xff) << 16) | (color & 0xff00) | ((color >> 16) & 0xff)
}
unsafe fn rounded(dc: HDC, r: RECT, fill_color: u32, border: u32, radius: i32) {
    unsafe {
        let g = graphics(dc);
        if g.is_null() {
            let brush = CreateSolidBrush(fill_color);
            let pen = CreatePen(PS_SOLID, 1, border);
            let old_brush = SelectObject(dc, brush);
            let old_pen = SelectObject(dc, pen);
            RoundRect(dc, r.left, r.top, r.right, r.bottom, radius, radius);
            SelectObject(dc, old_brush);
            SelectObject(dc, old_pen);
            DeleteObject(brush);
            DeleteObject(pen);
            return;
        }
        let (x, y) = (r.left as f32 + 0.5, r.top as f32 + 0.5);
        let w = (r.right - r.left) as f32 - 1.0;
        let h = (r.bottom - r.top) as f32 - 1.0;
        let d = (radius as f32).min(w).min(h);
        let mut path = ptr::null_mut();
        GdipCreatePath(FillModeAlternate, &mut path);
        GdipAddPathArc(path, x, y, d, d, 180.0, 90.0);
        GdipAddPathArc(path, x + w - d, y, d, d, 270.0, 90.0);
        GdipAddPathArc(path, x + w - d, y + h - d, d, d, 0.0, 90.0);
        GdipAddPathArc(path, x, y + h - d, d, d, 90.0, 90.0);
        GdipClosePathFigure(path);
        let mut brush = ptr::null_mut();
        GdipCreateSolidFill(argb(fill_color), &mut brush);
        GdipFillPath(g, brush.cast(), path);
        GdipDeleteBrush(brush.cast());
        if border != fill_color {
            let mut pen = ptr::null_mut();
            GdipCreatePen1(argb(border), 1.0, UnitPixel, &mut pen);
            GdipDrawPath(g, pen, path);
            GdipDeletePen(pen);
        }
        GdipDeletePath(path);
        GdipDeleteGraphics(g);
    }
}
/// An installed Noto Sans JP, else the bundled copy, else Meiryo UI.
/// The installed check comes first so the embedded font is never paged in when the
/// system already has it.
/// ponytail: once registered, the memory font stays for the life of the process.
pub fn ui_font() -> &'static str {
    thread_local! {static FACE: &'static str = unsafe {
        unsafe extern "system" fn found(
            _: *const LOGFONTW,
            _: *const TEXTMETRICW,
            _: u32,
            found: LPARAM,
        ) -> i32 {
            unsafe { *(found as *mut bool) = true };
            0
        }
        let dc = GetDC(ptr::null_mut());
        let mut logfont: LOGFONTW = mem::zeroed();
        logfont.lfCharSet = DEFAULT_CHARSET;
        copy_wide(&mut logfont.lfFaceName, "Noto Sans JP");
        let mut installed = false;
        EnumFontFamiliesExW(
            dc,
            &logfont,
            Some(found),
            (&raw mut installed) as LPARAM,
            0,
        );
        ReleaseDC(ptr::null_mut(), dc);
        if installed {
            return "Noto Sans JP";
        }
        const NOTO: &[u8] = include_bytes!("../../assets/fonts/NotoSansJP-VF.ttf");
        let mut fonts = 0u32;
        let loaded = AddFontMemResourceEx(
            NOTO.as_ptr().cast(),
            NOTO.len() as u32,
            ptr::null(),
            (&raw mut fonts) as *const u32,
        );
        if !loaded.is_null() && fonts > 0 {
            "Noto Sans JP"
        } else {
            "Meiryo UI"
        }
    }}
    FACE.with(|f| *f)
}
#[allow(clippy::too_many_arguments)]
unsafe fn text(dc: HDC, value: &str, r: RECT, size: i32, bold: bool, color: u32, flags: u32) {
    unsafe {
        let font = CreateFontW(
            -size,
            0,
            0,
            0,
            if bold { 600 } else { 400 },
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            DEFAULT_PITCH as u32,
            wide(ui_font()).as_ptr(),
        );
        let old = SelectObject(dc, font);
        SetBkMode(dc, TRANSPARENT as i32);
        SetTextColor(dc, color);
        let mut r = r;
        let value = wide(value);
        DrawTextW(
            dc,
            value.as_ptr(),
            value.len() as i32 - 1,
            &mut r,
            flags | DT_NOPREFIX,
        );
        SelectObject(dc, old);
        DeleteObject(font);
    }
}
unsafe fn line_text(dc: HDC, value: &str, r: RECT, size: i32, bold: bool, color: u32) {
    unsafe {
        text(
            dc,
            value,
            r,
            size,
            bold,
            color,
            DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
        );
    }
}
/// Traces the control's own outline one pixel inside it, so ring and frame stay concentric.
unsafe fn focus_ring(dc: HDC, w: i32, h: i32, radius: i32, c: Colors) {
    unsafe {
        let g = graphics(dc);
        if g.is_null() {
            let pen = CreatePen(PS_SOLID, 1, c.accent);
            let old_pen = SelectObject(dc, pen);
            let old_brush = SelectObject(dc, GetStockObject(HOLLOW_BRUSH));
            RoundRect(dc, 2, 2, w - 2, h - 2, radius, radius);
            SelectObject(dc, old_pen);
            SelectObject(dc, old_brush);
            DeleteObject(pen);
            return;
        }
        let (x, y) = (2.5, 2.5);
        let (rw, rh) = (w as f32 - 5.0, h as f32 - 5.0);
        let d = ((radius - 5) as f32).max(2.0).min(rw).min(rh);
        let mut path = ptr::null_mut();
        GdipCreatePath(FillModeAlternate, &mut path);
        GdipAddPathArc(path, x, y, d, d, 180.0, 90.0);
        GdipAddPathArc(path, x + rw - d, y, d, d, 270.0, 90.0);
        GdipAddPathArc(path, x + rw - d, y + rh - d, d, d, 0.0, 90.0);
        GdipAddPathArc(path, x, y + rh - d, d, d, 90.0, 90.0);
        GdipClosePathFigure(path);
        let mut pen = ptr::null_mut();
        GdipCreatePen1(argb(c.accent), 1.0, UnitPixel, &mut pen);
        GdipDrawPath(g, pen, path);
        GdipDeletePen(pen);
        GdipDeletePath(path);
        GdipDeleteGraphics(g);
    }
}
unsafe fn dot(dc: HDC, x: i32, y: i32, color: u32) {
    unsafe {
        rounded(dc, rect(x, y, 8, 8), color, color, 8);
    }
}
unsafe fn toggle(dc: HDC, x: i32, y: i32, on: bool, c: Colors) {
    unsafe {
        rounded(
            dc,
            rect(x, y, 42, 24),
            if on { c.accent } else { c.border },
            if on { c.accent } else { c.border },
            24,
        );
        rounded(
            dc,
            rect(x + if on { 22 } else { 4 }, y + 4, 16, 16),
            if on { c.accent_text } else { c.card },
            if on { c.accent_text } else { c.card },
            16,
        );
    }
}
/// Log tab: HISTORY_ROWS rows of address, region and time; one page per wheel notch.
unsafe fn history(dc: HDC, v: &View, c: Colors) {
    unsafe {
        const ROW: i32 = 48;
        if v.history_total > 0 {
            let first = v.history_offset + 1;
            let last = (v.history_offset + v.history.len()).max(first);
            text(
                dc,
                &format!("{} 件中 {first}–{last}", v.history_total),
                rect(232, 152, 220, 20),
                11,
                false,
                c.muted,
                DT_SINGLELINE | DT_VCENTER | DT_RIGHT,
            );
        }
        rounded(dc, rect(28, 178, 424, 252), c.card, c.border, 16);
        if v.history.is_empty() {
            text(
                dc,
                "まだ履歴がありません",
                rect(28, 178, 424, 252),
                12,
                false,
                c.muted,
                DT_SINGLELINE | DT_VCENTER | DT_CENTER,
            );
            return;
        }
        for (i, (address, region, at)) in v.history.iter().enumerate() {
            let y = 182 + i as i32 * ROW;
            let newest = i == 0 && v.history_offset == 0;
            dot(
                dc,
                48,
                y + 20,
                if newest && v.connected {
                    c.accent
                } else {
                    c.border
                },
            );
            line_text(dc, address, rect(68, y + 5, 220, 20), 14, newest, c.text);
            line_text(dc, region, rect(68, y + 26, 220, 18), 11, false, c.muted);
            text(
                dc,
                at,
                rect(292, y + 15, 140, 20),
                11,
                false,
                c.muted,
                DT_SINGLELINE | DT_VCENTER | DT_RIGHT,
            );
            if i + 1 < v.history.len() {
                fill(dc, rect(48, y + ROW - 1, 384, 1), c.border);
            }
        }
        // Scrollbar: proportional thumb over the whole list, drawn only when it can move.
        if v.history_total > v.history.len() {
            const TRACK: i32 = 236;
            let thumb = (TRACK * v.history.len() as i32 / v.history_total as i32).max(24);
            let travel = TRACK - thumb;
            let scrolled = v.history_total - v.history.len();
            let y = 186 + travel * v.history_offset.min(scrolled) as i32 / scrolled as i32;
            rounded(dc, rect(438, 186, 4, TRACK), c.inset, c.inset, 4);
            rounded(dc, rect(438, y, 4, thumb), c.muted, c.muted, 4);
        }
    }
}
/// Coordinates are in logical pixels and shared by live painting and render tests.
pub unsafe fn canvas(dc: HDC, v: &View) {
    unsafe {
        let c = palette(v.dark);
        fill(dc, rect(0, 0, WIDTH, HEIGHT), c.bg);
        // The header mark is the application icon itself (resource 1), so both stay in step.
        let icon = LoadImageW(
            GetModuleHandleW(ptr::null()),
            ptr::without_provenance(1),
            IMAGE_ICON,
            px(36),
            px(36),
            LR_DEFAULTCOLOR | LR_SHARED,
        );
        if !icon.is_null() {
            DrawIconEx(dc, 28, 28, icon as HICON, 36, 36, 0, ptr::null_mut(), DI_NORMAL);
        } else {
            rounded(dc, rect(28, 28, 36, 36), c.tint, c.tint, 12);
        }
        line_text(
            dc,
            "Region Watcher",
            rect(76, 23, 280, 30),
            22,
            true,
            c.text,
        );
        line_text(
            dc,
            "ESCAPE FROM TARKOV  /  非公式",
            rect(77, 55, 282, 18),
            10,
            false,
            c.muted,
        );
        if v.settings {
            line_text(dc, "設定", rect(28, 92, 260, 36), 28, true, c.text);
            line_text(
                dc,
                "使い方に合わせて、必要なものだけ。",
                rect(28, 132, 400, 24),
                13,
                false,
                c.muted,
            );
            rounded(dc, rect(28, 158, 424, 80), c.card, c.border, 16);
            line_text(dc, "ログフォルダ", rect(44, 170, 290, 22), 13, true, c.text);
            line_text(dc, &v.path, rect(44, 199, 290, 20), 11, false, c.muted);
            rounded(dc, rect(28, 252, 424, 224), c.card, c.border, 16);
        } else if v.log_tab {
            history(dc, v, c);
        } else {
            rounded(
                dc,
                rect(28, 150, if v.connected { 118 } else { 106 }, 28),
                if v.connected { c.tint } else { c.inset },
                if v.connected { c.tint } else { c.inset },
                28,
            );
            dot(dc, 40, 160, if v.connected { c.accent } else { c.muted });
            line_text(
                dc,
                if v.connected {
                    "接続を検出"
                } else {
                    "待機中"
                },
                rect(57, 151, 84, 26),
                12,
                true,
                if v.connected { c.accent } else { c.muted },
            );
            line_text(
                dc,
                if v.has_endpoint && !v.connected {
                    "最後に検出したサーバー"
                } else {
                    "ログから接続先を確認"
                },
                rect(160, 151, 270, 26),
                12,
                false,
                c.muted,
            );
            // Without the guidance line the card closes up around what is left.
            let gap = if v.provider.is_empty() { 36 } else { 0 };
            // Stacked so a long "国 / 地域 / 都市" label does not overflow the card.
            let parts: Vec<&str> = v.region.split(" / ").collect();
            // More lines need a taller band than the single-line layout leaves.
            let extra = match parts.len() { 1 => 0, 2 => 12, _ => 28 };
            rounded(dc, rect(28, 194, 424, 236 - gap + extra), c.card, c.border, 18);
            line_text(dc, "推定地域", rect(48, 213, 345, 22), 12, true, c.muted);
            let size = match (parts.len(), v.has_endpoint) {
                (1, true) => 28,
                (1, false) => 24,
                (2, _) => 24,
                _ => 20,
            };
            // +8, not +4: at +4 the box is shorter than the line and clips descenders (Angeles).
            let lh = size + 8;
            let top = 271 + extra / 2 - parts.len() as i32 * lh / 2;
            for (i, part) in parts.iter().enumerate() {
                line_text(
                    dc,
                    part,
                    rect(48, top + i as i32 * lh, 384, lh),
                    size,
                    true,
                    c.text,
                );
            }
            line_text(dc, &v.provider, rect(48, 301 + extra, 384, 24), 12, false, c.muted);
            fill(dc, rect(48, 346 - gap + extra, 384, 1), c.border);
            line_text(dc, "SERVER", rect(48, 362 - gap + extra, 80, 20), 10, true, c.muted);
            line_text(dc, &v.endpoint, rect(130, 359 - gap + extra, 302, 26), 17, true, c.text);
            line_text(dc, &v.time, rect(48, 395 - gap + extra, 384, 18), 11, false, c.muted);
        }
        if !v.settings {
            let notice = if !v.notice.is_empty() {
                &v.notice
            } else if !v.watching {
                "ログフォルダを選択してください。設定から変更できます。"
            } else {
                "地域は IP による推定です。接続継続を保証するものではありません。"
            };
            text(
                dc,
                notice,
                rect(28, 496, 424, 32),
                11,
                false,
                c.muted,
                DT_WORDBREAK | DT_END_ELLIPSIS,
            );
        } else {
            line_text(
                dc,
                concat!("EFTRegionWatcher  /  v", env!("CARGO_PKG_VERSION")),
                rect(28, 553, 240, 18),
                10,
                false,
                c.muted,
            );
        }
    }
}
unsafe fn scaled_canvas(dc: HDC, v: &View) {
    unsafe {
        let saved = SaveDC(dc);
        SetMapMode(dc, MM_ANISOTROPIC);
        SetWindowExtEx(dc, 96, 96, ptr::null_mut());
        let dpi = dpi();
        SetViewportExtEx(dc, dpi, dpi, ptr::null_mut());
        canvas(dc, v);
        RestoreDC(dc, saved);
    }
}
/// Drawn off-screen and blitted in one go: repaints must not flicker.
pub unsafe fn paint(hwnd: HWND) {
    unsafe {
        let mut ps: PAINTSTRUCT = mem::zeroed();
        let dc = BeginPaint(hwnd, &mut ps);
        let mut client: RECT = mem::zeroed();
        GetClientRect(hwnd, &mut client);
        let view = snapshot();
        let buffer = CreateCompatibleDC(dc);
        let bitmap = if buffer.is_null() {
            ptr::null_mut()
        } else {
            CreateCompatibleBitmap(dc, client.right, client.bottom)
        };
        if bitmap.is_null() {
            if !buffer.is_null() {
                DeleteDC(buffer);
            }
            scaled_canvas(dc, &view);
        } else {
            let old = SelectObject(buffer, bitmap);
            scaled_canvas(buffer, &view);
            BitBlt(
                dc,
                0,
                0,
                client.right,
                client.bottom,
                buffer,
                0,
                0,
                SRCCOPY,
            );
            SelectObject(buffer, old);
            DeleteObject(bitmap);
            DeleteDC(buffer);
        }
        EndPaint(hwnd, &ps);
    }
}
pub unsafe fn button(item: &DRAWITEMSTRUCT) {
    unsafe {
        let v = snapshot();
        let r = item.rcItem;
        let (pw, ph) = (r.right - r.left, r.bottom - r.top);
        let dpi = dpi();
        let buffer = CreateCompatibleDC(item.hDC);
        let bitmap = if buffer.is_null() {
            ptr::null_mut()
        } else {
            CreateCompatibleBitmap(item.hDC, pw, ph)
        };
        let buffered = !bitmap.is_null();
        let dc = if buffered { buffer } else { item.hDC };
        let old = if buffered {
            SelectObject(dc, bitmap)
        } else {
            ptr::null_mut()
        };
        let saved = SaveDC(dc);
        if !buffered {
            SetViewportOrgEx(dc, r.left, r.top, ptr::null_mut());
        }
        SetMapMode(dc, MM_ANISOTROPIC);
        SetWindowExtEx(dc, 96, 96, ptr::null_mut());
        SetViewportExtEx(dc, dpi, dpi, ptr::null_mut());
        draw_button(
            dc,
            item.CtlID as usize,
            pw * 96 / dpi,
            ph * 96 / dpi,
            item.itemState,
            &v,
        );
        RestoreDC(dc, saved);
        if buffered {
            BitBlt(item.hDC, r.left, r.top, pw, ph, buffer, 0, 0, SRCCOPY);
            SelectObject(buffer, old);
            DeleteObject(bitmap);
        }
        if !buffer.is_null() {
            DeleteDC(buffer);
        }
    }
}
unsafe fn draw_button(dc: HDC, id: usize, w: i32, h: i32, state: u32, v: &View) {
    unsafe {
        let c = palette(v.dark);
        // Whatever the button sits on shows through its rounded corners.
        fill(dc, rect(0, 0, w, h), if id == SELECT { c.card } else { c.bg });
        let pressed = state & ODS_SELECTED != 0;
        let disabled = state & ODS_DISABLED != 0;
        if [DARK, RESIDENT, NOTIFY, AUTO_UPDATE].contains(&id) {
            let (title, description, on) = match id {
                DARK => ("ダークモード", "落ち着いた暗い配色に切り替える", v.dark),
                RESIDENT => (
                    "閉じてもトレイに残す",
                    "オフのときは × でアプリを終了します",
                    v.resident,
                ),
                NOTIFY => (
                    "接続通知",
                    "新しい接続先を検出したときに通知",
                    v.notifications,
                ),
                _ => (
                    "自動更新チェック",
                    "起動時に新しいバージョンを確認",
                    v.auto_update,
                ),
            };
            // Rows sit inside one shared card drawn by canvas(); hairlines separate them.
            fill(dc, rect(0, 0, w, h), c.card);
            if pressed {
                rounded(dc, rect(0, 2, w, h - 4), c.inset, c.inset, 10);
            }
            if id != AUTO_UPDATE {
                fill(dc, rect(8, h - 1, w - 16, 1), c.border);
            }
            line_text(dc, title, rect(16, 8, w - 100, 22), 14, true, c.text);
            line_text(
                dc,
                description,
                rect(16, 32, w - 100, 19),
                11,
                false,
                c.muted,
            );
            toggle(dc, w - 60, (h - 24) / 2, on, c);
        } else if id == TAB_NOW || id == TAB_LOG {
            let active = (id == TAB_LOG) == v.log_tab;
            let bg = if active {
                c.tint
            } else if pressed {
                c.inset
            } else {
                c.card
            };
            rounded(dc, rect(0, 0, w, h), bg, if active { bg } else { c.border }, 10);
            text(
                dc,
                if id == TAB_NOW {
                    "現在の接続"
                } else {
                    "接続ログ"
                },
                rect(0, 0, w, h),
                12,
                active,
                if active { c.accent } else { c.muted },
                DT_SINGLELINE | DT_VCENTER | DT_CENTER,
            );
        } else {
            let primary = id == CHECK;
            let bg = if disabled {
                c.inset
            } else if pressed {
                c.border
            } else if primary {
                c.accent
            } else {
                c.card
            };
            rounded(
                dc,
                rect(0, 0, w, h),
                bg,
                if primary { bg } else { c.border },
                12,
            );
            let title = match id {
                COPY => "IP をコピー",
                SETTINGS => "設定",
                CLOSE => "閉じる",
                CHECK => "更新を確認",
                EXIT => "終了",
                SELECT => "変更",
                BACK => "戻る",
                _ => "",
            };
            text(
                dc,
                title,
                rect(0, 0, w, h),
                13,
                true,
                if disabled {
                    c.muted
                } else if primary {
                    c.accent_text
                } else {
                    c.text
                },
                DT_SINGLELINE | DT_VCENTER | DT_CENTER,
            );
        }
        if state & ODS_FOCUS != 0 && v.focus {
            let radius = if id == TAB_NOW || id == TAB_LOG { 10 } else { 12 };
            focus_ring(dc, w, h, radius, c);
        }
    }
}
pub fn button_specs(settings: bool) -> Vec<(usize, &'static str, i32, i32, i32, i32)> {
    if settings {
        vec![
            (BACK, "メイン画面に戻る", 384, 28, 68, 36),
            (SELECT, "ログフォルダを変更", 348, 178, 88, 38),
            (DARK, "ダークモードを切り替える", 36, 260, 408, 52),
            (RESIDENT, "閉じてもトレイに残すかを切り替える", 36, 312, 408, 52),
            (NOTIFY, "接続通知を切り替える", 36, 364, 408, 52),
            (AUTO_UPDATE, "自動更新チェックを切り替える", 36, 416, 408, 52),
            (CHECK, "更新を確認", 28, 492, 424, 40),
            (EXIT, "アプリを終了", 380, 548, 72, 26),
        ]
    } else {
        vec![
            (SETTINGS, "設定", 384, 28, 68, 36),
            (TAB_NOW, "現在の接続", 28, 96, 100, 34),
            (TAB_LOG, "接続ログ", 132, 96, 100, 34),
            (EXIT, "アプリを終了", 380, 548, 72, 26),
        ]
    }
}

struct ModalStyle {
    title: String,
    dark: bool,
    default_id: i32,
    brush: HBRUSH,
}
thread_local! {static MODAL:RefCell<Option<ModalStyle>>=const {RefCell::new(None)};}
const MODAL_RESULT: u32 = WM_APP + 5;
unsafe extern "system" fn modal_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_ERASEBKGND => 1,
            WM_CLOSE => {
                PostMessageW(hwnd, MODAL_RESULT, 0, 0);
                0
            }
            WM_COMMAND => {
                if (w >> 16) as u32 == BN_CLICKED {
                    PostMessageW(hwnd, MODAL_RESULT, w & 0xffff, 0);
                }
                0
            }
            WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT => MODAL.with(|state| {
                let s = state.borrow();
                if let Some(s) = s.as_ref() {
                    let c = palette(s.dark);
                    SetTextColor(w as HDC, c.text);
                    SetBkColor(w as HDC, c.card);
                    s.brush as isize
                } else {
                    0
                }
            }),
            WM_PAINT => {
                let mut ps: PAINTSTRUCT = mem::zeroed();
                let dc = BeginPaint(hwnd, &mut ps);
                MODAL.with(|state| {
                    if let Some(s) = state.borrow().as_ref() {
                        let c = palette(s.dark);
                        let mut client: RECT = mem::zeroed();
                        GetClientRect(hwnd, &mut client);
                        fill(dc, client, c.bg);
                        let saved = SaveDC(dc);
                        SetMapMode(dc, MM_ANISOTROPIC);
                        SetWindowExtEx(dc, 96, 96, ptr::null_mut());
                        SetViewportExtEx(dc, dpi(), dpi(), ptr::null_mut());
                        text(
                            dc,
                            &s.title,
                            rect(24, 24, 392, 60),
                            21,
                            true,
                            c.text,
                            DT_WORDBREAK,
                        );
                        rounded(dc, rect(20, 94, 400, 186), c.card, c.border, 12);
                        RestoreDC(dc, saved);
                    }
                });
                EndPaint(hwnd, &ps);
                0
            }
            WM_DRAWITEM => {
                if l != 0 {
                    let item = &*(l as *const DRAWITEMSTRUCT);
                    MODAL.with(|state| {
                        if let Some(s) = state.borrow().as_ref() {
                            let c = palette(s.dark);
                            let mut buffer = [0u16; 160];
                            let n = GetWindowTextW(
                                item.hwndItem,
                                buffer.as_mut_ptr(),
                                buffer.len() as i32,
                            );
                            let label = String::from_utf16_lossy(&buffer[..n.max(0) as usize]);
                            let dc = item.hDC;
                            let saved = SaveDC(dc);
                            SetMapMode(dc, MM_ANISOTROPIC);
                            SetWindowExtEx(dc, 96, 96, ptr::null_mut());
                            SetViewportExtEx(dc, dpi(), dpi(), ptr::null_mut());
                            let w = (item.rcItem.right - item.rcItem.left) * 96 / dpi();
                            let h = (item.rcItem.bottom - item.rcItem.top) * 96 / dpi();
                            fill(dc, rect(0, 0, w, h), c.bg);
                            let primary = item.CtlID as i32 == s.default_id;
                            let color = if item.itemState & ODS_SELECTED != 0 {
                                c.border
                            } else if primary {
                                c.accent
                            } else {
                                c.card
                            };
                            rounded(
                                dc,
                                rect(0, 0, w, h),
                                color,
                                if primary { color } else { c.border },
                                12,
                            );
                            text(
                                dc,
                                &label,
                                rect(8, 0, w - 16, h),
                                13,
                                true,
                                if primary { c.accent_text } else { c.text },
                                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
                            );
                            if item.itemState & ODS_FOCUS != 0 {
                                focus_ring(dc, w, h, 12, c);
                            }
                            RestoreDC(dc, saved);
                        }
                    });
                }
                1
            }
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}
/// Plain text, themed native modal; no HTML rendering and no web runtime.
pub fn modal(
    parent: HWND,
    title: &str,
    content: &str,
    buttons: &[(i32, &str)],
    default: i32,
) -> i32 {
    unsafe {
        let dark = snapshot().dark;
        let c = palette(dark);
        let class = wide("EFTRegionWatcher.Modal");
        let instance = GetModuleHandleW(ptr::null());
        let wc = WNDCLASSW {
            lpfnWndProc: Some(modal_proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
            ..mem::zeroed()
        };
        RegisterClassW(&wc);
        let height = 310 + buttons.len() as i32 * 48;
        let mut bounds = rect(0, 0, px(440), px(height));
        AdjustWindowRectEx(&mut bounds, WS_CAPTION | WS_SYSMENU, 0, WS_EX_DLGMODALFRAME);
        let mut owner: RECT = mem::zeroed();
        GetWindowRect(parent, &mut owner);
        let x = (owner.left + owner.right - (bounds.right - bounds.left)) / 2;
        let y = owner.top + 40;
        let brush = CreateSolidBrush(c.card);
        MODAL.with(|s| {
            *s.borrow_mut() = Some(ModalStyle {
                title: title.into(),
                dark,
                default_id: default,
                brush,
            })
        });
        let hwnd = CreateWindowExW(
            WS_EX_DLGMODALFRAME,
            class.as_ptr(),
            wide("EFTRegionWatcher").as_ptr(),
            WS_CAPTION | WS_SYSMENU,
            x,
            y,
            bounds.right - bounds.left,
            bounds.bottom - bounds.top,
            parent,
            ptr::null_mut(),
            instance,
            ptr::null(),
        );
        if hwnd.is_null() {
            MODAL.with(|s| s.borrow_mut().take());
            DeleteObject(brush);
            return 0;
        }
        apply_theme(hwnd, dark);
        let edit = control(
            hwnd,
            "EDIT",
            content,
            ES_MULTILINE as u32 | ES_READONLY as u32 | ES_AUTOVSCROLL as u32 | WS_VSCROLL,
            px(32),
            px(106),
            px(376),
            px(160),
            900,
        );
        let font = CreateFontW(
            -px(13),
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            0,
            0,
            CLEARTYPE_QUALITY as u32,
            0,
            wide(ui_font()).as_ptr(),
        );
        SendMessageW(edit, WM_SETFONT, font as usize, 1);
        let mut default_button = ptr::null_mut();
        for (index, (id, label)) in buttons.iter().enumerate() {
            let b = control(
                hwnd,
                "BUTTON",
                label,
                BS_OWNERDRAW as u32,
                px(24),
                px(298 + index as i32 * 48),
                px(392),
                px(40),
                *id as usize,
            );
            if *id == default {
                default_button = b;
            }
        }
        EnableWindow(parent, 0);
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        if !default_button.is_null() {
            SetFocus(default_button);
        }
        let mut result = 0;
        let mut msg: MSG = mem::zeroed();
        loop {
            let status = GetMessageW(&mut msg, ptr::null_mut(), 0, 0);
            if status <= 0 {
                if status == 0 {
                    PostQuitMessage(msg.wParam as i32);
                }
                break;
            }
            if msg.message == WM_KEYDOWN && msg.wParam == VK_ESCAPE as usize {
                break;
            }
            if msg.message == WM_KEYDOWN && msg.wParam == VK_RETURN as usize {
                let id = GetDlgCtrlID(GetFocus());
                result = if buttons.iter().any(|(button, _)| *button == id) {
                    id
                } else {
                    default
                };
                break;
            }
            if msg.hwnd == hwnd && msg.message == MODAL_RESULT {
                let selected = msg.wParam as i32;
                if selected == 0 || buttons.iter().any(|(id, _)| *id == selected) {
                    result = selected;
                    break;
                }
            }
            if IsDialogMessageW(hwnd, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        EnableWindow(parent, 1);
        DestroyWindow(hwnd);
        DeleteObject(font);
        MODAL.with(|s| s.borrow_mut().take());
        DeleteObject(brush);
        SetForegroundWindow(parent);
        result
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundled_font_is_usable() {
        assert_eq!(ui_font(), "Noto Sans JP");
        unsafe {
            let mut fonts = 0u32;
            const NOTO: &[u8] = include_bytes!("../../assets/fonts/NotoSansJP-VF.ttf");
            let loaded = AddFontMemResourceEx(
                NOTO.as_ptr().cast(),
                NOTO.len() as u32,
                ptr::null(),
                (&raw mut fonts) as *const u32,
            );
            assert!(!loaded.is_null() && fonts > 0, "embedded font rejected by GDI");
        }
    }
    #[test]
    fn render_theme_previews() {
        unsafe {
            let output =
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/ui-previews");
            std::fs::create_dir_all(&output).unwrap();
            for (name, dark, settings, connected, log_tab, scale) in [
                ("dark-connected", true, false, true, false, 1),
                ("dark-waiting", true, false, false, false, 1),
                ("light-connected", false, false, true, false, 1),
                ("dark-log", true, false, true, true, 1),
                ("light-log", false, false, true, true, 1),
                ("dark-settings", true, true, false, false, 1),
                ("light-settings", false, true, false, false, 1),
                ("dark-log-200", true, false, true, true, 2),
            ] {
                let width = WIDTH * scale;
                let height = HEIGHT * scale;
                let dc = CreateCompatibleDC(ptr::null_mut());
                assert!(!dc.is_null());
                let mut info: BITMAPINFO = mem::zeroed();
                info.bmiHeader.biSize = mem::size_of::<BITMAPINFOHEADER>() as u32;
                info.bmiHeader.biWidth = width;
                info.bmiHeader.biHeight = -height;
                info.bmiHeader.biPlanes = 1;
                info.bmiHeader.biBitCount = 32;
                let mut bits = ptr::null_mut();
                let bitmap =
                    CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, ptr::null_mut(), 0);
                assert!(!bitmap.is_null());
                let old = SelectObject(dc, bitmap);
                SetMapMode(dc, MM_ANISOTROPIC);
                SetWindowExtEx(dc, 1, 1, ptr::null_mut());
                SetViewportExtEx(dc, scale, scale, ptr::null_mut());
                let view = View {
                    dark,
                    settings,
                    connected,
                    has_endpoint: connected,
                    watching: true,
                    region: if connected {
                        "Japan / Tokyo".into()
                    } else {
                        "接続先を待っています".into()
                    },
                    provider: if connected {
                        String::new()
                    } else {
                        "EFT の接続ログを検出すると、推定地域を表示します。".into()
                    },
                    endpoint: if connected {
                        "203.0.113.10".into()
                    } else {
                        "まだ検出されていません".into()
                    },
                    time: if connected {
                        "検出  2026/09/10 14:30:00".into()
                    } else {
                        String::new()
                    },
                    log_tab,
                    history: if connected {
                        (0..5)
                            .map(|i| {
                                (
                                    format!("203.0.113.{}", 10 + i),
                                    ["Japan / Tokyo", "Germany / Frankfurt", "地域を取得しています…"]
                                        [i % 3]
                                        .into(),
                                    format!("2026/09/10 1{}:30:00", 4 - i),
                                )
                            })
                            .collect()
                    } else {
                        Vec::new()
                    },
                    history_total: if connected { 12 } else { 0 },
                    path: r"D:\Battlestate Games\EFT\Logs".into(),
                    ..View::default()
                };
                canvas(dc, &view);
                for (id, _, x, y, w, h) in button_specs(view.settings) {
                    let saved = SaveDC(dc);
                    SetViewportOrgEx(dc, x * scale, y * scale, ptr::null_mut());
                    draw_button(
                        dc,
                        id,
                        w,
                        h,
                        if id == COPY && !view.has_endpoint {
                            ODS_DISABLED
                        } else {
                            0
                        },
                        &view,
                    );
                    RestoreDC(dc, saved);
                }
                GdiFlush();
                let size = (width * height * 4) as usize;
                let bytes = std::slice::from_raw_parts(bits.cast::<u8>(), size);
                let mut bmp = Vec::new();
                bmp.extend_from_slice(b"BM");
                bmp.extend_from_slice(&(54 + size as u32).to_le_bytes());
                bmp.extend_from_slice(&[0; 4]);
                bmp.extend_from_slice(&54u32.to_le_bytes());
                bmp.extend_from_slice(&40u32.to_le_bytes());
                bmp.extend_from_slice(&width.to_le_bytes());
                bmp.extend_from_slice(&(-height).to_le_bytes());
                bmp.extend_from_slice(&1u16.to_le_bytes());
                bmp.extend_from_slice(&32u16.to_le_bytes());
                bmp.extend_from_slice(&[0; 24]);
                bmp.extend_from_slice(bytes);
                std::fs::write(output.join(format!("{name}.bmp")), bmp).unwrap();
                SelectObject(dc, old);
                DeleteObject(bitmap);
                DeleteDC(dc);
            }
        }
    }
}
