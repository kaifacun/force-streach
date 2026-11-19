//! Windows 구현: 작업 표시줄 트레이 상주 + Win32 전체 화면 가림 창
mod overlay;
pub mod update;

use std::cell::RefCell;

use windows::Win32::Foundation::{
    COLORREF, ERROR_SUCCESS, HWND, LPARAM, LRESULT, POINT,
    RECT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    ANTIALIASED_QUALITY, BLACK_BRUSH, BLACKNESS, CLIP_DEFAULT_PRECIS, CreateBitmap,
    CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, CreateSolidBrush, DEFAULT_CHARSET,
    DT_CENTER, DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject, DrawTextW, Ellipse, FF_DONTCARE,
    FW_NORMAL, GetDC, GetStockObject, NULL_PEN, OUT_DEFAULT_PRECIS, PatBlt, ReleaseDC,
    SelectObject, SetBkMode, SetTextColor, TRANSPARENT, WHITENESS,
};
use windows::Win32::System::Diagnostics::Debug::MessageBeep;
use windows::Win32::System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleW};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
    ShellExecuteW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, GetSystemMetrics, HICON, ICONINFO,
    SM_CXSMICON, CreateWindowExW, DefWindowProcW, DestroyMenu,
    DispatchMessageW, GetCursorPos, GetForegroundWindow, GetMessageW, HMENU, IDI_APPLICATION,
    LoadIconW, MB_ICONASTERISK, MB_ICONEXCLAMATION, MF_CHECKED, MF_GRAYED, MF_POPUP,
    MF_SEPARATOR, MF_STRING, MSG, PostMessageW, PostQuitMessage,
    RegisterClassW, RegisterWindowMessageW, SW_SHOWNORMAL, SetForegroundWindow, SetTimer,
    TPM_BOTTOMALIGN, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WINDOW_EX_STYLE,
    WM_APP, WM_COMMAND, WM_DESTROY, WM_DISPLAYCHANGE, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP,
    WM_TIMER, WNDCLASSW, WS_OVERLAPPED,
};
use windows::core::{HSTRING, PCWSTR, w};

use crate::common::config::{self, Config};
use crate::common::schedule::{Event, Now, Scheduler, fmt_dur_label};
use crate::common::update::{self as upd, Updater};
use crate::common::{failsafe, log};

//#region 상수 (메시지·메뉴 ID)
const WM_TRAY: u32 = WM_APP + 1;
const TIMER_ID: usize = 1;
const TRAY_UID: u32 = 1;

const ID_START: usize = 1;
const ID_PAUSE: usize = 2;
const ID_OPEN_CONFIG: usize = 3;
const ID_OPEN_LOG: usize = 4;
const ID_AUTOSTART: usize = 5;
const ID_UPDATE: usize = 6;
const ID_QUIT: usize = 9;
/// 주기 메뉴: 10000 + 초
const ID_INTERVAL_BASE: usize = 10_000;
/// 스트레칭 시간 메뉴: 20000 + 초
const ID_BREAK_BASE: usize = 20_000;

const INTERVAL_OPTIONS_MIN: [u64; 6] = [20, 30, 45, 50, 60, 90];
const BREAK_OPTIONS_SECS: [u64; 5] = [60, 120, 180, 300, 600];

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const RUN_VALUE: PCWSTR = w!("ForceStretch");
//#endregion

//#region 앱 상태 (메인 스레드 전용)
struct App {
    hwnd: HWND,
    sched: Scheduler,
    test_mode: bool,
    overlays: Vec<HWND>,
    displays_dirty: bool,
    taskbar_created_msg: u32,
    updater: Updater,
    /// 사용자가 설치를 눌렀음 (실패하면 릴리스 페이지를 열어 수동 설치 안내)
    install_requested: bool,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// 앱 상태에 접근. 재진입(이미 빌려간 상태)이면 건너뜀.
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}
//#endregion

//#region OS 유틸: 자리 비움, 문자열 변환
/// 마지막 키보드·마우스 입력 이후 경과 초
fn idle_secs() -> f64 {
    let mut lii = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    if unsafe { GetLastInputInfo(&mut lii) }.as_bool() {
        // GetTickCount 는 약 49.7일마다 0으로 돌아가므로 wrapping 연산
        unsafe { GetTickCount() }.wrapping_sub(lii.dwTime) as f64 / 1000.0
    } else {
        0.0
    }
}

