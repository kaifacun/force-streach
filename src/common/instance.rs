//! 중복 실행 방지 (파일 잠금, 프로세스가 끝나면 OS 가 자동 해제)
use std::fs::{File, OpenOptions};
use std::time::{Duration, Instant};

use super::config;

/// 잠금을 얻을 때까지 최대 `wait` 동안 재시도.
/// 업데이트 직후 새 버전이 뜰 때 이전 프로세스가 아직 종료 중일 수 있어서 기다린다.
/// 반환된 파일을 프로세스가 끝날 때까지 들고 있어야 잠금이 유지된다.
pub fn acquire(wait: Duration) -> Option<File> {
    let dir = config::data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("force-stretch.lock"))
        .ok()?;
    let deadline = Instant::now() + wait;
    loop {
        if file.try_lock().is_ok() {
            return Some(file);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}
