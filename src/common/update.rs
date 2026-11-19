//! GitHub 릴리스 기반 업데이트 확인·다운로드 (OS 무관 부분)
//!
//! - 최신 버전은 `/releases/latest` 의 리디렉션 위치(`/releases/tag/vX.Y.Z`)에서 읽는다 (API 호출 제한 없음).
//! - 받은 파일은 `/releases/download/vX.Y.Z/<자산>` 에서만 받는다.
//! - 네트워크는 OS 기본 `curl` 로 처리해 TLS 의존성을 추가하지 않는다.
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::log;

//#region 상수
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
const REPO: &str = "https://github.com/kaifacun/force-streach";
pub const RELEASES_PAGE: &str = "https://github.com/kaifacun/force-streach/releases/latest";
/// 자동 확인 주기 (실패 시에도 이 간격으로 재시도)
const CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);
/// 시작 직후 네트워크가 준비될 시간
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(30);
//#endregion

//#region 버전 비교
pub fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.trim().trim_start_matches('v');
    let mut it = s.split('.').map(|p| p.parse::<u64>().ok());
    let v = (it.next()??, it.next()??, it.next()??);
    if it.next().is_some() { None } else { Some(v) }
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}
//#endregion

//#region 네트워크 (curl)
fn curl() -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let mut c = Command::new(format!(r"{root}\System32\curl.exe"));
        c.creation_flags(CREATE_NO_WINDOW);
        c
    }
    #[cfg(not(windows))]
    {
        Command::new("/usr/bin/curl")
    }
}

/// 최신 릴리스 버전 문자열 (예: "0.2.0")
pub fn fetch_latest() -> Result<String, String> {
    let out = curl()
        .args(["-fsSI", "--max-time", "20", RELEASES_PAGE])
        .output()
        .map_err(|e| format!("curl 실행 실패: {e}"))?;
    if !out.status.success() {
        return Err(format!("최신 버전 조회 실패 (curl {})", out.status));
    }
    let headers = String::from_utf8_lossy(&out.stdout);
    let location = headers
        .lines()
        .find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim().eq_ignore_ascii_case("location").then(|| v.trim().to_string())
        })
        .ok_or("리디렉션 위치 없음")?;
    let prefix = format!("{REPO}/releases/tag/v");
    let version = location.strip_prefix(&prefix).ok_or_else(|| format!("예상 밖 위치: {location}"))?;
    parse_version(version).ok_or_else(|| format!("버전 형식 오류: {version}"))?;
    Ok(version.to_string())
}

pub fn asset_url(version: &str, asset: &str) -> String {
    format!("{REPO}/releases/download/v{version}/{asset}")
}

/// 파일 다운로드 (리디렉션 따라감). 실패 시 받다 만 파일 삭제.
pub fn download(url: &str, dest: &Path) -> Result<(), String> {
    let status = curl()
        .args(["-fsSL", "--max-time", "300", "-o"])
        .arg(dest)
        .arg(url)
        .status()
        .map_err(|e| format!("curl 실행 실패: {e}"))?;
    if !status.success() {
        let _ = std::fs::remove_file(dest);
        return Err(format!("다운로드 실패 (curl {status}): {url}"));
    }
    Ok(())
}
//#endregion

//#region 업데이트 상태 (백그라운드 스레드 ↔ UI 스레드 공유)
#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Idle,
    Checking,
    UpToDate,
    Available(String),
    Installing(String),
    /// 교체 완료. UI 스레드가 새 버전을 띄우고 종료해야 함.
    Ready(String),
    Failed(String),
}

impl State {
    /// 메뉴에 표시할 문구
    pub fn menu_title(&self) -> String {
        match self {
            State::Idle | State::Failed(_) => format!("업데이트 확인 (현재 v{CURRENT})"),
            State::Checking => "업데이트 확인 중…".into(),
            State::UpToDate => format!("최신 버전입니다 (v{CURRENT})"),
            State::Available(v) => format!("새 버전 v{v} 설치"),
            State::Installing(v) => format!("v{v} 설치 중…"),
            State::Ready(v) => format!("v{v}(으)로 다시 시작하는 중…"),
        }
    }

    /// 메뉴 항목을 누를 수 있는지
    pub fn clickable(&self) -> bool {
        !matches!(self, State::Checking | State::Installing(_) | State::Ready(_))
    }
}

pub struct Updater {
    state: Arc<Mutex<State>>,
    next_check: Instant,
}

