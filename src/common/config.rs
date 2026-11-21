use serde::{Deserialize, Serialize};
use std::path::PathBuf;

//#region 설정 구조체
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// 작업 주기 (초). 이 시간 동안 일하면 스트레칭이 시작된다.
    pub interval_secs: u64,
    /// 스트레칭(화면 가림) 시간 (초)
    pub break_secs: u64,
    /// 스트레칭 시작 전 메뉴바 경고 표시 시간 (초)
    pub warn_secs: u64,
    /// 자리 비움(입력 없음)이 idle_reset_secs 이상이면 주기를 초기화할지 여부
    pub idle_reset: bool,
    /// 이 시간(초) 이상 입력이 없으면 자리 비움으로 판단. 스트레칭 시간보다 짧으면 스트레칭 시간을 사용.
    pub idle_reset_secs: u64,
    /// 비상 해제 문구. 화면 가림 중 이 문구를 정확히 타이핑하면 해제된다. 빈 문자열이면 비활성.
    pub emergency_phrase: String,
    /// 작업 중 가끔(10~20분 간격, 무작위) 앉은 자세를 2초간 전체 화면으로 귀띔할지 여부
    pub posture_nudge: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            interval_secs: 50 * 60,
            break_secs: 3 * 60,
            warn_secs: 30,
            idle_reset: true,
            idle_reset_secs: 300,
            emergency_phrase: "i promise i will stretch later".to_string(),
            posture_nudge: true,
        }
    }
}
//#endregion

//#region 설정 파일 입출력
/// 설정·로그 파일을 두는 폴더 (OS별)
pub fn data_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        PathBuf::from(home).join("Library/Application Support/force-stretch")
    }
    #[cfg(windows)]
    {
        let base = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
        PathBuf::from(base).join("force-stretch")
    }
}

pub fn config_path() -> PathBuf {
    data_dir().join("config.toml")
}

impl Config {
    pub fn load() -> Self {
        let path = config_path();
        let cfg = match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str::<Config>(&text).unwrap_or_else(|e| {
                eprintln!("[force-stretch] 설정 파싱 실패, 기본값 사용: {e}");
                Config::default()
            }),
            Err(_) => {
                let cfg = Config::default();
                cfg.save();
                cfg
            }
        };
        cfg.sanitized()
    }

    pub fn save(&self) {
        let path = config_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match toml::to_string_pretty(self) {
            Ok(text) => {
                if let Err(e) = std::fs::write(&path, text) {
                    eprintln!("[force-stretch] 설정 저장 실패: {e}");
                }
            }
            Err(e) => eprintln!("[force-stretch] 설정 직렬화 실패: {e}"),
        }
    }

    /// 비정상 값 보정 (0초 주기 등으로 화면이 계속 가려지는 사고 방지)
    /// 시험 모드: 5초 뒤 8초간 가림, 설정 파일은 저장하지 않음
    pub fn test_mode() -> Self {
        Config {
            interval_secs: 5,
            break_secs: 8,
            warn_secs: 3,
            idle_reset: false,
            ..Config::load()
        }
    }

    fn sanitized(mut self) -> Self {
        self.interval_secs = self.interval_secs.clamp(60, 24 * 3600);
        self.break_secs = self.break_secs.clamp(10, 30 * 60);
        self.warn_secs = self.warn_secs.min(self.interval_secs / 2);
        self.idle_reset_secs = self.idle_reset_secs.clamp(120, 3600);
        self.emergency_phrase = self.emergency_phrase.to_lowercase();
        self
    }
}
//#endregion
