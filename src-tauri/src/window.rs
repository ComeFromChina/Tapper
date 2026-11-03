//! 窗口归属判定。停止条件用「鼠标落到了哪个窗口」来判断，
//! 而不是看前台句柄 —— 切到小窗口、鼠标去别处点，都不应打断后台连点。

use windows::Win32::Foundation::POINT;
use windows::Win32::UI::WindowsAndMessaging::{GetAncestor, GA_ROOT, WindowFromPoint};

/// 坐标处最顶层的顶层窗口句柄（GA_ROOT），返回 0 表示该处没有窗口（桌面）。
pub fn window_at(x: i32, y: i32) -> isize {
    unsafe {
        let h = WindowFromPoint(POINT { x, y });
        if h.0.is_null() {
            return 0;
        }
        let root = GetAncestor(h, GA_ROOT);
        if root.0.is_null() {
            h.0 as isize
        } else {
            root.0 as isize
        }
    }
}
