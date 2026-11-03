use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};

use crate::models::AppConfig;

/// 配置文件路径：<config_dir>/Tapper/config.json
pub fn config_path(app: &AppHandle) -> PathBuf {
    let base = app
        .path()
        .app_config_dir()
        .unwrap_or_else(|_| PathBuf::from("."));
    base.join("config.json")
}

/// 载入配置；文件不存在、损坏、带 BOM 都回落默认值，绝不因配置问题启动失败。
pub fn load(app: &AppHandle) -> AppConfig {
    let path = config_path(app);
    let Ok(bytes) = std::fs::read(&path) else {
        return AppConfig::default();
    };
    let text = String::from_utf8_lossy(&bytes);
    let text = text.trim_start_matches('\u{feff}');
    serde_json::from_str(text).unwrap_or_default()
}

/// 保存配置（原子写：先写临时文件再替换）。
pub fn save(app: &AppHandle, config: &AppConfig) {
    let path = config_path(app);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let Ok(json) = serde_json::to_string_pretty(config) else {
        return;
    };
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, json.as_bytes()).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// 主显示器中心（首次启动时的默认点位）。
pub fn primary_monitor_center(app: &AppHandle) -> (i32, i32) {
    let window = app
        .get_webview_window("main")
        .or_else(|| app.webview_windows().into_values().next());
    if let Some(window) = window {
        if let Ok(Some(monitor)) = window.primary_monitor() {
            let pos = monitor.position();
            let size = monitor.size();
            return (
                pos.x + (size.width as i32) / 2,
                pos.y + (size.height as i32) / 2,
            );
        }
    }
    (800, 500)
}

#[allow(dead_code)]
pub fn ensure_dir(path: &Path) {
    let _ = std::fs::create_dir_all(path);
}
