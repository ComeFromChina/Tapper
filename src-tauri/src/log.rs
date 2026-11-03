//! 零依赖文件日志。release 版无控制台，关键事件必须落文件才能排障。

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();
static LOG_FILE: OnceLock<Mutex<Option<File>>> = OnceLock::new();

pub fn init(path: PathBuf) {
    let _ = LOG_PATH.set(path.clone());
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = LOG_FILE.set(Mutex::new(Some(file)));
    }
}

pub fn info(message: &str) {
    write("INFO", message);
}

pub fn error(message: &str) {
    write("ERROR", message);
}

fn write(level: &str, message: &str) {
    let Some(file) = LOG_FILE.get() else { return };
    let stamp = timestamp();
    let mut guard = match file.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    if let Some(handle) = guard.as_mut() {
        let _ = writeln!(handle, "[{stamp}] [{level}] {message}");
        let _ = handle.flush();
    }
}

fn timestamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // 本地时间偏移无法在无 std 时区库下可靠获取，日志用 Unix 秒便于对照。
    format!("{secs}")
}

pub fn path() -> PathBuf {
    LOG_PATH.get().cloned().unwrap_or_else(|| PathBuf::from("app.log"))
}
