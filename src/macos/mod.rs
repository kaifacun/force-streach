//! macOS 구현: 메뉴바 상주 + AppKit 전체 화면 가림 창
mod login;
mod overlay;
mod settings;
pub mod update;

use std::cell::{Cell, OnceCell, RefCell};
use std::panic::AssertUnwindSafe;
use crate::common::schedule::Now;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate,
    NSApplicationDidChangeScreenParametersNotification, NSApplicationPresentationOptions, NSButton,
    NSControlStateValueOff, NSControlStateValueOn, NSMenu, NSMenuItem, NSSound, NSStatusBar,
    NSStatusItem, NSVariableStatusItemLength, NSWorkspace,
};
use objc2_foundation::{
    MainThreadMarker, NSNotification, NSNotificationCenter, NSObject, NSObjectProtocol,
    NSRunLoop, NSRunLoopCommonModes, NSString, NSTimer, NSURL,
};

use crate::common::config::{self, Config};
use crate::common::schedule::{Event, Scheduler, fmt_dur_label};
use crate::common::update::{self as upd, Updater};
use crate::common::{failsafe, log};
use overlay::Overlay;
use settings::SettingsWindow;

//#region CoreGraphics FFI (마지막 입력 이후 경과 시간)
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventSourceSecondsSinceLastEventType(state_id: i32, event_type: u32) -> f64;
}

/// 키보드·마우스 입력이 마지막으로 있었던 뒤 지난 초
fn idle_secs() -> f64 {
    // 0 = kCGEventSourceStateCombinedSessionState, u32::MAX = kCGAnyInputEventType
    unsafe { CGEventSourceSecondsSinceLastEventType(0, u32::MAX) }
}
//#endregion

//#region 상태 정의
struct Inner {
    sched: Scheduler,
    test_mode: bool,
    overlays: Vec<Overlay>,
    updater: Updater,
    /// 사용자가 설치를 눌렀음 (실패하면 릴리스 페이지를 열어 수동 설치 안내)
    install_requested: bool,
}

struct Ivars {
    inner: RefCell<Inner>,
    /// 화면 구성 변경 플래그 (알림 콜백에서 RefCell 재진입을 피하려고 분리)
    screens_dirty: Cell<bool>,
    status_item: OnceCell<Retained<NSStatusItem>>,
    info_item: OnceCell<Retained<NSMenuItem>>,
    pause_item: OnceCell<Retained<NSMenuItem>>,
    interval_menu: OnceCell<Retained<NSMenu>>,
    break_menu: OnceCell<Retained<NSMenu>>,
    login_item: OnceCell<Retained<NSMenuItem>>,
    update_item: OnceCell<Retained<NSMenuItem>>,
    settings: RefCell<Option<SettingsWindow>>,
}
//#endregion

define_class!(
    // SAFETY: NSObject 서브클래싱 요구사항 없음, Drop 미구현
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Ivars]
    struct AppDelegate;

    unsafe impl NSObjectProtocol for AppDelegate {}

    unsafe impl NSApplicationDelegate for AppDelegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, _n: &NSNotification) {
            self.setup();
        }
    }

    impl AppDelegate {
        //#region 주기 타이머
        #[unsafe(method(tick:))]
        fn tick(&self, _timer: &NSTimer) {
            self.on_tick();
        }
        //#endregion

        //#region 화면 구성 변경 알림 (모니터 연결/해제)
        #[unsafe(method(screensChanged:))]
        fn screens_changed(&self, _n: &NSNotification) {
            self.ivars().screens_dirty.set(true);
        }
        //#endregion

        //#region 메뉴 액션
        #[unsafe(method(startNow:))]
        fn start_now(&self, _sender: &NSMenuItem) {
            self.start_break();
        }

        #[unsafe(method(togglePause:))]
        fn toggle_pause(&self, _sender: &NSMenuItem) {
            self.ivars().inner.borrow_mut().sched.toggle_pause(Now::now());
            self.refresh_menu();
        }

        #[unsafe(method(setInterval:))]
        fn set_interval(&self, sender: &NSMenuItem) {
            self.update_config(|c| c.interval_secs = sender.tag() as u64);
        }

        #[unsafe(method(setBreak:))]
        fn set_break(&self, sender: &NSMenuItem) {
            self.update_config(|c| c.break_secs = sender.tag() as u64);
        }

        #[unsafe(method(openSettings:))]
        fn open_settings(&self, _sender: &NSMenuItem) {
            self.show_settings();
        }

        #[unsafe(method(saveSettings:))]
        fn save_settings(&self, _sender: &NSButton) {
            self.apply_settings();
        }

        #[unsafe(method(closeSettings:))]
        fn close_settings(&self, _sender: &NSButton) {
            if let Some(sw) = self.ivars().settings.borrow_mut().take() {
                sw.window.close();
            }
        }

        #[unsafe(method(openConfig:))]
        fn open_config(&self, _sender: &NSMenuItem) {
            let path = config::config_path();
            let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
            NSWorkspace::sharedWorkspace().openURL(&url);
        }

        #[unsafe(method(toggleLogin:))]
        fn toggle_login(&self, _sender: &NSMenuItem) {
            login::set_enabled(!login::is_enabled());
            self.refresh_menu();
        }

        #[unsafe(method(updateAction:))]
        fn update_action(&self, _sender: &NSMenuItem) {
            self.on_update_clicked();
        }

        #[unsafe(method(quit:))]
        fn quit(&self, _sender: &NSMenuItem) {
            NSApplication::sharedApplication(self.mtm()).terminate(None);
        }
        //#endregion
    }
);

