use std::io::Write;

//#region 로그 출력 (stderr + 파일)
/// stderr 와 `<data_dir>/force-stretch.log` 에 함께 기록한다.
/// Windows GUI 앱은 stderr 가 없으므로 파일 로그가 유일한 진단 수단이다.
pub fn write(msg: &str) {
    let line = format!(
        "[{}] {msg}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    );
    eprintln!("[force-stretch] {msg}");
    let dir = super::config::data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("force-stretch.log");
    // 로그가 1MB 를 넘으면 비우고 새로 시작
    if std::fs::metadata(&path).map(|m| m.len() > 1_000_000).unwrap_or(false) {
        let _ = std::fs::remove_file(&path);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{line}");
    }
}
//#endregion
