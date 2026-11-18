use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::{MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSEvent, NSFont, NSScreen, NSTextAlignment, NSTextField,
    NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize, NSString};

//#region CoreGraphics FFI (최상위 창 레벨)
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGShieldingWindowLevel() -> i32;
}
//#endregion

//#region 키 입력 버퍼 (비상 해제 문구 감지용)
thread_local! {
    static TYPED: RefCell<String> = const { RefCell::new(String::new()) };
}

/// 지금까지 입력된 문자열이 비상 문구로 끝나는지 확인
pub fn typed_matches(phrase: &str) -> bool {
    if phrase.is_empty() {
        return false;
    }
    TYPED.with(|t| t.borrow().ends_with(phrase))
}

pub fn clear_typed() {
    TYPED.with(|t| t.borrow_mut().clear());
}

/// 현재까지 입력 길이 (화면에 진행 표시용)
pub fn typed_len() -> usize {
    TYPED.with(|t| t.borrow().chars().count())
}
//#endregion

//#region 물리 키코드 → 영문(US 배열) 문자 변환
fn keycode_to_char(code: u16) -> Option<char> {
    const MAP: &[(u16, char)] = &[
        (0, 'a'), (11, 'b'), (8, 'c'), (2, 'd'), (14, 'e'), (3, 'f'), (5, 'g'), (4, 'h'),
        (34, 'i'), (38, 'j'), (40, 'k'), (37, 'l'), (46, 'm'), (45, 'n'), (31, 'o'), (35, 'p'),
        (12, 'q'), (15, 'r'), (1, 's'), (17, 't'), (32, 'u'), (9, 'v'), (13, 'w'), (7, 'x'),
        (16, 'y'), (6, 'z'), (49, ' '), (29, '0'), (18, '1'), (19, '2'), (20, '3'), (21, '4'),
        (23, '5'), (22, '6'), (26, '7'), (28, '8'), (25, '9'), (27, '-'), (43, ','), (47, '.'),
    ];
    MAP.iter().find(|(k, _)| *k == code).map(|(_, c)| *c)
}
//#endregion

//#region 가림 창 클래스 (테두리 없는 창이 키 입력을 받도록 재정의)
define_class!(
    // SAFETY: NSWindow 서브클래싱 요구사항 없음, Drop 미구현
    #[unsafe(super(NSWindow))]
    #[thread_kind = MainThreadOnly]
    pub struct OverlayWindow;

    impl OverlayWindow {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key(&self) -> bool {
            true
        }

        #[unsafe(method(canBecomeMainWindow))]
        fn can_become_main(&self) -> bool {
            true
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            // 한글 입력 상태여도 같은 물리 키면 같은 영문자로 인식되도록 keyCode 로 해석
            let code = event.keyCode();
            {
                TYPED.with(|t| {
                    let mut t = t.borrow_mut();
                    if code == 51 {
                        t.pop();
                    } else if let Some(c) = keycode_to_char(code) {
                        t.push(c);
                    }
                    // 무한히 커지지 않도록 뒤쪽만 유지
                    if t.chars().count() > 200 {
                        let tail: String = t.chars().rev().take(100).collect::<Vec<_>>().into_iter().rev().collect();
                        *t = tail;
                    }
                });
            }
        }

        // 단축키(Cmd+Q, Cmd+W 등)를 삼켜서 아무 일도 일어나지 않게 함
        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, _event: &NSEvent) -> bool {
            true
        }
    }
);
//#endregion

//#region 화면 하나를 덮는 가림 창 + 라벨들
pub struct Overlay {
    pub window: Retained<OverlayWindow>,
    pub countdown: Retained<NSTextField>,
    pub tip: Retained<NSTextField>,
    pub footer: Retained<NSTextField>,
}

fn make_label(
    text: &str,
    font: &NSFont,
    color: &NSColor,
    frame: NSRect,
    mtm: MainThreadMarker,
) -> Retained<NSTextField> {
    let label = NSTextField::wrappingLabelWithString(&NSString::from_str(text), mtm);
    label.setFrame(frame);
    label.setAlignment(NSTextAlignment::Center);
    label.setFont(Some(font));
    label.setTextColor(Some(color));
    label.setDrawsBackground(false);
    label.setBezeled(false);
    label.setEditable(false);
    label.setSelectable(false);
    label
}

