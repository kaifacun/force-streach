use std::time::{Duration, SystemTime};

use super::config::Config;
use super::tips::TIPS;

//#region 상태·이벤트 정의
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    Working,
    Paused { until: SystemTime },
    Breaking { started: SystemTime, ends_at: SystemTime },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    None,
    /// 플랫폼은 가림을 띄우고 `start_break` 를 호출해야 함
    StartBreak,
    /// 플랫폼은 가림을 걷고 `end_break` 를 호출해야 함
    EndBreak,
}

/// 메뉴바/트레이에 표시할 상태
pub struct Status {
    /// 짧은 표시 (예: "🧘 23:41")
    pub title: String,
    /// 긴 설명 (예: "다음 스트레칭까지 23:41")
    pub info: String,
}

/// 가림 화면에 표시할 내용
pub struct BreakView {
    pub countdown: String,
    pub tip: &'static str,
}

/// 잠자기 등으로 틱 간격이 이보다 길면 이미 쉰 것으로 간주
const SLEEP_GAP_SECS: f64 = 30.0;
//#endregion

//#region 시간 유틸
pub fn secs_between(a: SystemTime, b: SystemTime) -> f64 {
    b.duration_since(a).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

pub fn fmt_mmss(secs: f64) -> String {
    let s = secs.max(0.0).ceil() as u64;
    format!("{:02}:{:02}", s / 60, s % 60)
}

pub fn fmt_dur_label(secs: u64) -> String {
    if secs % 60 == 0 { format!("{}분", secs / 60) } else { format!("{}초", secs) }
}
//#endregion

//#region 스케줄러 (OS 무관 순수 로직)
pub struct Scheduler {
    pub cfg: Config,
    pub phase: Phase,
    /// 현재 주기에서 실제로 작업한 누적 초
    pub work_elapsed: f64,
    last_tick: SystemTime,
    tip_offset: usize,
}

impl Scheduler {
    pub fn new(cfg: Config, now: SystemTime) -> Self {
        Self { cfg, phase: Phase::Working, work_elapsed: 0.0, last_tick: now, tip_offset: 0 }
    }

    /// 주기적으로 호출. `idle_secs` 는 마지막 키보드·마우스 입력 이후 경과 초.
    pub fn tick(&mut self, now: SystemTime, idle_secs: f64) -> Event {
        let dt = secs_between(self.last_tick, now);
        self.last_tick = now;

        match self.phase {
            Phase::Paused { until } => {
                if now >= until {
                    self.phase = Phase::Working;
                    self.work_elapsed = 0.0;
                }
                Event::None
            }
            Phase::Working => {
                let idle_limit = self.cfg.idle_reset_secs.max(self.cfg.break_secs) as f64;
                if dt > SLEEP_GAP_SECS {
                    // 잠자기에서 깨어남
                    self.work_elapsed = 0.0;
                } else if self.cfg.idle_reset && idle_secs >= idle_limit {
                    // 자리 비움
                    self.work_elapsed = 0.0;
                } else {
                    self.work_elapsed += dt;
                }
                if self.work_elapsed >= self.cfg.interval_secs as f64 {
                    Event::StartBreak
                } else {
                    Event::None
                }
            }
            Phase::Breaking { ends_at, .. } => {
                if now >= ends_at { Event::EndBreak } else { Event::None }
            }
        }
    }

    pub fn is_breaking(&self) -> bool {
        matches!(self.phase, Phase::Breaking { .. })
    }

    pub fn is_paused(&self) -> bool {
        matches!(self.phase, Phase::Paused { .. })
    }

    /// 스트레칭 시작. 종료 예정 시각 반환.
    pub fn start_break(&mut self, now: SystemTime) -> SystemTime {
        let ends_at = now + Duration::from_secs(self.cfg.break_secs);
        self.phase = Phase::Breaking { started: now, ends_at };
        self.tip_offset = now
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs() as usize)
            .unwrap_or(0)
            % TIPS.len();
        ends_at
    }

    pub fn end_break(&mut self) {
        self.phase = Phase::Working;
        self.work_elapsed = 0.0;
    }

    /// 일시정지(1시간) ↔ 재개
    pub fn toggle_pause(&mut self, now: SystemTime) {
        self.phase = match self.phase {
            Phase::Paused { .. } => {
                self.work_elapsed = 0.0;
                Phase::Working
            }
            Phase::Breaking { .. } => return,
            Phase::Working => Phase::Paused { until: now + Duration::from_secs(3600) },
        };
    }

    /// 설정 교체. 주기가 바뀌면 처음부터 다시 셈.
    pub fn set_config(&mut self, cfg: Config) {
        if cfg.interval_secs != self.cfg.interval_secs {
            self.work_elapsed = 0.0;
        }
        self.cfg = cfg;
    }

    pub fn status(&self, now: SystemTime) -> Status {
        match self.phase {
            Phase::Working => {
                let left = self.cfg.interval_secs as f64 - self.work_elapsed;
                let icon = if left <= self.cfg.warn_secs as f64 { "⚠️" } else { "🧘" };
                Status {
                    title: format!("{icon} {}", fmt_mmss(left)),
                    info: format!("다음 스트레칭까지 {}", fmt_mmss(left)),
                }
            }
            Phase::Paused { until } => {
                let left = secs_between(now, until);
                Status {
                    title: format!("⏸ {}", fmt_mmss(left)),
                    info: format!("일시정지 중 · {} 후 재개", fmt_mmss(left)),
                }
            }
            Phase::Breaking { ends_at, .. } => {
                let left = secs_between(now, ends_at);
                Status { title: format!("🤸 {}", fmt_mmss(left)), info: "스트레칭 중".to_string() }
            }
        }
    }

    pub fn break_view(&self, now: SystemTime) -> Option<BreakView> {
        let Phase::Breaking { started, ends_at } = self.phase else {
            return None;
        };
        let elapsed = secs_between(started, now);
        let idx = (self.tip_offset + (elapsed / 20.0) as usize) % TIPS.len();
        Some(BreakView { countdown: fmt_mmss(secs_between(now, ends_at)), tip: TIPS[idx] })
    }
}
//#endregion