impl AppDelegate {
    fn new(mtm: MainThreadMarker, cfg: Config, test_mode: bool) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            inner: RefCell::new(Inner {
                sched: Scheduler::new(cfg, Now::now()),
                test_mode,
                overlays: Vec::new(),
                updater: Updater::new(),
                install_requested: false,
            }),
            screens_dirty: Cell::new(false),
            status_item: OnceCell::new(),
            info_item: OnceCell::new(),
            pause_item: OnceCell::new(),
            interval_menu: OnceCell::new(),
            break_menu: OnceCell::new(),
            login_item: OnceCell::new(),
            update_item: OnceCell::new(),
            settings: RefCell::new(None),
        });
        unsafe { msg_send![super(this), init] }
    }

    fn target(&self) -> &AnyObject {
        self
    }

    //#region 초기화: 메뉴바 아이템, 타이머, 알림 등록
    fn setup(&self) {
        let mtm = self.mtm();

        //#region 메뉴바 아이템과 메뉴 구성
        let status_item =
            NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
        let menu = NSMenu::new(mtm);
        menu.setAutoenablesItems(false);

        let info = self.menu_item("", None, 0);
        info.setEnabled(false);
        menu.addItem(&info);
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        menu.addItem(&self.menu_item("지금 스트레칭 시작", Some(sel!(startNow:)), 0));
        let pause = self.menu_item("1시간 일시정지", Some(sel!(togglePause:)), 0);
        menu.addItem(&pause);
        menu.addItem(&NSMenuItem::separatorItem(mtm));

        let interval_menu = NSMenu::new(mtm);
        for m in [20u64, 30, 45, 50, 60, 90] {
            interval_menu.addItem(&self.menu_item(
                &format!("{m}분마다"),
                Some(sel!(setInterval:)),
                (m * 60) as isize,
            ));
        }
        let interval_root = self.menu_item("스트레칭 주기", None, 0);
        interval_root.setSubmenu(Some(&interval_menu));
        menu.addItem(&interval_root);

        let break_menu = NSMenu::new(mtm);
        for s in [60u64, 120, 180, 300, 600] {
            break_menu.addItem(&self.menu_item(&fmt_dur_label(s), Some(sel!(setBreak:)), s as isize));
        }
        let break_root = self.menu_item("스트레칭 시간", None, 0);
        break_root.setSubmenu(Some(&break_menu));
        menu.addItem(&break_root);

        menu.addItem(&NSMenuItem::separatorItem(mtm));
        menu.addItem(&self.menu_item("설정…", Some(sel!(openSettings:)), 0));
        menu.addItem(&self.menu_item("설정 파일 열기…", Some(sel!(openConfig:)), 0));
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        let login_item = self.menu_item("로그인 시 자동 실행", Some(sel!(toggleLogin:)), 0);
        menu.addItem(&login_item);
        let update_item = self.menu_item("", Some(sel!(updateAction:)), 0);
        menu.addItem(&update_item);
        menu.addItem(&self.menu_item("종료", Some(sel!(quit:)), 0));
        status_item.setMenu(Some(&menu));
        NSApplication::sharedApplication(mtm)
            .setMainMenu(Some(&settings::install_edit_menu(mtm)));

        let iv = self.ivars();
        let _ = iv.status_item.set(status_item);
        let _ = iv.info_item.set(info);
        let _ = iv.pause_item.set(pause);
        let _ = iv.interval_menu.set(interval_menu);
        let _ = iv.break_menu.set(break_menu);
        let _ = iv.login_item.set(login_item);
        let _ = iv.update_item.set(update_item);
        //#endregion

        //#region 0.5초 주기 타이머 (메뉴가 열려 있어도 돌도록 common mode 등록)
        let timer = unsafe {
            NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                0.5,
                self.target(),
                sel!(tick:),
                None,
                true,
            )
        };
        unsafe { NSRunLoop::currentRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes) };
        //#endregion

        //#region 모니터 연결/해제 감지
        unsafe {
            NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
                self.target(),
                sel!(screensChanged:),
                Some(NSApplicationDidChangeScreenParametersNotification),
                None,
            );
        }
        //#endregion

        self.refresh_menu();
    }

    fn menu_item(&self, title: &str, action: Option<Sel>, tag: isize) -> Retained<NSMenuItem> {
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(self.mtm()),
                &NSString::from_str(title),
                action,
                &NSString::from_str(""),
            )
        };
        if action.is_some() {
            unsafe { item.setTarget(Some(self.target())) };
        }
        item.setTag(tag);
        item
    }
    //#endregion

    //#region 매 틱 상태 진행
    fn on_tick(&self) {
        if self.handle_update() {
            return;
        }
        let now = Now::now();
        let event = {
            let mut inner = self.ivars().inner.borrow_mut();
            let ev = inner.sched.tick(now, idle_secs());
            if inner.sched.is_breaking() && overlay::typed_matches(&inner.sched.cfg.emergency_phrase) {
                log::write("비상 해제 문구 입력됨");
                Event::EndBreak
            } else {
                ev
            }
        };

        match event {
            Event::StartBreak => self.start_break(),
            Event::EndBreak => self.end_break(),
            Event::None => {
                if self.is_breaking() {
                    if self.ivars().screens_dirty.replace(false) {
                        self.rebuild_overlays();
                    }
                    self.update_overlays(now);
                    self.keep_on_top();
                    if self.ivars().inner.borrow().test_mode {
                        self.log_diagnostics();
                    }
                }
            }
        }
        self.update_status_title();
    }

    fn is_breaking(&self) -> bool {
        self.ivars().inner.borrow().sched.is_breaking()
    }
    //#endregion

    //#region 업데이트
    fn on_update_clicked(&self) {
        let mut inner = self.ivars().inner.borrow_mut();
        if inner.sched.is_breaking() {
            return;
        }
        if matches!(inner.updater.state(), upd::State::Available(_)) {
            // 이 Mac 에서 자동 교체가 불가능하면 바로 릴리스 페이지 안내
            if let Err(e) = update::app_bundle() {
                log::write(&format!("자동 설치 불가: {e}"));
                drop(inner);
                open_url(upd::RELEASES_PAGE);
                return;
            }
            inner.install_requested = true;
            inner.updater.install(update::install);
        } else {
            inner.updater.check();
        }
    }

    /// 자동 확인, 설치 완료 시 새 버전 실행 후 종료, 설치 실패 시 릴리스 페이지 안내. 종료를 시작했으면 true.
    fn handle_update(&self) -> bool {
        let (state, requested, breaking) = {
            let mut inner = self.ivars().inner.borrow_mut();
            if !inner.test_mode {
                inner.updater.tick();
            }
            let requested = std::mem::take(&mut inner.install_requested);
            (inner.updater.state(), requested, inner.sched.is_breaking())
        };
        match state {
            // 스트레칭 중에는 다시 시작하지 않고 끝날 때까지 기다림
            upd::State::Ready(_) if !breaking => {
                match update::app_bundle().and_then(|b| update::relaunch(&b)) {
                    Ok(()) => log::write("새 버전을 실행하고 종료합니다"),
                    Err(e) => {
                        log::write(&e);
                        open_url(upd::RELEASES_PAGE);
                    }
                }
                NSApplication::sharedApplication(self.mtm()).terminate(None);
                return true;
            }
            upd::State::Failed(_) if requested => open_url(upd::RELEASES_PAGE),
            // 설치 진행 중이면 결과가 나올 때까지 플래그 유지
            upd::State::Installing(_) if requested => self.ivars().inner.borrow_mut().install_requested = true,
            _ => {}
        }
        false
    }
    //#endregion

    //#region 스트레칭 시작/종료
    fn start_break(&self) {
        if self.is_breaking() {
            return;
        }
        let mtm = self.mtm();
        let now = Now::now();
        let ends_at = self.ivars().inner.borrow_mut().sched.start_break(now);
        failsafe::arm(ends_at);
        overlay::clear_typed();
        self.ivars().screens_dirty.set(false);
        self.rebuild_overlays();

        //#region 앱 활성화 + 키오스크 모드 (Dock/메뉴바 숨김, 앱 전환 차단)
        let app = NSApplication::sharedApplication(mtm);
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
        let opts = NSApplicationPresentationOptions::HideDock
            | NSApplicationPresentationOptions::HideMenuBar
            | NSApplicationPresentationOptions::DisableAppleMenu
            | NSApplicationPresentationOptions::DisableProcessSwitching
            | NSApplicationPresentationOptions::DisableHideApplication;
        if let Err(e) = objc2::exception::catch(AssertUnwindSafe(|| app.setPresentationOptions(opts))) {
            log::write(&format!("presentation options 설정 실패: {e:?}"));
        }
        //#endregion

        if let Some(sound) = NSSound::soundNamed(&NSString::from_str("Glass")) {
            sound.play();
        }
        self.update_overlays(now);
        self.keep_on_top();
        self.update_status_title();
    }

    fn end_break(&self) {
        let mtm = self.mtm();
        let test_mode = {
            let mut inner = self.ivars().inner.borrow_mut();
            for o in inner.overlays.drain(..) {
                o.close();
            }
            inner.sched.end_break();
            inner.test_mode
        };
        failsafe::disarm();
        overlay::clear_typed();

        let app = NSApplication::sharedApplication(mtm);
        let _ = objc2::exception::catch(AssertUnwindSafe(|| {
            app.setPresentationOptions(NSApplicationPresentationOptions::Default)
        }));
        if let Some(sound) = NSSound::soundNamed(&NSString::from_str("Hero")) {
            sound.play();
        }
        if test_mode {
            log::write("시험 모드: 스트레칭 1회 완료, 종료합니다");
            app.terminate(None);
        }
        self.update_status_title();
    }

    /// 시험 모드 진단: 활성화/키 창/키오스크 옵션 적용 여부 출력
    fn log_diagnostics(&self) {
        let app = NSApplication::sharedApplication(self.mtm());
        let inner = self.ivars().inner.borrow();
        let key = inner.overlays.first().map(|o| o.window.isKeyWindow()).unwrap_or(false);
        log::write(&format!(
            "진단: active={} key={} screens={} sys_opts={:#x}",
            app.isActive(),
            key,
            inner.overlays.len(),
            app.currentSystemPresentationOptions().0
        ));
    }
    //#endregion

    //#region 가림 창 관리
    fn rebuild_overlays(&self) {
        let mtm = self.mtm();
        let mut inner = self.ivars().inner.borrow_mut();
        for o in inner.overlays.drain(..) {
            o.close();
        }
        inner.overlays = overlay::build_all(mtm);
        for o in &inner.overlays {
            o.show();
        }
    }

    fn keep_on_top(&self) {
        let app = NSApplication::sharedApplication(self.mtm());
        if !app.isActive() {
            #[allow(deprecated)]
            app.activateIgnoringOtherApps(true);
        }
        let inner = self.ivars().inner.borrow();
        for o in &inner.overlays {
            o.window.orderFrontRegardless();
        }
        if let Some(first) = inner.overlays.first() {
            if !first.window.isKeyWindow() {
                first.window.makeKeyWindow();
            }
        }
    }

    fn update_overlays(&self, now: Now) {
        let inner = self.ivars().inner.borrow();
        let Some(view) = inner.sched.break_view(now) else {
            return;
        };
        let countdown = NSString::from_str(&view.countdown);
        let tip = NSString::from_str(view.tip);
        let footer = if inner.sched.cfg.emergency_phrase.is_empty() {
            NSString::from_str("")
        } else {
            NSString::from_str(&format!(
                "비상 해제: 설정에 등록한 비상 해제 문구를 그대로 타이핑하세요 · 입력 {}자",
                overlay::typed_len()
            ))
        };
        for o in &inner.overlays {
            o.countdown.setStringValue(&countdown);
            o.tip.setStringValue(&tip);
            o.footer.setStringValue(&footer);
        }
    }
    //#endregion

    //#region 설정 변경/설정 창
    fn update_config(&self, f: impl FnOnce(&mut Config)) {
        {
            let mut inner = self.ivars().inner.borrow_mut();
            let mut cfg = inner.sched.cfg.clone();
            f(&mut cfg);
            if !inner.test_mode {
                cfg.save();
            }
            inner.sched.set_config(cfg);
        }
        self.refresh_menu();
    }

    fn show_settings(&self) {
        let mtm = self.mtm();
        let mut slot = self.ivars().settings.borrow_mut();
        // 열 때마다 현재 설정으로 새로 생성 (닫기 버튼으로 닫힌 낡은 창 재사용 방지)
        if let Some(old) = slot.take() {
            old.window.close();
        }
        let cfg = self.ivars().inner.borrow().sched.cfg.clone();
        *slot = Some(SettingsWindow::new(&cfg, self.target(), mtm));
        let app = NSApplication::sharedApplication(mtm);
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
        if let Some(sw) = slot.as_ref() {
            sw.window.makeKeyAndOrderFront(None);
        }
    }

    fn apply_settings(&self) {
        let new_cfg = {
            let slot = self.ivars().settings.borrow();
            let Some(sw) = slot.as_ref() else { return };
            let base = self.ivars().inner.borrow().sched.cfg.clone();
            match sw.read(&base) {
                Some(c) => c,
                None => return,
            }
        };
        self.update_config(|c| *c = new_cfg);
        if let Some(sw) = self.ivars().settings.borrow_mut().take() {
            sw.window.close();
        }
    }
    //#endregion

    //#region 메뉴바 표시 갱신
    fn update_status_title(&self) {
        let iv = self.ivars();
        let (status, upd_state) = {
            let inner = iv.inner.borrow();
            (inner.sched.status(Now::now()), inner.updater.state())
        };
        if let Some(item) = iv.status_item.get() {
            if let Some(button) = item.button(self.mtm()) {
                // 새 버전이 있으면 메뉴바에 작은 표시
                let mark = if matches!(upd_state, upd::State::Available(_)) { " ⬆︎" } else { "" };
                button.setTitle(&NSString::from_str(&format!("{}{mark}", status.title)));
            }
        }
        if let Some(u) = iv.update_item.get() {
            u.setTitle(&NSString::from_str(&upd_state.menu_title()));
            u.setEnabled(upd_state.clickable());
        }
        if let Some(info_item) = iv.info_item.get() {
            info_item.setTitle(&NSString::from_str(&status.info));
        }
    }

    fn refresh_menu(&self) {
        let iv = self.ivars();
        {
            let inner = iv.inner.borrow();
            let mark = |menu: &NSMenu, value: u64| {
                for i in 0..menu.numberOfItems() {
                    if let Some(item) = menu.itemAtIndex(i) {
                        item.setState(if item.tag() as u64 == value {
                            NSControlStateValueOn
                        } else {
                            NSControlStateValueOff
                        });
                    }
                }
            };
            if let Some(m) = iv.interval_menu.get() {
                mark(m, inner.sched.cfg.interval_secs);
            }
            if let Some(m) = iv.break_menu.get() {
                mark(m, inner.sched.cfg.break_secs);
            }
            if let Some(l) = iv.login_item.get() {
                l.setState(if login::is_enabled() { NSControlStateValueOn } else { NSControlStateValueOff });
            }
            if let Some(p) = iv.pause_item.get() {
                let t = if inner.sched.is_paused() { "일시정지 해제" } else { "1시간 일시정지" };
                p.setTitle(&NSString::from_str(t));
            }
        }
        self.update_status_title();
    }
    //#endregion
}

fn open_url(url: &str) {
    if let Some(u) = NSURL::URLWithString(&NSString::from_str(url)) {
        NSWorkspace::sharedWorkspace().openURL(&u);
    }
}

//#region 진입점
pub fn run(cfg: Config, test_mode: bool) {
    let mtm = MainThreadMarker::new().expect("메인 스레드에서 실행해야 합니다");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    let delegate = AppDelegate::new(mtm, cfg, test_mode);
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    app.run();
}
//#endregion
