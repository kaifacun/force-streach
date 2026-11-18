use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadOnly, sel};
use objc2_app_kit::{
    NSBackingStoreType, NSButton, NSControlStateValueOff, NSControlStateValueOn, NSFont, NSMenu,
    NSMenuItem, NSTextAlignment, NSTextField, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize, NSString};

use crate::config::Config;

//#region 설정 창 구성 요소
pub struct SettingsWindow {
    pub window: Retained<NSWindow>,
    interval_min: Retained<NSTextField>,
    break_min: Retained<NSTextField>,
    warn_sec: Retained<NSTextField>,
    idle_reset: Retained<NSButton>,
    phrase: Retained<NSTextField>,
    error: Retained<NSTextField>,
}

const W: f64 = 460.0;
const H: f64 = 330.0;
const LABEL_W: f64 = 170.0;
const ROW_H: f64 = 24.0;

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}

fn fmt_num(v: f64) -> String {
    if v.fract() == 0.0 { format!("{}", v as i64) } else { format!("{v:.1}") }
}
//#endregion

impl SettingsWindow {
    //#region 창 생성 (target: 저장/취소 버튼 액션을 받을 delegate)
    pub fn new(cfg: &Config, target: &AnyObject, mtm: MainThreadMarker) -> Self {
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                rect(0.0, 0.0, W, H),
                NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        // SAFETY: Retained 로 직접 보관
        unsafe { window.setReleasedWhenClosed(false) };
        window.setTitle(&NSString::from_str("Force Stretch 설정"));
        let view = window.contentView().expect("content view");

        //#region 입력 행 배치
        let mut y = H - 56.0;
        let mut row = |label: &str, field: &NSTextField, field_w: f64, hint: &str| {
            let l = NSTextField::labelWithString(&NSString::from_str(label), mtm);
            l.setFrame(rect(20.0, y, LABEL_W, ROW_H));
            l.setAlignment(NSTextAlignment::Right);
            view.addSubview(&l);
            field.setFrame(rect(20.0 + LABEL_W + 10.0, y, field_w, ROW_H));
            view.addSubview(field);
            if !hint.is_empty() {
                let h = NSTextField::labelWithString(&NSString::from_str(hint), mtm);
                h.setFrame(rect(20.0 + LABEL_W + 18.0 + field_w, y, 120.0, ROW_H));
                h.setFont(Some(&NSFont::systemFontOfSize(12.0)));
                view.addSubview(&h);
            }
            y -= 40.0;
        };

        let text = |s: &str| NSTextField::textFieldWithString(&NSString::from_str(s), mtm);
        let interval_min = text(&fmt_num(cfg.interval_secs as f64 / 60.0));
        let break_min = text(&fmt_num(cfg.break_secs as f64 / 60.0));
        let warn_sec = text(&cfg.warn_secs.to_string());
        let phrase = text(&cfg.emergency_phrase);
        phrase.setPlaceholderString(Some(&NSString::from_str("비워두면 비상 해제 불가")));

        row("스트레칭 주기", &interval_min, 70.0, "분마다 (1~1440)");
        row("스트레칭 시간", &break_min, 70.0, "분 (0.5~30)");
        row("시작 전 경고", &warn_sec, 70.0, "초 전 메뉴바 ⚠️");
        row("비상 해제 문구", &phrase, 240.0, "");

        let idle_reset = unsafe {
            NSButton::checkboxWithTitle_target_action(
                &NSString::from_str("5분 이상 자리를 비우면 주기를 처음부터 다시 시작"),
                None,
                None,
                mtm,
            )
        };
        idle_reset.setFrame(rect(20.0 + LABEL_W + 10.0 - 2.0, y, 270.0, ROW_H));
        idle_reset.setState(if cfg.idle_reset { NSControlStateValueOn } else { NSControlStateValueOff });
        view.addSubview(&idle_reset);
        //#endregion

        //#region 오류 표시 + 버튼
        let error = NSTextField::labelWithString(&NSString::from_str(""), mtm);
        error.setFrame(rect(20.0, 52.0, W - 40.0, 20.0));
        error.setTextColor(Some(&objc2_app_kit::NSColor::systemRedColor()));
        view.addSubview(&error);

        let save = unsafe {
            NSButton::buttonWithTitle_target_action(
                &NSString::from_str("저장"),
                Some(target),
                Some(sel!(saveSettings:)),
                mtm,
            )
        };
        save.setFrame(rect(W - 120.0, 14.0, 100.0, 32.0));
        save.setKeyEquivalent(&NSString::from_str("\r"));
        view.addSubview(&save);

        let cancel = unsafe {
            NSButton::buttonWithTitle_target_action(
                &NSString::from_str("취소"),
                Some(target),
                Some(sel!(closeSettings:)),
                mtm,
            )
        };
        cancel.setFrame(rect(W - 230.0, 14.0, 100.0, 32.0));
        cancel.setKeyEquivalent(&NSString::from_str("\u{1b}"));
        view.addSubview(&cancel);
        //#endregion

        window.center();
        Self { window, interval_min, break_min, warn_sec, idle_reset, phrase, error }
    }
    //#endregion

    //#region 입력값 검증 → 새 설정 반환 (실패 시 창에 오류 표시)
    pub fn read(&self, base: &Config) -> Option<Config> {
        let num = |f: &NSTextField| f.stringValue().to_string().trim().parse::<f64>().ok();
        let interval = num(&self.interval_min).filter(|v| (1.0..=1440.0).contains(v));
        let brk = num(&self.break_min).filter(|v| (0.5..=30.0).contains(v));
        let warn = num(&self.warn_sec).filter(|v| (0.0..=600.0).contains(v));
        let (Some(interval), Some(brk), Some(warn)) = (interval, brk, warn) else {
            self.error.setStringValue(&NSString::from_str(
                "숫자를 확인하세요: 주기 1~1440분, 시간 0.5~30분, 경고 0~600초",
            ));
            return None;
        };
        self.error.setStringValue(&NSString::from_str(""));
        Some(Config {
            interval_secs: (interval * 60.0).round() as u64,
            break_secs: (brk * 60.0).round() as u64,
            warn_secs: warn as u64,
            idle_reset: self.idle_reset.state() == NSControlStateValueOn,
            emergency_phrase: self.phrase.stringValue().to_string().trim().to_lowercase(),
            ..base.clone()
        })
    }
    //#endregion
}

//#region 앱 메인 메뉴 (설정 창 텍스트 필드에서 복사/붙여넣기 단축키가 동작하도록 Edit 메뉴만 등록)
pub fn install_edit_menu(mtm: MainThreadMarker) -> Retained<NSMenu> {
    let main = NSMenu::new(mtm);
    let edit_root = NSMenuItem::new(mtm);
    let edit = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str("Edit"));
    for (title, action, key) in [
        ("Cut", sel!(cut:), "x"),
        ("Copy", sel!(copy:), "c"),
        ("Paste", sel!(paste:), "v"),
        ("Select All", sel!(selectAll:), "a"),
        ("Undo", sel!(undo:), "z"),
    ] {
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(title),
                Some(action),
                &NSString::from_str(key),
            )
        };
        edit.addItem(&item);
    }
    edit_root.setSubmenu(Some(&edit));
    main.addItem(&edit_root);
    main
}
//#endregion