fn to_wide_fixed<const N: usize>(s: &str) -> [u16; N] {
    let mut buf = [0u16; N];
    for (i, c) in s.encode_utf16().take(N - 1).enumerate() {
        buf[i] = c;
    }
    buf
}
//#endregion

//#region 로그인 시 자동 실행 (HKCU\...\Run 레지스트리)
fn autostart_enabled() -> bool {
    let r = unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE, RRF_RT_REG_SZ, None, None, None) };
    r == ERROR_SUCCESS
}

fn set_autostart(enable: bool) {
    unsafe {
        if enable {
            let mut buf = [0u16; 1024];
            let len = GetModuleFileNameW(None, &mut buf) as usize;
            let exe = String::from_utf16_lossy(&buf[..len]);
            let value: Vec<u16> = format!("\"{exe}\"").encode_utf16().chain(Some(0)).collect();
            let r = RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                RUN_VALUE,
                REG_SZ.0,
                Some(value.as_ptr() as *const _),
                (value.len() * 2) as u32,
            );
            log::write(&format!("자동 실행 등록: {exe} → {:?}", r));
        } else {
            let r = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE);
            log::write(&format!("자동 실행 해제 → {:?}", r));
        }
    }
}
//#endregion

//#region 트레이 아이콘
thread_local! {
    static TRAY_ICON: std::cell::OnceCell<HICON> = const { std::cell::OnceCell::new() };
}

/// 전용 아이콘 (한 번 그려서 재사용). 실패하면 기본 앱 아이콘.
fn tray_icon() -> HICON {
    TRAY_ICON.with(|c| {
        *c.get_or_init(|| {
            draw_icon().unwrap_or_else(|| unsafe { LoadIconW(None, IDI_APPLICATION) }.unwrap_or_default())
        })
    })
}

/// 코럴색 원 + 흰색 🧘 글리프를 GDI 로 그려 아이콘 생성
fn draw_icon() -> Option<HICON> {
    unsafe {
        let size = GetSystemMetrics(SM_CXSMICON).max(16);
        let screen = GetDC(None);
        let color = CreateCompatibleBitmap(screen, size, size);
        let mask = CreateBitmap(size, size, 1, 1, None);
        let dc = CreateCompatibleDC(Some(screen));

        //#region 색상 비트맵: 바깥은 검정(마스크와 XOR 시 투명), 안은 원 + 글리프
        let old = SelectObject(dc, color.into());
        let _ = PatBlt(dc, 0, 0, size, size, BLACKNESS);
        let brush = CreateSolidBrush(COLORREF(0x005B6BFF)); // BGR: #FF6B5B
        let old_brush = SelectObject(dc, brush.into());
        let old_pen = SelectObject(dc, GetStockObject(NULL_PEN));
        let _ = Ellipse(dc, 0, 0, size + 1, size + 1);
        let font = CreateFontW(
            -(size * 3 / 4), 0, 0, 0, FW_NORMAL.0 as i32, 0, 0, 0, DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, ANTIALIASED_QUALITY, FF_DONTCARE.0 as u32,
            w!("Segoe UI Emoji"),
        );
        let old_font = SelectObject(dc, font.into());
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, COLORREF(0x00FFFFFF));
        let mut text: Vec<u16> = "🧘".encode_utf16().collect();
        let mut rect = RECT { left: 0, top: 0, right: size, bottom: size };
        DrawTextW(dc, &mut text, &mut rect, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        SelectObject(dc, old_font);
        SelectObject(dc, old_pen);
        SelectObject(dc, old_brush);
        let _ = DeleteObject(font.into());
        let _ = DeleteObject(brush.into());
        //#endregion

        //#region 마스크: 흰색(1)=투명, 원 안은 검정(0)=불투명
        SelectObject(dc, mask.into());
        let _ = PatBlt(dc, 0, 0, size, size, WHITENESS);
        let old_brush = SelectObject(dc, GetStockObject(BLACK_BRUSH));
        let old_pen = SelectObject(dc, GetStockObject(NULL_PEN));
        let _ = Ellipse(dc, 0, 0, size + 1, size + 1);
        SelectObject(dc, old_pen);
        SelectObject(dc, old_brush);
        //#endregion

        SelectObject(dc, old);
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);

        let info = ICONINFO { fIcon: true.into(), xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info).ok();
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon
    }
}

fn tray_data(hwnd: HWND, tip: &str) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_UID,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_TRAY,
        hIcon: tray_icon(),
        szTip: to_wide_fixed::<128>(tip),
        ..Default::default()
    }
}

