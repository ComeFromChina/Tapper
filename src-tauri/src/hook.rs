//! 全局低级钩子（一个线程、一个消息循环，同时装鼠标与键盘）。
//!
//! 热键为什么要这么写：
//! - 回调里绝不做耗时操作。Windows 对低级钩子有超时限制（默认 300ms），
//!   超时会把钩子**静默卸载**，之后所有热键都失效 —— 这就是“聚焦某个窗口后热键没反应”的主因。
//!   所以回调只把键码丢进队列，真正的处理在独立线程里做。
//! - 不再周期性重挂钩子：卸载到重新安装之间存在 1–2ms 空窗，
//!   且看门狗线程调度抖动时会与按键时机重合，导致 F6/F7 偶发丢键。

use std::sync::atomic::{AtomicIsize, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, OnceLock};

use windows::Win32::Foundation::{LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetAncestor, GetMessageW, SetWindowsHookExW, TranslateMessage,
    UnhookWindowsHookEx, WindowFromPoint, GA_ROOT, KBDLLHOOKSTRUCT, MSG, MSLLHOOKSTRUCT,
    WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_MOUSEMOVE,
    WM_SYSKEYDOWN, WM_SYSKEYUP,
};

use crate::engine::Engine;
use crate::hotkey;
use crate::input;
use crate::log;
use crate::recording::Recording;

const LLMHF_INJECTED: u32 = 0x0000_0001;
const CORNER_TOLERANCE: i32 = 6;

/// 每个热键的「按下时间戳（毫秒）」。0 表示未按下。
/// 用时间戳而不是布尔值：万一 KEYUP 丢失（全屏切换、Alt+Tab），
/// 超过 STALE_MS 就自动视为已松开，不会出现“按了没反应”。
static KEY_DOWN_AT: [AtomicU64; 4] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];

/// 按住超过该时长视为“上一次的按下状态已失效”，允许重新触发。
const STALE_MS: u64 = 400;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
static KEY_HOOK: AtomicIsize = AtomicIsize::new(0);
static KEY_TX: OnceLock<mpsc::Sender<u32>> = OnceLock::new();
static CTX: OnceLock<Arc<HookContext>> = OnceLock::new();

pub struct HookContext {
    pub app: tauri::AppHandle,
    pub engine: Arc<Engine>,
    pub recording: Arc<Recording>,
}

pub fn install(
    app: tauri::AppHandle,
    engine: Arc<Engine>,
    recording: Arc<Recording>,
) -> Vec<String> {
    let ctx = Arc::new(HookContext {
        app,
        engine,
        recording,
    });
    if CTX.set(ctx.clone()).is_err() {
        return Vec::new();
    }

    // 热键执行线程：钩子回调只负责投递，绝不在回调里做事。
    let (tx, rx) = mpsc::channel::<u32>();
    let _ = KEY_TX.set(tx);
    std::thread::Builder::new()
        .name("hotkey-worker".into())
        .spawn(move || {
            while let Ok(vk) = rx.recv() {
                hotkey::handle(&ctx.app, &ctx.engine, &ctx.recording, vk);
            }
        })
        .ok();

    let (ready_tx, ready_rx) = mpsc::channel::<Vec<String>>();
    std::thread::Builder::new()
        .name("input-hooks".into())
        .spawn(move || unsafe {
            let mut failed = Vec::new();

            let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), None, 0);
            if mouse.is_err() {
                log::error("安装鼠标钩子失败");
            }
            match SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), None, 0) {
                Ok(h) => {
                    KEY_HOOK.store(h.0 as isize, Ordering::SeqCst);
                    log::info("键盘热键钩子已安装");
                }
                Err(e) => {
                    log::error(&format!("安装键盘钩子失败：{e}"));
                    failed.extend(["F6".to_string(), "F7".to_string(), "F8".to_string(), "F9".to_string()]);
                    // 延迟重试：安全软件的钩子初始化有时比我们慢。
                    for _ in 0..10 {
                        std::thread::sleep(std::time::Duration::from_millis(500));
                        if let Ok(h) = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), None, 0)
                        {
                            KEY_HOOK.store(h.0 as isize, Ordering::SeqCst);
                            log::info("键盘热键钩子重试成功");
                            break;
                        }
                    }
                }
            }
            log::info("全局钩子线程就绪");
            let _ = ready_tx.send(failed);

            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            if let Ok(h) = mouse {
                let _ = UnhookWindowsHookEx(h);
            }
        })
        .ok();

    ready_rx.recv().unwrap_or_else(|_| Vec::new())
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        if let Some(ctx) = CTX.get() {
            let info = &*(lparam.0 as *const MSLLHOOKSTRUCT);
            if info.flags & LLMHF_INJECTED == 0 {
                let message = wparam.0 as u32;

                // 录制先记账：即使这次点击被悬浮层吃掉，坐标也要留下。
                if message == WM_LBUTTONDOWN
                    && ctx.recording.is_active()
                    && !is_own_window_at(info.pt)
                {
                    ctx.recording.push_click(info.pt.x, info.pt.y);
                }

                // 悬浮层命中测试：命中的事件由悬浮层吃掉，游戏不该再收到。
                if crate::overlay_input::handle_mouse(&ctx.app, message, info.pt) {
                    return LRESULT(1);
                }

                if message == WM_MOUSEMOVE
                    && ctx.engine.is_running()
                    && ctx.engine.corner_enabled()
                {
                    let (cx, cy) = input::corner_point();
                    if (info.pt.x - cx).abs() <= CORNER_TOLERANCE
                        && (info.pt.y - cy).abs() <= CORNER_TOLERANCE
                    {
                        log::info("甩角急停触发");
                        ctx.engine.stop(Some("鼠标甩到左上角，已紧急停止"));
                    }
                }
            }
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let info = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        let vk = info.vkCode;
        if (hotkey::VK_F6..=hotkey::VK_F9).contains(&vk) {
            let message = wparam.0 as u32;
            let index = (vk - hotkey::VK_F6) as usize;
            if message == WM_KEYDOWN || message == WM_SYSKEYDOWN {
                // 回调内只做原子操作 + 入队，必须立即返回。
                let now = now_ms();
                let prev = KEY_DOWN_AT[index].swap(now, Ordering::SeqCst);
                let repeated = prev != 0 && now.saturating_sub(prev) < STALE_MS;
                if !repeated {
                    if let Some(tx) = KEY_TX.get() {
                        let _ = tx.send(vk);
                    }
                }
                return LRESULT(1);
            }
            if message == WM_KEYUP || message == WM_SYSKEYUP {
                KEY_DOWN_AT[index].store(0, Ordering::SeqCst);
                return LRESULT(1);
            }
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

unsafe fn is_own_window_at(pt: POINT) -> bool {
    let hwnd = WindowFromPoint(pt);
    if hwnd.0.is_null() {
        return false;
    }
    let root = GetAncestor(hwnd, GA_ROOT);
    let root = if root.0.is_null() { hwnd } else { root };
    let (pid, _) = input::window_info(root.0 as isize);
    pid == input::current_pid()
}
