//! Windows 전체 화면 가림 창 + 저수준 키보드 훅
use std::cell::{Cell, RefCell};

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateCompatibleBitmap,
    CreateCompatibleDC, CreateFontW, CreateSolidBrush, DEFAULT_CHARSET, DT_CENTER, DT_SINGLELINE,
    DT_VCENTER, DT_WORDBREAK, DeleteDC, DeleteObject, DrawTextW, EndPaint, EnumDisplayMonitors,
    FF_DONTCARE, FW_BOLD, FW_NORMAL, FillRect, GetMonitorInfoW, HDC, HFONT, HGDIOBJ, HMONITOR,
    InvalidateRect, MONITORINFO, OUT_DEFAULT_PRECIS, PAINTSTRUCT, SRCCOPY, SelectObject,
    SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SetFocus, VK_BACK, VK_OEM_COMMA, VK_OEM_MINUS, VK_OEM_PERIOD, VK_SPACE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, CallNextHookEx, CreateWindowExW, GetForegroundWindow, GetWindowThreadProcessId, DefWindowProcW, DestroyWindow, GetClientRect, HC_ACTION,
    HHOOK, HWND_TOPMOST, IDC_ARROW, KBDLLHOOKSTRUCT, LoadCursorW, RegisterClassW,
    SWP_NOACTIVATE, SWP_SHOWWINDOW, SetForegroundWindow, SetWindowPos, SetWindowsHookExW,
    ShowWindow, SW_SHOWNA, UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_CLOSE, WM_ERASEBKGND,
    WM_KEYDOWN, WM_PAINT, WM_SYSKEYDOWN, WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
    LWA_ALPHA, SetLayeredWindowAttributes, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TRANSPARENT,
};
use windows::core::{BOOL, PCWSTR, w};

use crate::common::phrase::PhraseBuffer;
use crate::common::tips::PostureTip;

const CLASS_NAME: PCWSTR = w!("ForceStretchOverlay");
const NUDGE_CLASS: PCWSTR = w!("ForceStretchNudge");

//#region 화면에 그릴 내용 (WM_PAINT 에서 읽음, 앱 상태와 분리해 재진입 방지)
#[derive(Default, Clone)]
pub struct PaintText {
    pub countdown: String,
    pub tip: String,
    pub footer: String,
}

thread_local! {
    static PAINT: RefCell<PaintText> = RefCell::new(PaintText::default());
    static TYPED: RefCell<PhraseBuffer> = const { RefCell::new(PhraseBuffer::new()) };
    static HOOK: Cell<Option<HHOOK>> = const { Cell::new(None) };
    static NUDGE: RefCell<Option<&'static PostureTip>> = const { RefCell::new(None) };
}

pub fn set_paint_text(t: PaintText) {
    PAINT.with(|p| *p.borrow_mut() = t);
}
//#endregion

//#region 비상 문구 버퍼 접근
pub fn typed_matches(phrase: &str) -> bool {
    TYPED.with(|t| t.borrow().matches(phrase))
}

pub fn clear_typed() {
    TYPED.with(|t| t.borrow_mut().clear());
}

pub fn typed_len() -> usize {
    TYPED.with(|t| t.borrow().len())
}
//#endregion

