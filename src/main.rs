// Windows 에서 콘솔 창이 뜨지 않도록 GUI 서브시스템 사용
#![cfg_attr(windows, windows_subsystem = "windows")]

mod common;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod win;

//#region 진입점: OS별 앱 실행
fn main() {
    let test_mode = std::env::args().any(|a| a == "--test");
    let cfg = if test_mode {
        common::config::Config::test_mode()
    } else {
        common::config::Config::load()
    };
    common::failsafe::spawn();

    #[cfg(target_os = "macos")]
    macos::run(cfg, test_mode);
    #[cfg(windows)]
    win::run(cfg, test_mode);
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = (cfg, test_mode);
        eprintln!("지원하지 않는 운영체제입니다 (macOS / Windows 전용)");
    }
}
//#endregion
