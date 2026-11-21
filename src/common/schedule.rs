use std::ops::{Add, AddAssign};
use std::time::{Duration, Instant, SystemTime};

use super::config::Config;
use super::tips::{POSTURE_TIPS, PostureTip, TIPS};

//#region 현재 시각 (벽시계 + 단조 시계)
/// 벽시계(`wall`)는 시계 변경·동기화로 앞뒤로 뛸 수 있으므로
/// 경과 시간은 단조 시계(`mono`)로 재고, 벽시계는 잠자기 감지·일시정지·표시용으로만 쓴다.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Now {
    pub wall: SystemTime,
    pub mono: Instant,
}

impl Now {
    pub fn now() -> Self {
        Self { wall: SystemTime::now(), mono: Instant::now() }
    }
}

impl Add<Duration> for Now {
    type Output = Now;
    fn add(self, d: Duration) -> Now {
        Now { wall: self.wall + d, mono: self.mono + d }
    }
}

impl AddAssign<Duration> for Now {
    fn add_assign(&mut self, d: Duration) {
        *self = *self + d;
    }
}
//#endregion

//#region 상태·이벤트 정의
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    Working,
    Paused { until: SystemTime },
    /// 스트레칭 시간은 단조 시계로 잼 (시계가 뛰어도 일찍 끝나지 않음)
    Breaking { started: Instant, ends_at: Instant },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    None,
    /// 플랫폼은 가림을 띄우고 `start_break` 를 호출해야 함
    StartBreak,
    /// 플랫폼은 가림을 걷고 `end_break` 를 호출해야 함
    EndBreak,
    /// 플랫폼은 `posture_tip()` 을 2초간 전체 화면으로 띄워야 함 (입력·초점은 빼앗지 않음)
    Nudge,
}

/// 메뉴바/트레이에 표시할 상태
pub struct Status {
    /// 짧은 표시 (예: "🧘 23:41"). Windows 트레이는 글자를 띄울 수 없어 미사용.
    #[cfg_attr(windows, allow(dead_code))]
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

/// 자세 귀띔 간격 (작업 누적 초, 이 범위에서 무작위)
pub const NUDGE_GAP_MIN_SECS: f64 = 10.0 * 60.0;
pub const NUDGE_GAP_MAX_SECS: f64 = 20.0 * 60.0;
/// 자세 귀띔 표시 시간
pub const NUDGE_SHOW: Duration = Duration::from_secs(2);
/// 스트레칭 직전(경고 시간 + 이 여유) 에는 귀띔하지 않음
const NUDGE_QUIET_BEFORE_BREAK_SECS: f64 = 60.0;
/// 이 시간 이상 입력이 없으면 자리에 없다고 보고 귀띔을 미룸
const NUDGE_ACTIVE_IDLE_SECS: f64 = 30.0;
//#endregion

//#region 시간 유틸
pub fn secs_between(a: SystemTime, b: SystemTime) -> f64 {
    b.duration_since(a).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

pub fn mono_secs_between(a: Instant, b: Instant) -> f64 {
    b.saturating_duration_since(a).as_secs_f64()
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
    last_tick: Now,
    tip_offset: usize,
    /// 작업 누적이 이 값에 이르면 자세 귀띔
    next_nudge_at: f64,
    nudge_idx: usize,
    rng: u64,
}

impl Scheduler {
    pub fn new(cfg: Config, now: Now) -> Self {
        let seed = now
            .wall
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9_7F4A_7C15);
        let mut s = Self {
            cfg,
            phase: Phase::Working,
            work_elapsed: 0.0,
            last_tick: now,
            tip_offset: 0,
            next_nudge_at: 0.0,
            nudge_idx: 0,
            rng: seed | 1,
        };
        s.nudge_idx = s.rand_below(POSTURE_TIPS.len());
        s.reset_work();
        s
    }

    //#region 자세 귀띔용 난수 (외부 crate 없이 xorshift)
    fn rand_u64(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x
    }

    fn rand_below(&mut self, n: usize) -> usize {
        (self.rand_u64() % n.max(1) as u64) as usize
    }

    fn roll_nudge_gap(&mut self) -> f64 {
        let t = (self.rand_u64() % 10_000) as f64 / 10_000.0;
        NUDGE_GAP_MIN_SECS + (NUDGE_GAP_MAX_SECS - NUDGE_GAP_MIN_SECS) * t
    }
    //#endregion

    /// 작업 누적을 처음부터 다시 셈 (다음 자세 귀띔 시점도 새로 뽑음)
    fn reset_work(&mut self) {
        self.work_elapsed = 0.0;
        self.next_nudge_at = self.roll_nudge_gap();
    }

    /// 주기적으로 호출. `idle_secs` 는 마지막 키보드·마우스 입력 이후 경과 초.
    pub fn tick(&mut self, now: Now, idle_secs: f64) -> Event {
        let wall_dt = secs_between(self.last_tick.wall, now.wall);
        let dt = mono_secs_between(self.last_tick.mono, now.mono);
        self.last_tick = now;

        match self.phase {
            Phase::Paused { until } => {
                if now.wall >= until {
                    self.phase = Phase::Working;
                    self.reset_work();
                }
                Event::None
            }
            Phase::Working => {
                let idle_limit = self.cfg.idle_reset_secs.max(self.cfg.break_secs) as f64;
                // macOS 단조 시계는 잠자기 동안 멈추므로 벽시계 간격도 함께 봄
                if dt.max(wall_dt) > SLEEP_GAP_SECS {
                    // 잠자기에서 깨어남
                    self.reset_work();
                } else if self.cfg.idle_reset && idle_secs >= idle_limit {
                    // 자리 비움
                    self.reset_work();
                } else {
                    self.work_elapsed += dt;
                }
                if self.work_elapsed >= self.cfg.interval_secs as f64 {
                    Event::StartBreak
                } else if self.nudge_due(idle_secs) {
                    self.nudge_idx = (self.nudge_idx + 1 + self.rand_below(POSTURE_TIPS.len() - 1))
                        % POSTURE_TIPS.len();
                    self.next_nudge_at = self.work_elapsed + self.roll_nudge_gap();
                    Event::Nudge
                } else {
                    Event::None
                }
            }
            Phase::Breaking { ends_at, .. } => {
                if now.mono >= ends_at { Event::EndBreak } else { Event::None }
            }
        }
    }

    //#region 자세 귀띔
    /// 켜져 있고, 시점이 됐고, 사용자가 자리에 있고, 스트레칭 직전이 아니면 귀띔
    fn nudge_due(&self, idle_secs: f64) -> bool {
        let left = self.cfg.interval_secs as f64 - self.work_elapsed;
        self.cfg.posture_nudge
            && self.work_elapsed >= self.next_nudge_at
            && idle_secs < NUDGE_ACTIVE_IDLE_SECS
            && left > self.cfg.warn_secs as f64 + NUDGE_QUIET_BEFORE_BREAK_SECS
    }

    /// 마지막 `Event::Nudge` 때 고른 자세 문구
    pub fn posture_tip(&self) -> &'static PostureTip {
        &POSTURE_TIPS[self.nudge_idx % POSTURE_TIPS.len()]
    }
    //#endregion