//#region 저수준 키보드 훅: 스트레칭 중 모든 키 차단 + 비상 문구 수집
/// 가상 키코드 → 영문 문자 (한글 IME 상태와 무관하게 물리 키 기준)
fn vk_to_char(vk: u32) -> Option<char> {
    match vk {
        0x41..=0x5A => char::from_u32(vk + 0x20), // A-Z → a-z
        0x30..=0x39 => char::from_u32(vk),        // 0-9
        v if v == VK_SPACE.0 as u32 => Some(' '),
        v if v == VK_OEM_MINUS.0 as u32 => Some('-'),
        v if v == VK_OEM_COMMA.0 as u32 => Some(','),
        v if v == VK_OEM_PERIOD.0 as u32 => Some('.'),
        _ => None,
    }
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let msg = wparam.0 as u32;
        if msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN {
            // SAFETY: WH_KEYBOARD_LL 의 lParam 은 KBDLLHOOKSTRUCT 포인터
            let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
            TYPED.with(|t| {
                if let Ok(mut t) = t.try_borrow_mut() {
                    if info.vkCode == VK_BACK.0 as u32 {
                        t.backspace();
                    } else if let Some(c) = vk_to_char(info.vkCode) {
                        t.push(c);
                    }
                }
            });
            // 키 누름은 모두 삼킴: Win 키, Alt+Tab, Alt+F4 등 (Ctrl+Alt+Del 은 OS 가 처리하므로 차단 불가)
            return LRESULT(1);
        }
        // 키 뗌은 통과: 가림 시작 시 누르고 있던 Shift 등이 "눌린 채" 남지 않도록
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

pub fn install_hook() -> bool {
    if HOOK.with(|h| h.get()).is_some() {
        return true;
    }
    let hmod = unsafe { GetModuleHandleW(None) }.ok().map(|m| m.into());
    match unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), hmod, 0) } {
        Ok(h) => {
            HOOK.with(|c| c.set(Some(h)));
            true
        }
        Err(e) => {
            crate::common::log::write(&format!("키보드 훅 설치 실패: {e}"));
            false
        }
    }
}

pub fn remove_hook() {
    if let Some(h) = HOOK.with(|c| c.take()) {
        let _ = unsafe { UnhookWindowsHookEx(h) };
    }
}
//#endregion

//#region 가림 창 클래스 등록 + 그리기
pub fn register_class() {
    let hinstance = unsafe { GetModuleHandleW(None) }.unwrap_or_default();
    let wc = WNDCLASSW {
        lpfnWndProc: Some(overlay_proc),
        hInstance: hinstance.into(),
        lpszClassName: CLASS_NAME,
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default(),
        ..Default::default()
    };
    unsafe { RegisterClassW(&wc) };
    let nudge = WNDCLASSW { lpfnWndProc: Some(nudge_proc), lpszClassName: NUDGE_CLASS, ..wc };
    unsafe { RegisterClassW(&nudge) };
}

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
}

fn font(height: i32, bold: bool) -> HFONT {
    font_face(height, bold, w!("Malgun Gothic"))
}

fn font_face(height: i32, bold: bool, face: PCWSTR) -> HFONT {
    unsafe {
        CreateFontW(
            -height,
            0,
            0,
            0,
            if bold { FW_BOLD.0 as i32 } else { FW_NORMAL.0 as i32 },
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            FF_DONTCARE.0 as u32,
            face,
        )
    }
}

fn draw_text(hdc: HDC, text: &str, rect: RECT, height: i32, bold: bool, color: COLORREF, wrap: bool) {
    draw_text_font(hdc, text, rect, font(height, bold), color, wrap);
}

fn draw_text_font(hdc: HDC, text: &str, rect: RECT, f: HFONT, color: COLORREF, wrap: bool) {
    unsafe {
        let old = SelectObject(hdc, HGDIOBJ(f.0));
        SetTextColor(hdc, color);
        let mut buf: Vec<u16> = text.encode_utf16().collect();
        let mut r = rect;
        let flags = if wrap { DT_CENTER | DT_WORDBREAK } else { DT_CENTER | DT_VCENTER | DT_SINGLELINE };
        if !buf.is_empty() {
            DrawTextW(hdc, &mut buf, &mut r, flags);
        }
        SelectObject(hdc, old);
        let _ = DeleteObject(HGDIOBJ(f.0));
    }
}

