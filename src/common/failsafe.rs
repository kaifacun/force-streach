use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

//#region 안전장치: UI와 독립된 감시 스레드
/// 현재 스트레칭 종료 예정 시각 (유닉스 초). 0이면 스트레칭 중 아님.
static BREAK_DEADLINE: AtomicU64 = AtomicU64::new(0);
/// 예정 종료 시각을 이만큼 넘겨도 가림이 남아 있으면 프로세스를 강제 종료
const MARGIN_SECS: u64 = 30;

fn unix(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn spawn() {
    std::thread::spawn(|| loop {
        std::thread::sleep(Duration::from_secs(2));
        let deadline = BREAK_DEADLINE.load(Ordering::SeqCst);
        if deadline != 0 && unix(SystemTime::now()) > deadline + MARGIN_SECS {
            super::log::write("안전장치 작동: 가림 화면이 예정보다 오래 남아 프로세스를 종료합니다");
            std::process::exit(2);
        }
    });
}

/// 스트레칭 시작 시 종료 예정 시각 등록
pub fn arm(ends_at: SystemTime) {
    BREAK_DEADLINE.store(unix(ends_at), Ordering::SeqCst);
}

/// 스트레칭 종료 시 해제
pub fn disarm() {
    BREAK_DEADLINE.store(0, Ordering::SeqCst);
}
//#endregion