fn tray_add(hwnd: HWND) {
    let data = tray_data(hwnd, "Force Stretch");
    let ok = unsafe { Shell_NotifyIconW(NIM_ADD, &data) }.as_bool();
    if !ok {
        log::write("트레이 아이콘 추가 실패");
    }
}

fn tray_update(hwnd: HWND, tip: &str) {
    let data = tray_data(hwnd, tip);
    unsafe {
        let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
    }
}

fn tray_remove(hwnd: HWND) {
    let data = tray_data(hwnd, "");
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &data);
    }
}
//#endregion

//#region 트레이 메뉴 (열 때마다 현재 상태로 생성)
fn append(menu: HMENU, flags: windows::Win32::UI::WindowsAndMessaging::MENU_ITEM_FLAGS, id: usize, text: &str) {
    let h = HSTRING::from(text);
    unsafe {
        let _ = AppendMenuW(menu, flags, id, &h);
    }
}

fn show_menu(hwnd: HWND) {
    // 메뉴 표시 중(모달 루프)에는 APP 를 빌려두지 않도록 필요한 값만 복사
    let Some((info, paused, interval, brk, upd_state)) = with_app(|app| {
        let st = app.sched.status(Now::now());
        (st.info, app.sched.is_paused(), app.sched.cfg.interval_secs, app.sched.cfg.break_secs, app.updater.state())
    }) else {
        return;
    };

    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        append(menu, MF_STRING | MF_GRAYED, 0, &info);
        append(menu, MF_SEPARATOR, 0, "");
        append(menu, MF_STRING, ID_START, "지금 스트레칭 시작");
        append(menu, MF_STRING, ID_PAUSE, if paused { "일시정지 해제" } else { "1시간 일시정지" });
        append(menu, MF_SEPARATOR, 0, "");

        if let Ok(sub) = CreatePopupMenu() {
            for m in INTERVAL_OPTIONS_MIN {
                let secs = m * 60;
                let flags = if secs == interval { MF_STRING | MF_CHECKED } else { MF_STRING };
                append(sub, flags, ID_INTERVAL_BASE + secs as usize, &format!("{m}분마다"));
            }
            append(menu, MF_POPUP, sub.0 as usize, "스트레칭 주기");
        }
        if let Ok(sub) = CreatePopupMenu() {
            for s in BREAK_OPTIONS_SECS {
                let flags = if s == brk { MF_STRING | MF_CHECKED } else { MF_STRING };
                append(sub, flags, ID_BREAK_BASE + s as usize, &fmt_dur_label(s));
            }
            append(menu, MF_POPUP, sub.0 as usize, "스트레칭 시간");
        }

        append(menu, MF_SEPARATOR, 0, "");
        let auto_flags = if autostart_enabled() { MF_STRING | MF_CHECKED } else { MF_STRING };
        append(menu, auto_flags, ID_AUTOSTART, "로그인 시 자동 실행");
        let upd_flags = if upd_state.clickable() { MF_STRING } else { MF_STRING | MF_GRAYED };
        append(menu, upd_flags, ID_UPDATE, &upd_state.menu_title());
        append(menu, MF_STRING, ID_OPEN_CONFIG, "설정 파일 열기…");
        append(menu, MF_STRING, ID_OPEN_LOG, "로그 파일 열기…");
        append(menu, MF_STRING, ID_QUIT, "종료");

        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        // 메뉴 바깥 클릭 시 닫히도록 전면 지정 후 표시 (Win32 관례)
        let _ = SetForegroundWindow(hwnd);
        let _ = TrackPopupMenu(menu, TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, pt.x, pt.y, None, hwnd, None);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
    }
}