//#region 단위 테스트
#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        Config {
            interval_secs: 600,
            break_secs: 60,
            warn_secs: 30,
            idle_reset: true,
            idle_reset_secs: 300,
            emergency_phrase: "x".into(),
        }
    }

    fn t0() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000)
    }

    /// 1초 간격으로 n번 틱. 마지막 이벤트 반환.
    fn run(s: &mut Scheduler, start: SystemTime, n: u64, idle: f64) -> (SystemTime, Event) {
        let mut ev = Event::None;
        let mut now = start;
        for _ in 0..n {
            now += Duration::from_secs(1);
            ev = s.tick(now, idle);
            if ev != Event::None {
                break;
            }
        }
        (now, ev)
    }

    #[test]
    fn starts_break_after_interval() {
        let mut s = Scheduler::new(cfg(), t0());
        let (_, ev) = run(&mut s, t0(), 599, 0.0);
        assert_eq!(ev, Event::None);
        let (_, ev) = run(&mut s, t0() + Duration::from_secs(599), 1, 0.0);
        assert_eq!(ev, Event::StartBreak);
    }

    #[test]
    fn break_ends_after_duration() {
        let mut s = Scheduler::new(cfg(), t0());
        let ends = s.start_break(t0());
        assert_eq!(ends, t0() + Duration::from_secs(60));
        assert_eq!(s.tick(t0() + Duration::from_secs(59), 0.0), Event::None);
        assert_eq!(s.tick(t0() + Duration::from_secs(60), 0.0), Event::EndBreak);
        s.end_break();
        assert!(!s.is_breaking());
        assert_eq!(s.work_elapsed, 0.0);
    }

    #[test]
    fn short_idle_does_not_reset() {
        // 스트레칭 시간(60초)만큼 입력이 없어도, 자리 비움 기준(300초) 미만이면 초기화 안 됨
        let mut s = Scheduler::new(cfg(), t0());
        run(&mut s, t0(), 100, 120.0);
        assert!(s.work_elapsed >= 99.0);
    }

    #[test]
    fn long_idle_resets() {
        let mut s = Scheduler::new(cfg(), t0());
        let (now, _) = run(&mut s, t0(), 100, 0.0);
        s.tick(now + Duration::from_secs(1), 300.0);
        assert_eq!(s.work_elapsed, 0.0);
    }

    #[test]
    fn idle_reset_disabled() {
        let mut c = cfg();
        c.idle_reset = false;
        let mut s = Scheduler::new(c, t0());
        run(&mut s, t0(), 100, 10_000.0);
        assert!(s.work_elapsed >= 99.0);
    }

    #[test]
    fn sleep_gap_resets() {
        let mut s = Scheduler::new(cfg(), t0());
        let (now, _) = run(&mut s, t0(), 100, 0.0);
        s.tick(now + Duration::from_secs(31), 0.0);
        assert_eq!(s.work_elapsed, 0.0);
    }

    #[test]
    fn pause_blocks_then_resumes() {
        let mut s = Scheduler::new(cfg(), t0());
        s.toggle_pause(t0());
        assert!(s.is_paused());
        let (_, ev) = run(&mut s, t0(), 1000, 0.0);
        assert_eq!(ev, Event::None);
        assert!(s.is_paused());
        s.tick(t0() + Duration::from_secs(3600), 0.0);
        assert!(!s.is_paused());
        assert_eq!(s.work_elapsed, 0.0);
    }

    #[test]
    fn toggle_pause_resumes_manually() {
        let mut s = Scheduler::new(cfg(), t0());
        s.toggle_pause(t0());
        s.toggle_pause(t0());
        assert_eq!(s.phase, Phase::Working);
    }

    #[test]
    fn interval_change_resets_elapsed() {
        let mut s = Scheduler::new(cfg(), t0());
        run(&mut s, t0(), 100, 0.0);
        let mut c = cfg();
        c.break_secs = 120;
        s.set_config(c.clone());
        assert!(s.work_elapsed > 0.0, "주기 외 변경은 누적 유지");
        c.interval_secs = 1200;
        s.set_config(c);
        assert_eq!(s.work_elapsed, 0.0);
    }

    #[test]
    fn status_shows_warning() {
        let mut s = Scheduler::new(cfg(), t0());
        s.work_elapsed = 580.0;
        assert!(s.status(t0()).title.starts_with("⚠️"));
        s.work_elapsed = 10.0;
        assert!(s.status(t0()).title.starts_with("🧘"));
    }

    #[test]
    fn break_view_countdown() {
        let mut s = Scheduler::new(cfg(), t0());
        assert!(s.break_view(t0()).is_none());
        s.start_break(t0());
        let v = s.break_view(t0() + Duration::from_secs(15)).unwrap();
        assert_eq!(v.countdown, "00:45");
    }

    #[test]
    fn fmt() {
        assert_eq!(fmt_mmss(0.2), "00:01");
        assert_eq!(fmt_mmss(-5.0), "00:00");
        assert_eq!(fmt_mmss(3000.0), "50:00");
        assert_eq!(fmt_dur_label(120), "2분");
        assert_eq!(fmt_dur_label(90), "90초");
    }
}
//#endregion
