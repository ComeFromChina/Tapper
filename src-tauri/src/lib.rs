//! Tapper 应用入口。
//!
//! 关键约定：
//! - 关闭主窗口 = 隐藏到托盘，引擎继续跑；
//! - 引擎真值全在 Rust 侧，热键/托盘停止不依赖前端；
//! - 运行态不持久化，重启必回空闲。

mod commands;
mod config;
mod engine;
mod hook;
mod hotkey;
mod input;
mod log;
mod models;
mod overlay;
mod overlay_input;
mod recording;
mod state;
mod tray;
mod window;

use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Manager, WindowEvent};

/// 托盘退出时若正在运行，请求前端二次确认。
pub const EVT_QUIT_CONFIRM: &str = "request-quit-confirm";

pub fn run() {
    let engine = Arc::new(engine::Engine::new());
    let recording = Arc::new(recording::Recording::new());

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            // 第二实例只唤起面板；写日志以便区分“单实例静默退出”与“启动崩溃”。
            log::info("检测到第二个实例，已唤起主面板");
            show_main_window(app);
        }))
        .setup(move |app| {
            let handle = app.handle().clone();

            let log_dir = app
                .path()
                .app_log_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));
            log::init(log_dir.join("tapper.log"));
            log::info(&format!(
                "Tapper 启动 v{}（{}权限）",
                env!("CARGO_PKG_VERSION"),
                if input::is_elevated() { "管理员" } else { "普通用户" }
            ));

            let mut cfg = config::load(&handle);
            if cfg.point.x == 0 && cfg.point.y == 0 {
                let (x, y) = config::primary_monitor_center(&handle);
                cfg.point = models::ClickPoint { x, y };
            }

            if let Some(main) = app.get_webview_window("main") {
                if let Ok(h) = main.hwnd() {
                    engine.set_main_hwnd(h.0 as isize);
                }
            }

            // 鼠标钩子（急停/录制）与键盘钩子（F6–F9）一起装，返回注册失败项。
            let failed_hotkeys = hook::install(
                handle.clone(),
                Arc::clone(&engine),
                Arc::clone(&recording),
            );
            let monitors = overlay::monitors(&handle);

            let tray = tray::TrayController::new();
            tray.setup(
                &handle,
                |app| {
                    let active = app
                        .try_state::<state::AppState>()
                        .map(|s| s.engine.is_active())
                        .unwrap_or(false);
                    if active {
                        if let Some(s) = app.try_state::<state::AppState>() {
                            s.engine.stop(Some("托盘停止"));
                        }
                        return;
                    }
                    if let Err(msg) = commands::start_engine_internal(app) {
                        commands::set_notice(app, &msg);
                        show_main_window(app);
                    }
                },
                |app| {
                    if let Some(s) = app.try_state::<state::AppState>() {
                        s.engine.stop(Some("托盘紧急停止"));
                    }
                },
                |app| {
                    if let Err(msg) = commands::toggle_marker_internal(app) {
                        commands::set_notice(app, &msg);
                    }
                },
            )?;

            // 托盘图标跟随引擎状态切换（前端的启停不一定经过托盘，需要主动同步）。
            let tray_watcher = tray.clone();
            let engine_watcher = Arc::clone(&engine);
            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_millis(600));
                tray_watcher.set_state(if engine_watcher.is_running() {
                    tray::TrayState::Running
                } else {
                    tray::TrayState::Idle
                });
            });

            let initial_marker = cfg.show_marker;
            let initial_monitor = cfg.monitor_index;

            app.manage(state::AppState::new(
                cfg,
                Arc::clone(&engine),
                Arc::clone(&recording),
                monitors,
                failed_hotkeys,
            ));
            app.manage(tray);

            // 悬浮层的鼠标交互（命中测试 / 拖拽 / 光标）全部在 Rust 侧完成。
            overlay_input::start(handle.clone());

            // 上次退出时标记是显示的 → 启动即恢复标记（穿透、不抢焦点）。
            if initial_marker {
                match overlay::open(&handle, initial_monitor) {
                    Ok((geometry, list)) => {
                        if let Some(s) = app.try_state::<state::AppState>() {
                            *s.overlay_geometry.lock().unwrap_or_else(|p| p.into_inner()) =
                                Some(geometry);
                            *s.monitors.lock().unwrap_or_else(|p| p.into_inner()) = list;
                            s.set_overlay_mode(models::OverlayMode::Passive);
                            s.refresh_hit_zones();
                        }
                    }
                    Err(e) => log::error(&e),
                }
            }

                        Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::frontend_ready,
            commands::get_state,
            commands::get_overlay_state,
            commands::start_engine,
            commands::stop_engine,
            commands::set_point,
            commands::set_point_at_cursor,
            commands::set_cps,
            commands::set_options,
            commands::set_target_window,
            commands::pick_target_window,
            commands::restart_as_admin,
            commands::preview_click,
            commands::set_theme,
            commands::set_mode,
            commands::set_monitor,
            commands::set_marker_visible,
            commands::toggle_marker,
            commands::set_overlay_rects,
            commands::set_hud_offset,
            commands::reset_hud_offset,
            commands::open_panel,
            commands::refresh_monitors,
            commands::start_recording,
            commands::stop_recording,
            commands::discard_recording,
            commands::apply_recording,
            commands::save_script,
            commands::delete_script,
            commands::select_script,
            commands::rename_script,
            commands::hide_main_window,
            commands::quit_app,
            commands::open_log_dir,
        ])
        .run(tauri::generate_context!())
        .expect("Tapper 启动失败");
}

/// 显示并聚焦主窗口（托盘 / 单实例 / 悬浮层右键菜单共用）。
///
/// 光靠 show + set_focus 不够：目标程序多半是全屏游戏，窗口可能被压在后面，
/// 用户看着就像「没打开」。所以临时置顶一会儿再撤掉，保证窗口真的跳到前面。
pub fn show_main_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_always_on_top(true);
        let _ = win.set_focus();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(1500));
            let _ = win.set_always_on_top(false);
        });
    }
}
