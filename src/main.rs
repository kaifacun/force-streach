// Windows 에서 콘솔 창이 뜨지 않도록 GUI 서브시스템 사용
#![cfg_attr(windows, windows_subsystem = "windows")]

mod common;
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(windows)]
mod win;

//#region 진입점: OS별 앱 실행
fn main() {
    if std::env::args().any(|a| a == "--update-now") {
        #[cfg(target_os = "macos")]
        std::process::exit(common::update::run_cli(macos::update::install, || {
            macos::update::relaunch(&macos::update::app_bundle()?)
        }));
        #[cfg(windows)]
        std::process::exit(common::update::run_cli(win::update::install, win::update::relaunch));
    }
    // --test-nudge: 자세 귀띔을 한 번 띄우고 종료 (설정 저장·업데이트 확인·중복 실행 검사 없음)
    let nudge_demo = std::env::args().any(|a| a == "--test-nudge");
    let break_test = std::env::args().any(|a| a == "--test");
    let test_mode = break_test || nudge_demo;
    let cfg = if break_test {
        common::config::Config::test_mode()
    } else {
        common::config::Config::load()
    };
    // 중복 실행 방지 (업데이트 직후에는 이전 프로세스가 끝날 때까지 잠시 기다림)
    let _instance_lock = if test_mode {
        None
    } else {
        match common::instance::acquire(std::time::Duration::from_secs(10)) {
            Some(f) => Some(f),
            None => {
                common::log::write("이미 실행 중이므로 종료합니다");
                return;
            }
        }
    };
    common::failsafe::spawn();

    #[cfg(target_os = "macos")]
    macos::run(cfg, test_mode, nudge_demo);
    #[cfg(windows)]
    win::run(cfg, test_mode, nudge_demo);
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = (cfg, test_mode, nudge_demo);
        eprintln!("지원하지 않는 운영체제입니다 (macOS / Windows 전용)");
    }
}
//#endregion
