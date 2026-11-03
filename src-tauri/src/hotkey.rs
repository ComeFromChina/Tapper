//! 全局热键动作。
//!
//! 不用 RegisterHotKey：它会被其他程序抢占，且在小程序 / 游戏窗口获得焦点时可能静默失效。
//! 改为在 hook 线程里装 WH_KEYBOARD_LL，只要按键进到系统就能收到，并在命中时吞掉按键，
//! 避免游戏同时收到 F6/F7。

use std::sync::Arc;

use tauri::{AppHandle, Emitter};

use crate::engine::Engine;
use crate::recording::Recording;

pub const EVT_STOP: &str = "hotkey-stop";
pub const EVT_RECORD: &str = "hotkey-record";

pub const VK_F6: u32 = 0x75;
pub const VK_F7: u32 = 0x76;
pub const VK_F8: u32 = 0x77;
pub const VK_F9: u32 = 0x78;

pub fn handle(app: &AppHandle, engine: &Arc<Engine>, recording: &Arc<Recording>, vk: u32) -> bool {
    match vk {
        VK_F6 => {
            if engine.is_active() {
                engine.stop(Some("快捷键 F6 停止"));
            } else if let Err(msg) = crate::commands::start_engine_internal(app) {
                crate::commands::set_notice(app, &msg);
                crate::show_main_window(app);
            }
            true
        }
        VK_F7 => {
            let was_active = engine.is_active();
            engine.stop(Some("紧急停止（F7）"));
            if was_active {
                let _ = app.emit(EVT_STOP, ());
            }
            true
        }
        VK_F8 => {
            if recording.is_active() {
                recording.stop();
                let _ = app.emit(EVT_RECORD, false);
            } else if !engine.is_active() {
                recording.start();
                let _ = app.emit(EVT_RECORD, true);
            }
            true
        }
        VK_F9 => {
            if !engine.is_active() {
                if let Err(msg) = crate::commands::toggle_marker_internal(app) {
                    crate::commands::set_notice(app, &msg);
                }
            }
            true
        }
        _ => false,
    }
}
