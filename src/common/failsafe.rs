use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

//#region 안전장치: UI와 독립된 감시 스레드
/// 현재 스트레칭 종료 예정 시각 (기준 시점 이후 밀리초, 단조 시계). 0이면 스트레칭 중 아님.
static BREAK_DEADLINE: AtomicU64 = AtomicU64::new(0);
/// 예정 종료 시각을 이만큼 넘겨도 가림이 남아 있으면 프로세스를 강제 종료
const MARGIN: Duration = Duration::from_secs(30);

/// 단조 시계 기준점 (시계 변경에 영향받지 않음)
fn base() -> Instant {
    static BASE: OnceLock<Instant> = OnceLock::new();
    *BASE.get_or_init(Instant::now)
}

fn millis(t: Instant) -> u64 {
    // 0 은 "해제" 표시이므로 최소 1
    (t.saturating_duration_since(base()).as_millis() as u64).max(1)
}

pub fn spawn() {
    base();
    std::thread::spawn(|| loop {
        std::thread::sleep(Duration::from_secs(2));
        let deadline = BREAK_DEADLINE.load(Ordering::SeqCst);
        if deadline != 0 && millis(Instant::now()) > deadline + MARGIN.as_millis() as u64 {
            super::log::write("안전장치 작동: 가림 화면이 예정보다 오래 남아 프로세스를 종료합니다");
            std::process::exit(2);
        }
    });
}

/// 스트레칭 시작 시 종료 예정 시각 등록
pub fn arm(ends_at: Instant) {
    BREAK_DEADLINE.store(millis(ends_at), Ordering::SeqCst);
}

/// 스트레칭 종료 시 해제
pub fn disarm() {
    BREAK_DEADLINE.store(0, Ordering::SeqCst);
}
//#endregion