/// 메모리 DC 에 그린 뒤 한 번에 복사 (깜빡임 방지). `draw(mem, w, h)` 가 배경 위에 내용을 그림.
fn paint_buffered(hwnd: HWND, draw: impl FnOnce(HDC, i32, i32)) {
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);
        let mut rc = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);
        let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
        let mem = CreateCompatibleDC(Some(hdc));
        let bmp = CreateCompatibleBitmap(hdc, w, h);
        let old_bmp = SelectObject(mem, HGDIOBJ(bmp.0));

        let bg = CreateSolidBrush(rgb(20, 23, 33));
        FillRect(mem, &rc, bg);
        let _ = DeleteObject(HGDIOBJ(bg.0));
        SetBkMode(mem, TRANSPARENT);
        draw(mem, w, h);

        let _ = BitBlt(hdc, 0, 0, w, h, Some(mem), 0, 0, SRCCOPY);
        SelectObject(mem, old_bmp);
        let _ = DeleteObject(HGDIOBJ(bmp.0));
        let _ = DeleteDC(mem);
        let _ = EndPaint(hwnd, &ps);
    }
}

fn paint(hwnd: HWND) {
    let text = PAINT.with(|p| p.borrow().clone());
    paint_buffered(hwnd, |mem, w, h| {
        let pad = 40;
        let band = |top: f64, height: f64| RECT {
            left: pad,
            right: w - pad,
            top: (h as f64 * top) as i32,
            bottom: (h as f64 * (top + height)) as i32,
        };
        let hf = h as f64;
        draw_text(mem, "일어나서 스트레칭할 시간입니다", band(0.22, 0.10),
            (hf * 0.05).clamp(28.0, 64.0) as i32, true, rgb(255, 107, 89), false);
        draw_text(mem, &text.countdown, band(0.34, 0.28),
            (hf * 0.2).clamp(80.0, 260.0) as i32, true, rgb(255, 255, 255), false);
        draw_text(mem, &text.tip, band(0.66, 0.18),
            (hf * 0.032).clamp(20.0, 40.0) as i32, false, rgb(255, 255, 255), true);
        draw_text(mem, &text.footer, RECT { left: pad, right: w - pad, top: h - 64, bottom: h - 24 },
            14, false, rgb(140, 140, 150), false);
    });
}

unsafe extern "system" fn overlay_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        // 배경 지우기 생략 (WM_PAINT 에서 전부 칠함 → 깜빡임 방지)
        WM_ERASEBKGND => LRESULT(1),
        // Alt+F4 등으로 닫히지 않게 무시
        WM_CLOSE => LRESULT(0),
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
//#endregion

//#region 자세 귀띔 창 (2초 표시, 초점·키 입력·마우스를 빼앗지 않음)
fn paint_nudge(hwnd: HWND) {
    let Some(tip) = NUDGE.with(|n| *n.borrow()) else { return };
    paint_buffered(hwnd, |mem, w, h| {
        let pad = 40;
        let hf = h as f64;
        let band = |top: f64, height: f64| RECT {
            left: pad,
            right: w - pad,
            top: (hf * top) as i32,
            bottom: (hf * (top + height)) as i32,
        };
        // GDI 는 컬러 이모지를 못 그리므로 Segoe UI Emoji 단색 글리프로 표시
        let icon_font = font_face((hf * 0.14).clamp(64.0, 180.0) as i32, false, w!("Segoe UI Emoji"));
        draw_text_font(mem, tip.icon, band(0.22, 0.24), icon_font, rgb(255, 255, 255), false);
        draw_text(mem, tip.title, band(0.48, 0.12),
            (hf * 0.075).clamp(40.0, 96.0) as i32, true, rgb(255, 107, 89), false);
        draw_text(mem, tip.detail, band(0.61, 0.08),
            (hf * 0.038).clamp(22.0, 48.0) as i32, false, rgb(255, 255, 255), false);
    });
}