fn open_url(url: &str) {
    let u = HSTRING::from(url);
    unsafe {
        ShellExecuteW(None, w!("open"), &u, PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

fn open_in_notepad(path: &std::path::Path) {
    // %APPDATA% 경로에 공백이 있을 수 있으므로 따옴표로 감쌈
    let p = HSTRING::from(format!("\"{}\"", path.display()));
    unsafe {
        ShellExecuteW(None, w!("open"), w!("notepad.exe"), &p, PCWSTR::null(), SW_SHOWNORMAL);
    }
}

fn on_command(id: usize) {
    match id {
        ID_START => start_break(),
        ID_PAUSE => {
            with_app(|app| app.sched.toggle_pause(Now::now()));
        }
        ID_AUTOSTART => set_autostart(!autostart_enabled()),
        ID_UPDATE => {
            with_app(|app| {
                if app.sched.is_breaking() {
                    return;
                }
                if matches!(app.updater.state(), upd::State::Available(_)) {
                    app.install_requested = true;
                    app.updater.install(update::install);
                } else {
                    app.updater.check();
                }
            });
        }
        ID_OPEN_CONFIG => {
            // 파일이 없으면 기본값으로 생성 후 열기
            if !config::config_path().exists() {
                Config::default().save();
            }
            open_in_notepad(&config::config_path());
        }
        ID_OPEN_LOG => open_in_notepad(&config::data_dir().join("force-stretch.log")),
        ID_QUIT => unsafe { PostQuitMessage(0) },
        id if id >= ID_BREAK_BASE => update_config(|c| c.break_secs = (id - ID_BREAK_BASE) as u64),
        id if id >= ID_INTERVAL_BASE => {
            update_config(|c| c.interval_secs = (id - ID_INTERVAL_BASE) as u64)
        }
        _ => {}
    }
    refresh_tray();
}

fn update_config(f: impl FnOnce(&mut Config)) {
    with_app(|app| {
        let mut cfg = app.sched.cfg.clone();
        f(&mut cfg);
        if !app.test_mode {
            cfg.save();
        }
        app.sched.set_config(cfg);
    });
}
//#endregion

//#region 매 틱 상태 진행
/// 업데이트 진행: 자동 확인, 설치 완료 시 새 버전 실행 후 종료, 설치 실패 시 릴리스 페이지 안내.
/// 종료를 시작했으면 true.
fn handle_update() -> bool {
    let Some((state, requested, breaking)) = with_app(|app| {
        if !app.test_mode {
            app.updater.tick();
        }
        (app.updater.state(), std::mem::take(&mut app.install_requested), app.sched.is_breaking())
    }) else {
        return false;
    };
    match state {
        // 스트레칭 중에는 다시 시작하지 않고 끝날 때까지 기다림
        upd::State::Ready(_) if !breaking => match update::relaunch() {
            Ok(()) => {
                log::write("새 버전을 실행하고 종료합니다");
                unsafe { PostQuitMessage(0) };
                return true;
            }
            Err(e) => {
                log::write(&e);
                open_url(upd::RELEASES_PAGE);
                unsafe { PostQuitMessage(0) };
                return true;
            }
        },
        upd::State::Failed(_) if requested => open_url(upd::RELEASES_PAGE),
        // 설치 진행 중이면 결과가 나올 때까지 플래그 유지
        upd::State::Installing(_) if requested => {
            with_app(|app| app.install_requested = true);
        }
        _ => {}
    }
    false
}

fn on_tick() {
    if handle_update() {
        return;
    }
    let now = Now::now();
    let event = with_app(|app| {
        let ev = app.sched.tick(now, idle_secs());
        if app.sched.is_breaking() && overlay::typed_matches(&app.sched.cfg.emergency_phrase) {
            log::write("비상 해제 문구 입력됨");
            Event::EndBreak
        } else {
            ev
        }
    });

    match event {
        Some(Event::StartBreak) => start_break(),
        Some(Event::EndBreak) => end_break(),
        Some(Event::None) => {
            with_app(|app| {
                if !app.sched.is_breaking() {
                    return;
                }
                if std::mem::take(&mut app.displays_dirty) {
                    overlay::destroy_all(&mut app.overlays);
                    app.overlays = overlay::build_all();
                }
                update_paint(app, now);
                overlay::keep_on_top(&app.overlays);
                if app.test_mode {
                    let fg = unsafe { GetForegroundWindow() };
                    log::write(&format!(
                        "진단: overlays={} monitors={} foreground_is_overlay={}",
                        app.overlays.len(),
                        overlay::monitor_rects().len(),
                        app.overlays.contains(&fg)
                    ));
                }
            });
        }
        None => {}
    }
    refresh_tray();
}

fn update_paint(app: &App, now: Now) {
    let Some(view) = app.sched.break_view(now) else { return };
    let footer = if app.sched.cfg.emergency_phrase.is_empty() {
        String::new()
    } else {
        format!(
            "비상 해제: 설정 파일의 emergency_phrase 를 그대로 타이핑하세요 · 입력 {}자",
            overlay::typed_len()
        )
    };
    overlay::set_paint_text(overlay::PaintText {
        countdown: view.countdown,
        tip: view.tip.to_string(),
        footer,
    });
    overlay::invalidate(&app.overlays);
}

fn refresh_tray() {
    with_app(|app| {
        let st = app.sched.status(Now::now());
        // 트레이는 글자를 못 띄우므로 툴팁에 남은 시간 표시 (이모지 제외)
        let extra = match app.updater.state() {
            upd::State::Available(v) => format!(" · 새 버전 v{v}"),
            _ => String::new(),
        };
        tray_update(app.hwnd, &format!("Force Stretch · {}{extra}", st.info));
    });
}
//#endregion

//#region 스트레칭 시작/종료
fn start_break() {
    let now = Now::now();
    with_app(|app| {
        if app.sched.is_breaking() {
            return;
        }
        let ends_at = app.sched.start_break(now);
        failsafe::arm(ends_at);
        overlay::clear_typed();
        let hooked = overlay::install_hook();
        app.displays_dirty = false;
        overlay::destroy_all(&mut app.overlays);
        app.overlays = overlay::build_all();
        update_paint(app, now);
        overlay::keep_on_top(&app.overlays);
        log::write(&format!("스트레칭 시작: overlays={} hook={hooked}", app.overlays.len()));
    });
    unsafe {
        let _ = MessageBeep(MB_ICONEXCLAMATION);
    }
}

fn end_break() {
    let test_mode = with_app(|app| {
        overlay::remove_hook();
        overlay::destroy_all(&mut app.overlays);
        app.sched.end_break();
        app.test_mode
    })
    .unwrap_or(false);
    failsafe::disarm();
    overlay::clear_typed();
    log::write("스트레칭 종료");
    unsafe {
        let _ = MessageBeep(MB_ICONASTERISK);
    }
    if test_mode {
        log::write("시험 모드: 스트레칭 1회 완료, 종료합니다");
        unsafe { PostQuitMessage(0) };
    }
}
//#endregion

//#region 숨은 메시지 창 프로시저 (타이머·트레이·메뉴 명령 수신)
unsafe extern "system" fn main_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER if wparam.0 == TIMER_ID => {
            on_tick();
            LRESULT(0)
        }
        WM_TRAY => {
            let ev = (lparam.0 & 0xFFFF) as u32;
            if ev == WM_RBUTTONUP || ev == WM_LBUTTONUP {
                show_menu(hwnd);
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            on_command(wparam.0 & 0xFFFF);
            LRESULT(0)
        }
        WM_DISPLAYCHANGE => {
            with_app(|app| app.displays_dirty = true);
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => {
            // 탐색기 재시작 시 트레이 아이콘 다시 등록
            let tb = with_app(|app| app.taskbar_created_msg).unwrap_or(0);
            if tb != 0 && msg == tb {
                tray_add(hwnd);
                return LRESULT(0);
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
    }
}
//#endregion

//#region 진입점
pub fn run(cfg: Config, test_mode: bool) {
    unsafe {
        //#region 고해상도(모니터별 배율) 대응: 창 생성 전에 설정해야 배율 모니터도 전부 덮음
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        //#endregion

        // 중복 실행 방지는 main.rs 의 파일 잠금이 담당
        update::cleanup_old();

        //#region 숨은 메시지 창 + 트레이 + 타이머
        let hinstance = GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSW {
            lpfnWndProc: Some(main_proc),
            hInstance: hinstance.into(),
            lpszClassName: w!("ForceStretchMain"),
            ..Default::default()
        };
        RegisterClassW(&wc);
        overlay::register_class();

        let hwnd = match CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("ForceStretchMain"),
            w!("Force Stretch"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinstance.into()),
            None,
        ) {
            Ok(h) => h,
            Err(e) => {
                log::write(&format!("메인 창 생성 실패: {e}"));
                return;
            }
        };

        let taskbar_created_msg = RegisterWindowMessageW(w!("TaskbarCreated"));
        APP.with(|a| {
            *a.borrow_mut() = Some(App {
                hwnd,
                sched: Scheduler::new(cfg, Now::now()),
                test_mode,
                overlays: Vec::new(),
                displays_dirty: false,
                taskbar_created_msg,
                updater: Updater::new(),
                install_requested: false,
            })
        });

        tray_add(hwnd);
        SetTimer(Some(hwnd), TIMER_ID, 500, None);
        log::write(&format!("시작 (test_mode={test_mode})"));
        //#endregion

        //#region 메시지 루프
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        //#endregion

        //#region 정리
        with_app(|app| {
            overlay::destroy_all(&mut app.overlays);
        });
        overlay::remove_hook();
        tray_remove(hwnd);
        log::write("종료");
        //#endregion
    }
}
//#endregion