impl Updater {
    pub fn new() -> Self {
        Self { state: Arc::new(Mutex::new(State::Idle)), next_check: Instant::now() + FIRST_CHECK_DELAY }
    }

    pub fn state(&self) -> State {
        self.state.lock().map(|s| s.clone()).unwrap_or(State::Idle)
    }

    fn set(state: &Arc<Mutex<State>>, s: State) {
        if let Ok(mut g) = state.lock() {
            *g = s;
        }
    }

    /// 틱마다 호출: 때가 되면 백그라운드에서 확인
    pub fn tick(&mut self) {
        if Instant::now() >= self.next_check {
            match self.state() {
                State::Idle | State::UpToDate | State::Failed(_) => self.check(),
                // 새 버전 알림 중이거나 설치 중이면 다시 묻지 않음
                _ => self.next_check = Instant::now() + CHECK_EVERY,
            }
        }
    }

    /// 지금 확인 (메뉴에서 호출 가능)
    pub fn check(&mut self) {
        if !self.state().clickable() {
            return;
        }
        Self::set(&self.state, State::Checking);
        self.next_check = Instant::now() + CHECK_EVERY;
        let state = Arc::clone(&self.state);
        std::thread::spawn(move || {
            let s = match fetch_latest() {
                Ok(v) if is_newer(&v, CURRENT) => {
                    log::write(&format!("업데이트 있음: v{CURRENT} → v{v}"));
                    State::Available(v)
                }
                Ok(_) => State::UpToDate,
                Err(e) => {
                    log::write(&format!("업데이트 확인 실패: {e}"));
                    State::Failed(e)
                }
            };
            Self::set(&state, s);
        });
    }

    /// OS별 설치 함수를 백그라운드에서 실행. 성공하면 Ready, 실패하면 Failed.
    pub fn install<F>(&self, install: F)
    where
        F: FnOnce(&str) -> Result<(), String> + Send + 'static,
    {
        let State::Available(version) = self.state() else { return };
        Self::set(&self.state, State::Installing(version.clone()));
        let state = Arc::clone(&self.state);
        std::thread::spawn(move || {
            log::write(&format!("업데이트 설치 시작: v{version}"));
            let s = match install(&version) {
                Ok(()) => {
                    log::write(&format!("업데이트 교체 완료: v{version}"));
                    State::Ready(version)
                }
                Err(e) => {
                    log::write(&format!("업데이트 설치 실패: {e}"));
                    State::Failed(e)
                }
            };
            Self::set(&state, s);
        });
    }
}
//#endregion

//#region 명령줄 업데이트 (--update-now)
/// 확인 → 설치 → 다시 시작을 메뉴 없이 한 번에. 종료 코드 반환 (0 = 최신이거나 업데이트 완료).
pub fn run_cli(
    install: impl FnOnce(&str) -> Result<(), String>,
    relaunch: impl FnOnce() -> Result<(), String>,
) -> i32 {
    let latest = match fetch_latest() {
        Ok(v) => v,
        Err(e) => {
            log::write(&format!("업데이트 확인 실패: {e}"));
            return 1;
        }
    };
    if !is_newer(&latest, CURRENT) {
        log::write(&format!("최신 버전입니다 (현재 v{CURRENT}, 릴리스 v{latest})"));
        return 0;
    }
    log::write(&format!("업데이트 설치: v{CURRENT} → v{latest}"));
    if let Err(e) = install(&latest) {
        log::write(&format!("업데이트 설치 실패: {e}"));
        return 1;
    }
    match relaunch() {
        Ok(()) => {
            log::write(&format!("v{latest} 설치 완료, 다시 시작합니다"));
            0
        }
        Err(e) => {
            log::write(&format!("v{latest} 설치 완료, 다시 시작 실패: {e}"));
            1
        }
    }
}
//#endregion

//#region 단위 테스트
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(parse_version("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("0.10.0"), Some((0, 10, 0)));
        assert_eq!(parse_version("1.2"), None);
        assert_eq!(parse_version("1.2.3.4"), None);
        assert_eq!(parse_version("1.2.x"), None);
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(is_newer("0.10.0", "0.9.0"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.0.9", "0.1.0"));
        assert!(!is_newer("garbage", "0.1.0"));
    }

    #[test]
    fn asset_url_shape() {
        assert_eq!(
            asset_url("0.2.0", "force-stretch.exe"),
            "https://github.com/kaifacun/force-streach/releases/download/v0.2.0/force-stretch.exe"
        );
    }
}
//#endregion
