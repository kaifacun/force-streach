//! Windows 업데이트 설치: 실행 중인 exe 를 .old 로 비켜두고 새 exe 로 교체
use std::path::PathBuf;
use std::process::Command;

use crate::common::{log, update};

const ASSET: &str = "force-stretch.exe";

fn current_exe() -> Result<PathBuf, String> {
    std::env::current_exe().map_err(|e| format!("실행 파일 경로 확인 실패: {e}"))
}

fn old_path(exe: &std::path::Path) -> PathBuf {
    exe.with_extension("exe.old")
}

/// 지난 업데이트에서 남은 .old 파일 삭제 (시작 시 호출)
pub fn cleanup_old() {
    if let Ok(exe) = current_exe() {
        let old = old_path(&exe);
        if old.exists() {
            match std::fs::remove_file(&old) {
                Ok(()) => log::write("이전 버전 파일 정리 완료"),
                Err(e) => log::write(&format!("이전 버전 파일 정리 실패: {e}")),
            }
        }
    }
}

//#region 설치 (백그라운드 스레드에서 실행)
pub fn install(version: &str) -> Result<(), String> {
    let exe = current_exe()?;
    let new = exe.with_extension("exe.new");
    let old = old_path(&exe);

    //#region 다운로드 + 검증 (같은 폴더에 받아서 쓰기 권한도 함께 확인)
    update::download(&update::asset_url(version, ASSET), &new)?;
    let bytes = std::fs::read(&new).map_err(|e| format!("받은 파일 읽기 실패: {e}"))?;
    if bytes.len() < 100 * 1024 || !bytes.starts_with(b"MZ") {
        let _ = std::fs::remove_file(&new);
        return Err(format!("받은 파일이 실행 파일이 아님 ({} 바이트)", bytes.len()));
    }
    //#endregion

    //#region 교체: 실행 중인 exe 는 지울 수 없지만 이름은 바꿀 수 있음
    let _ = std::fs::remove_file(&old);
    std::fs::rename(&exe, &old).map_err(|e| {
        let _ = std::fs::remove_file(&new);
        format!("현재 실행 파일 이동 실패: {e}")
    })?;
    if let Err(e) = std::fs::rename(&new, &exe) {
        // 되돌리기
        let _ = std::fs::rename(&old, &exe);
        let _ = std::fs::remove_file(&new);
        return Err(format!("새 실행 파일 배치 실패: {e}"));
    }
    //#endregion
    Ok(())
}
//#endregion

/// 새 버전 실행 (이 프로세스는 직후 종료해야 함. 새 프로세스는 잠금이 풀릴 때까지 기다림)
pub fn relaunch() -> Result<(), String> {
    // 이름을 바꾼 뒤에도 원래 경로를 쓰도록 .old 접미사 제거
    let mut exe = current_exe()?;
    if exe.to_string_lossy().to_ascii_lowercase().ends_with(".exe.old") {
        exe = exe.with_extension("");
    }
    Command::new(&exe).spawn().map(|_| ()).map_err(|e| format!("새 버전 실행 실패: {e}"))
}