unsafe extern "system" fn nudge_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint_nudge(hwnd);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// 모든 모니터에 반투명 귀띔 창 표시. 클릭은 아래 창으로 통과하고, 활성화하지 않음.
pub fn show_nudges(tip: &'static PostureTip) -> Vec<HWND> {
    NUDGE.with(|n| *n.borrow_mut() = Some(tip));
    let hinstance = unsafe { GetModuleHandleW(None) }.ok();
    monitor_rects()
        .into_iter()
        .filter_map(|r| unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE,
                NUDGE_CLASS,
                w!("Force Stretch"),
                WS_POPUP,
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                None,
                None,
                hinstance.map(|h| h.into()),
                None,
            )
            .ok()
        })
        .inspect(|&hwnd| unsafe {
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 240, LWA_ALPHA);
            let _ = ShowWindow(hwnd, SW_SHOWNA);
        })
        .collect()
}
//#endregion

//#region 모니터마다 가림 창 생성/유지/제거
unsafe extern "system" fn collect_monitor(hmon: HMONITOR, _hdc: HDC, _rc: *mut RECT, data: LPARAM) -> BOOL {
    // SAFETY: data 는 build_all 의 Vec<RECT> 포인터
    let rects = unsafe { &mut *(data.0 as *mut Vec<RECT>) };
    let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    if unsafe { GetMonitorInfoW(hmon, &mut mi) }.as_bool() {
        rects.push(mi.rcMonitor);
    }
    BOOL(1)
}

pub fn monitor_rects() -> Vec<RECT> {
    let mut rects: Vec<RECT> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(collect_monitor), LPARAM(&mut rects as *mut _ as isize));
    }
    rects
}

pub fn build_all() -> Vec<HWND> {
    let hinstance = unsafe { GetModuleHandleW(None) }.ok();
    monitor_rects()
        .into_iter()
        .filter_map(|r| unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                CLASS_NAME,
                w!("Force Stretch"),
                WS_POPUP,
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                None,
                None,
                hinstance.map(|h| h.into()),
                None,
            )
            .ok()
        })
        .inspect(|&hwnd| unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNA);
        })
        .collect()
}

/// 매 틱 최상위 재지정 + 첫 창을 전면으로 (키 입력은 훅이 막지만, 초점까지 가져와야 확실함)
pub fn keep_on_top(windows: &[HWND]) {
    for &hwnd in windows {
        unsafe {
            let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0,
                windows::Win32::UI::WindowsAndMessaging::SWP_NOMOVE
                    | windows::Win32::UI::WindowsAndMessaging::SWP_NOSIZE
                    | SWP_NOACTIVATE
                    | SWP_SHOWWINDOW);
        }
    }
    if let Some(&first) = windows.first() {
        force_foreground(first, windows);
    }
}

//#region 전면 창 강제 전환
/// 윈도우는 백그라운드 프로세스의 SetForegroundWindow 를 막으므로(포그라운드 잠금),
/// 현재 전면 창의 입력 스레드에 잠시 붙어서 권한을 빌린 뒤 전환한다.
fn force_foreground(target: HWND, ours: &[HWND]) {
    unsafe {
        let fg = GetForegroundWindow();
        if ours.contains(&fg) {
            return;
        }
        let me = GetCurrentThreadId();
        let fg_thread = if fg.is_invalid() { 0 } else { GetWindowThreadProcessId(fg, None) };
        let attached = fg_thread != 0 && fg_thread != me && AttachThreadInput(me, fg_thread, true).as_bool();
        let _ = BringWindowToTop(target);
        let _ = SetForegroundWindow(target);
        let _ = SetFocus(Some(target));
        if attached {
            let _ = AttachThreadInput(me, fg_thread, false);
        }
    }
}
//#endregion

pub fn invalidate(windows: &[HWND]) {
    for &hwnd in windows {
        unsafe {
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }
}

pub fn destroy_all(windows: &mut Vec<HWND>) {
    for hwnd in windows.drain(..) {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }
}
//#endregion
