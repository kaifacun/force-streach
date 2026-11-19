//! macOS 업데이트 설치: 릴리스 zip 을 받아 앱 번들을 이름 바꾸기로 교체
//!
//! curl 로 받은 파일에는 격리(quarantine) 속성이 붙지 않으므로, 교체된 앱은 Gatekeeper 경고 없이 실행된다.
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::common::update;

const ASSET: &str = "Force-Stretch-macos.zip";
const BUNDLE_ID: &str = "local.force-stretch";
const APP_NAME: &str = "Force Stretch.app";

//#region 현재 앱 번들 위치 확인 (안전하게 교체할 수 없으면 거부)
pub fn app_bundle() -> Result<PathBuf, String> {
    if !cfg!(target_arch = "aarch64") {
        return Err("릴리스 빌드는 Apple Silicon 전용이라 이 Mac 에는 자동 설치할 수 없습니다".into());
    }
    let exe = std::env::current_exe().map_err(|e| format!("실행 파일 경로 확인 실패: {e}"))?;
    let bundle = exe
        .parent() // MacOS
        .and_then(Path::parent) // Contents
        .and_then(Path::parent) // X.app
        .filter(|p| p.extension().is_some_and(|e| e == "app"))
        .ok_or("앱 번들(.app) 안에서 실행 중이 아닙니다")?
        .to_path_buf();
    if bundle.to_string_lossy().contains("/AppTranslocation/") {
        return Err("앱이 임시 위치에서 실행 중입니다. 응용 프로그램 폴더로 옮긴 뒤 다시 실행하세요".into());
    }
    let parent = bundle.parent().ok_or("앱 폴더 확인 실패")?;
    let probe = parent.join(".force-stretch-write-test");
    std::fs::write(&probe, b"").map_err(|_| format!("{} 에 쓸 권한이 없습니다", parent.display()))?;
    let _ = std::fs::remove_file(&probe);
    Ok(bundle)
}
//#endregion

fn plist_value(app: &Path, key: &str) -> Option<String> {
    let out = Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", &format!("Print :{key}")])
        .arg(app.join("Contents/Info.plist"))
        .output()
        .ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn run(cmd: &mut Command, what: &str) -> Result<(), String> {
    let st = cmd.status().map_err(|e| format!("{what} 실행 실패: {e}"))?;
    if st.success() { Ok(()) } else { Err(format!("{what} 실패 ({st})")) }
}

//#region 설치 (백그라운드 스레드에서 실행)
pub fn install(version: &str) -> Result<(), String> {
    let bundle = app_bundle()?;
    let parent = bundle.parent().ok_or("앱 폴더 확인 실패")?.to_path_buf();
    let work = std::env::temp_dir().join(format!("force-stretch-update-{version}"));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| format!("임시 폴더 생성 실패: {e}"))?;
    let result = install_in(version, &bundle, &parent, &work);
    let _ = std::fs::remove_dir_all(&work);
    result
}

fn install_in(version: &str, bundle: &Path, parent: &Path, work: &Path) -> Result<(), String> {
    //#region 다운로드 + 압축 해제 + 검증
    let zip = work.join(ASSET);
    update::download(&update::asset_url(version, ASSET), &zip)?;
    let unpacked = work.join("unpacked");
    run(Command::new("/usr/bin/ditto").args(["-x", "-k"]).arg(&zip).arg(&unpacked), "압축 해제")?;
    let new_app = unpacked.join(APP_NAME);
    if !new_app.join("Contents/MacOS/force-stretch").is_file() {
        return Err("받은 앱에 실행 파일이 없습니다".into());
    }
    if plist_value(&new_app, "CFBundleIdentifier").as_deref() != Some(BUNDLE_ID) {
        return Err("받은 앱의 번들 ID 가 다릅니다".into());
    }
    if plist_value(&new_app, "CFBundleShortVersionString").as_deref() != Some(version) {
        return Err("받은 앱의 버전이 릴리스 태그와 다릅니다".into());
    }
    //#endregion

    //#region 교체: 같은 폴더에 준비 → 기존을 백업으로 이름 변경 → 새것을 제자리로 (실패 시 되돌림)
    let staged = parent.join(".Force Stretch.app.new");
    let backup = parent.join(".Force Stretch.app.old");
    let _ = std::fs::remove_dir_all(&staged);
    let _ = std::fs::remove_dir_all(&backup);
    run(Command::new("/usr/bin/ditto").arg(&new_app).arg(&staged), "새 앱 복사")?;
    if let Err(e) = std::fs::rename(bundle, &backup) {
        let _ = std::fs::remove_dir_all(&staged);
        return Err(format!("기존 앱 이동 실패: {e}"));
    }
    if let Err(e) = std::fs::rename(&staged, bundle) {
        let _ = std::fs::rename(&backup, bundle);
        let _ = std::fs::remove_dir_all(&staged);
        return Err(format!("새 앱 배치 실패: {e}"));
    }
    // 실행 중인 바이너리는 이미 메모리에 올라와 있어 지워도 무방
    let _ = std::fs::remove_dir_all(&backup);
    //#endregion
    Ok(())
}
//#endregion

/// 이 프로세스가 끝나기를 기다렸다가 새 앱을 여는 도우미 실행.
/// LaunchAgent 로 실행 중이면 launchd 가 종료 시 같은 프로세스 그룹을 정리하므로 새 그룹으로 분리한다.
pub fn relaunch(bundle: &Path) -> Result<(), String> {
    Command::new("/bin/sh")
        .args(["-c", "while kill -0 \"$1\" 2>/dev/null; do sleep 0.2; done; /usr/bin/open -n \"$2\"", "sh"])
        .arg(std::process::id().to_string())
        .arg(bundle)
        .process_group(0)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("다시 시작 도우미 실행 실패: {e}"))
}