    pub fn is_breaking(&self) -> bool {
        matches!(self.phase, Phase::Breaking { .. })
    }

    pub fn is_paused(&self) -> bool {
        matches!(self.phase, Phase::Paused { .. })
    }

    /// 스트레칭 시작. 종료 예정 시각(단조 시계) 반환.
    pub fn start_break(&mut self, now: Now) -> Instant {
        let ends_at = now.mono + Duration::from_secs(self.cfg.break_secs);
        self.phase = Phase::Breaking { started: now.mono, ends_at };
        self.tip_offset = now
            .wall
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs() as usize)
            .unwrap_or(0)
            % TIPS.len();
        ends_at
    }

    pub fn end_break(&mut self) {
        self.phase = Phase::Working;
        self.reset_work();
    }

    /// 일시정지(1시간) ↔ 재개
    pub fn toggle_pause(&mut self, now: Now) {
        self.phase = match self.phase {
            Phase::Paused { .. } => {
                self.reset_work();
                Phase::Working
            }
            Phase::Breaking { .. } => return,
            Phase::Working => Phase::Paused { until: now.wall + Duration::from_secs(3600) },
        };
    }

    /// 설정 교체. 주기가 바뀌면 처음부터 다시 셈.
    pub fn set_config(&mut self, cfg: Config) {
        let reset = cfg.interval_secs != self.cfg.interval_secs;
        self.cfg = cfg;
        if reset {
            self.reset_work();
        }
    }

    pub fn status(&self, now: Now) -> Status {
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
                let left = secs_between(now.wall, until);
                Status {
                    title: format!("⏸ {}", fmt_mmss(left)),
                    info: format!("일시정지 중 · {} 후 재개", fmt_mmss(left)),
                }
            }
            Phase::Breaking { ends_at, .. } => {
                let left = mono_secs_between(now.mono, ends_at);
                Status { title: format!("🤸 {}", fmt_mmss(left)), info: "스트레칭 중".to_string() }
            }
        }
    }

    pub fn break_view(&self, now: Now) -> Option<BreakView> {
        let Phase::Breaking { started, ends_at } = self.phase else {
            return None;
        };
        let elapsed = mono_secs_between(started, now.mono);
        let idx = (self.tip_offset + (elapsed / 20.0) as usize) % TIPS.len();
        Some(BreakView { countdown: fmt_mmss(mono_secs_between(now.mono, ends_at)), tip: TIPS[idx] })
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
            posture_nudge: true,
        }
    }

    /// 긴 주기 설정 (자세 귀띔 시험용)
    fn long_cfg() -> Config {
        Config { interval_secs: 3600, warn_secs: 30, ..cfg() }
    }

    /// 1초 간격으로 n번 틱하며 귀띔 시점(작업 누적 초) 기록
    fn nudge_times(s: &mut Scheduler, n: u64, idle: f64) -> Vec<f64> {
        let mut now = t0();
        let mut out = Vec::new();
        for _ in 0..n {
            now += Duration::from_secs(1);
            match s.tick(now, idle) {
                Event::Nudge => out.push(s.work_elapsed),
                Event::StartBreak => break,
                _ => {}
            }
        }
        out
    }

    #[test]
    fn nudge_gaps_within_bounds() {
        let mut s = Scheduler::new(long_cfg(), t0());
        let times = nudge_times(&mut s, 3600, 0.0);
        assert!(times.len() >= 2, "1시간에 두 번 이상: {times:?}");
        let mut prev = 0.0;
        for t in &times {
            let gap = t - prev;
            assert!((NUDGE_GAP_MIN_SECS - 1.0..=NUDGE_GAP_MAX_SECS + 1.0).contains(&gap), "gap {gap}");
            prev = *t;
        }
        // 스트레칭 직전(경고 30초 + 60초)에는 귀띔 없음
        assert!(times.iter().all(|t| *t < 3600.0 - 90.0));
    }

    #[test]
    fn nudge_disabled() {
        let mut c = long_cfg();
        c.posture_nudge = false;
        let mut s = Scheduler::new(c, t0());
        assert!(nudge_times(&mut s, 3600, 0.0).is_empty());
    }

    #[test]
    fn nudge_waits_while_away() {
        // 입력이 없으면(자리 비움 초기화 기준 미만이라도) 귀띔하지 않음
        let mut c = long_cfg();
        c.idle_reset = false;
        let mut s = Scheduler::new(c, t0());
        assert!(nudge_times(&mut s, 3600, 60.0).is_empty());
    }

    #[test]
    fn nudge_never_while_paused_or_breaking() {
        let mut s = Scheduler::new(long_cfg(), t0());
        s.toggle_pause(t0());
        assert!(nudge_times(&mut s, 3000, 0.0).is_empty());
        let mut s = Scheduler::new(long_cfg(), t0());
        s.work_elapsed = 1500.0;
        s.start_break(t0());
        assert!(nudge_times(&mut s, 50, 0.0).is_empty());
    }

    #[test]
    fn nudge_tip_changes() {
        let mut s = Scheduler::new(long_cfg(), t0());
        let before = s.posture_tip().title;
        s.work_elapsed = s.next_nudge_at;
        assert_eq!(s.tick(t0() + Duration::from_millis(500), 0.0), Event::Nudge);
        assert_ne!(s.posture_tip().title, before);
    }

    fn t0() -> Now {
        static BASE: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        Now {
            wall: SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000),
            mono: *BASE.get_or_init(Instant::now),
        }
    }

    /// 1초 간격으로 n번 틱. 마지막 이벤트 반환.
    fn run(s: &mut Scheduler, start: Now, n: u64, idle: f64) -> (Now, Event) {
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
        assert_eq!(ends, (t0() + Duration::from_secs(60)).mono);
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
    fn wall_clock_jump_does_not_end_break_early() {
        // 시간 동기화로 벽시계만 10분 앞으로 뛰어도 스트레칭은 단조 시계 기준으로 유지
        let mut s = Scheduler::new(cfg(), t0());
        s.start_break(t0());
        let jumped = Now { wall: t0().wall + Duration::from_secs(600), mono: t0().mono + Duration::from_secs(1) };
        assert_eq!(s.tick(jumped, 0.0), Event::None);
        assert!(s.is_breaking());
        assert_eq!(s.break_view(jumped).unwrap().countdown, "00:59");
    }

    #[test]
    fn wall_clock_jump_back_keeps_counting() {
        // 벽시계가 뒤로 가도 작업 시간은 계속 누적
        let mut s = Scheduler::new(cfg(), t0());
        let back = Now { wall: t0().wall - Duration::from_secs(3600), mono: t0().mono + Duration::from_secs(1) };
        s.tick(back, 0.0);
        assert!((s.work_elapsed - 1.0).abs() < 1e-6);
    }

    #[test]
    fn mac_style_sleep_resets() {
        // macOS: 잠자기 동안 단조 시계는 멈추고 벽시계만 흐름
        let mut s = Scheduler::new(cfg(), t0());
        let (now, _) = run(&mut s, t0(), 100, 0.0);
        let woke = Now { wall: now.wall + Duration::from_secs(3600), mono: now.mono + Duration::from_secs(1) };
        s.tick(woke, 0.0);
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