impl Overlay {
    pub fn new(screen: &NSScreen, mtm: MainThreadMarker) -> Self {
        let frame = screen.frame();

        //#region 창 생성 및 최상위 고정
        let this = OverlayWindow::alloc(mtm).set_ivars(());
        let window: Retained<OverlayWindow> = unsafe {
            msg_send![super(this),
                initWithContentRect: frame,
                styleMask: NSWindowStyleMask::Borderless,
                backing: NSBackingStoreType::Buffered,
                defer: false]
        };
        // SAFETY: 창을 Retained 로 직접 보관하므로 닫힐 때 자동 해제 금지
        unsafe { window.setReleasedWhenClosed(false) };
        window.setLevel(unsafe { CGShieldingWindowLevel() } as isize);
        window.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );
        window.setOpaque(true);
        window.setBackgroundColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(
            0.08, 0.09, 0.13, 1.0,
        )));
        window.setFrame_display(frame, true);
        //#endregion

        //#region 라벨 배치 (창 내부 좌표, 원점 좌하단)
        let w = frame.size.width;
        let h = frame.size.height;
        let white = NSColor::colorWithSRGBRed_green_blue_alpha(1.0, 1.0, 1.0, 1.0);
        let accent = NSColor::colorWithSRGBRed_green_blue_alpha(1.0, 0.42, 0.35, 1.0);
        let dim = NSColor::colorWithSRGBRed_green_blue_alpha(1.0, 1.0, 1.0, 0.45);
        let pad = 40.0;
        let rect = |y: f64, height: f64| {
            NSRect::new(NSPoint::new(pad, y), NSSize::new(w - pad * 2.0, height))
        };

        let title = make_label(
            "🧘  일어나서 스트레칭할 시간입니다",
            &NSFont::boldSystemFontOfSize((h * 0.05).clamp(28.0, 64.0)),
            &accent,
            rect(h * 0.68, h * 0.1),
            mtm,
        );
        let countdown = make_label(
            "--:--",
            &NSFont::monospacedDigitSystemFontOfSize_weight((h * 0.2).clamp(80.0, 260.0), 0.3),
            &white,
            rect(h * 0.38, h * 0.28),
            mtm,
        );
        let tip = make_label(
            "",
            &NSFont::systemFontOfSize((h * 0.032).clamp(20.0, 40.0)),
            &white,
            rect(h * 0.16, h * 0.18),
            mtm,
        );
        let footer = make_label(
            "",
            &NSFont::systemFontOfSize(14.0),
            &dim,
            rect(24.0, 40.0),
            mtm,
        );

        let view = window.contentView().expect("content view");
        view.addSubview(&title);
        view.addSubview(&countdown);
        view.addSubview(&tip);
        view.addSubview(&footer);
        //#endregion

        Self { window, countdown, tip, footer }
    }

    pub fn show(&self) {
        self.window.orderFrontRegardless();
        self.window.makeKeyWindow();
    }

    pub fn close(&self) {
        self.window.orderOut(None);
        self.window.close();
    }
}

/// 연결된 모든 화면마다 가림 창 생성
pub fn build_all(mtm: MainThreadMarker) -> Vec<Overlay> {
    NSScreen::screens(mtm)
        .iter()
        .map(|screen| Overlay::new(&screen, mtm))
        .collect()
}
//#endregion

//#region 스트레칭 동작 안내 문구
pub const TIPS: &[&str] = &[
    "자리에서 일어나 양팔을 머리 위로 쭉 뻗고 10초 버티세요",
    "목을 천천히 좌우로 기울여 각 15초씩 늘려주세요",
    "어깨를 귀까지 올렸다가 툭 떨어뜨리기 — 10회",
    "손바닥을 앞으로 뻗고 반대 손으로 손가락을 몸쪽으로 당기세요 (손목)",
    "양손을 허리에 대고 상체를 뒤로 천천히 젖혀주세요",
    "깍지를 등 뒤에서 끼고 가슴을 활짝 열어주세요",
    "한쪽 팔을 머리 위로 넘겨 옆구리를 늘려주세요 — 좌우 각 15초",
    "벽을 짚고 한 발을 뒤로 빼서 종아리를 늘려주세요",
    "창밖 6m 이상 먼 곳을 20초 이상 바라보세요 (눈 휴식)",
    "제자리 걷기 30초 — 혈액순환을 깨워주세요",
    "턱을 뒤로 당겨 이중턱 만들기 — 거북목 교정 10회",
    "의자 없이 스쿼트 10회!",
];
//#endregion
