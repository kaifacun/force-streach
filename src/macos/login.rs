//! 로그인 시 자동 실행 (install.sh 와 같은 LaunchAgent plist 를 쓰고 지움)
//!
//! - 켜기: plist 파일만 작성 (지금 bootstrap 하면 RunAtLoad 로 두 번째 인스턴스가 뜸). 다음 로그인부터 적용.
//! - 끄기: plist 파일만 삭제 (지금 bootout 하면 실행 중인 이 앱이 종료됨).
use std::path::PathBuf;

use crate::common::log;

const LABEL: &str = "local.force-stretch";

fn plist_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/LaunchAgents").join(format!("{LABEL}.plist")))
}

fn current_exe() -> Option<String> {
    Some(std::env::current_exe().ok()?.to_string_lossy().into_owned())
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// 등록돼 있고, 그 경로가 지금 실행 중인 앱을 가리키면 켜진 것으로 봄
pub fn is_enabled() -> bool {
    let (Some(path), Some(exe)) = (plist_path(), current_exe()) else { return false };
    std::fs::read_to_string(path).is_ok_and(|s| s.contains(&xml_escape(&exe)))
}

pub fn set_enabled(on: bool) {
    let Some(path) = plist_path() else { return };
    if !on {
        match std::fs::remove_file(&path) {
            Ok(()) => log::write("로그인 시 자동 실행 해제"),
            Err(e) => log::write(&format!("자동 실행 해제 실패: {e}")),
        }
        return;
    }
    let Some(exe) = current_exe() else { return };
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array><string>{}</string></array>
    <key>RunAtLoad</key><true/>
    <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
    <key>ProcessType</key><string>Interactive</string>
</dict>
</plist>
"#,
        xml_escape(&exe)
    );
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match std::fs::write(&path, plist) {
        Ok(()) => log::write(&format!("로그인 시 자동 실행 등록: {exe}")),
        Err(e) => log::write(&format!("자동 실행 등록 실패: {e}")),
    }
}
