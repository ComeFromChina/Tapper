//! 悬浮层窗口：全屏透明、置顶、永不抢焦点、永远不接收鼠标。
//!
//! 关键点：
//! - 创建时加 WS_EX_NOACTIVATE + WS_EX_TOOLWINDOW，并且显示必须走 SW_SHOWNOACTIVATE
//!   （见 `show_without_activating`）：tao 的 `show()` 在首次显示之后会退回 SW_SHOW，
//!   那会把这块全屏层变成前台窗口，把你正在打字的窗口的输入焦点顶掉
//!   —— 表现为「一开启悬浮按钮就丢焦点」；
//! - 窗口永远 click-through：WebView2 的透明区域不会自动穿透，
//!   只要这个窗口接收鼠标，整块屏幕的点击都会被它吞掉（状态条周围会出现看不见的遮罩），
//!   所以鼠标交互全部改在全局钩子里做命中测试，见 overlay_input；
//! - 窗口按「选中的显示器」的物理像素铺满，前端用 CSS 像素换算。

use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::log;
use crate::models::{MonitorInfo, OverlayGeometry};

/// 枚举显示器（物理像素 + 缩放）。
pub fn monitors(app: &AppHandle) -> Vec<MonitorInfo> {
    let Some(main) = app.get_webview_window("main") else {
        return Vec::new();
    };
    // Windows 上 Monitor::name() 可能为 None，不能靠名字认定主屏，
    // 用「位置 + 尺寸」与 primary_monitor() 比对才可靠。
    let primary = main.primary_monitor().ok().flatten();
    let primary_key = primary
        .as_ref()
        .map(|m| (m.position().x, m.position().y, m.size().width, m.size().height));

    let mut list = Vec::new();
    if let Ok(ms) = main.available_monitors() {
        for m in ms {
            let name = m.name().cloned().unwrap_or_default();
            let key = (m.position().x, m.position().y, m.size().width, m.size().height);
            list.push(MonitorInfo {
                name,
                x: m.position().x,
                y: m.position().y,
                width: m.size().width,
                height: m.size().height,
                scale_factor: m.scale_factor(),
                is_primary: primary_key == Some(key),
            });
        }
    }
    // 兜底：一个都没标上主屏时，把第一个当作主屏，避免退化成“随便选一个”。
    if !list.is_empty() && !list.iter().any(|m| m.is_primary) {
        if let Some(first) = list.first_mut() {
            first.is_primary = true;
        }
    }
    list
}


/// 标记离屏幕边缘至少留这么多物理像素（不然半个图标被切掉）。
pub const POINT_MARGIN: i32 = 24;
/// 状态条整条离屏幕边缘的留白。
pub const HUD_MARGIN: i32 = 8;

/// 由悬浮层几何反推显示器信息（显示器列表拿不到时的兜底）。
pub fn monitor_of(geo: OverlayGeometry) -> MonitorInfo {
    MonitorInfo {
        name: String::new(),
        x: geo.origin_x,
        y: geo.origin_y,
        width: geo.width,
        height: geo.height,
        scale_factor: geo.scale_factor,
        is_primary: true,
    }
}

/// 绝对坐标落在哪块屏（屏内左闭右开，两块屏不会抢同一条公共边）；
/// 落在屏与屏之间的空隙就返回 current，表示「没走进另一块屏」。
pub fn monitor_for(list: &[MonitorInfo], current: usize, x: i32, y: i32) -> usize {
    list.iter()
        .position(|m| m.x <= x && x < m.x + m.width as i32 && m.y <= y && y < m.y + m.height as i32)
        .unwrap_or(current)
}

/// 落点收进这块屏内，离边至少留 margin（不然半个标记被切在屏幕外）。
pub fn clamp_point_in(m: &MonitorInfo, x: i32, y: i32, margin: i32) -> (i32, i32) {
    let min_x = m.x + margin;
    let max_x = (m.x + m.width as i32 - margin).max(min_x);
    let min_y = m.y + margin;
    let max_y = (m.y + m.height as i32 - margin).max(min_y);
    (x.clamp(min_x, max_x), y.clamp(min_y, max_y))
}

/// 绝对坐标在这块屏里吗。
pub fn contains_point(m: &MonitorInfo, x: i32, y: i32) -> bool {
    m.x <= x && x < m.x + m.width as i32 && m.y <= y && y < m.y + m.height as i32
}

/// 这块屏的正中（物理像素绝对坐标）。
pub fn center_of(m: &MonitorInfo) -> (i32, i32) {
    (m.x + (m.width as i32) / 2, m.y + (m.height as i32) / 2)
}

/// 换屏之后标记该放哪。
///
/// 标记存的还是上一块屏的绝对坐标，整层切过去了它就在画面外 —— 表现为「状态条拖过去了，
/// 悬浮按钮却找不到了」。所以：本来就在这块屏里就只夹进边距，跑出去了直接放屏幕正中。
pub fn marker_after_switch(m: &MonitorInfo, x: i32, y: i32) -> (i32, i32) {
    if contains_point(m, x, y) {
        clamp_point_in(m, x, y, POINT_MARGIN)
    } else {
        center_of(m)
    }
}

/// 状态条在该屏的基准中心：顶部居中，换算与 state::hud_center 保持一致。
pub fn hud_base(m: &MonitorInfo) -> (i32, i32) {
    let s = m.scale_factor.max(0.5);
    (m.x + (m.width as i32) / 2, m.y + ((18.0 + 17.0) * s) as i32)
}

/// 想让状态条中心落在 (want_x, want_y)：相对该屏基准中心该存多少偏移，整条留在屏内。
///
/// 状态条存的是「相对本屏顶部中心的偏移」，所以换屏一定要按新屏重新换算，
/// 直接沿用旧偏移会跑到屏幕外面去。
pub fn hud_offset_in(m: &MonitorInfo, want_x: i32, want_y: i32, margin: i32) -> (i32, i32) {
    let (base_x, base_y) = hud_base(m);
    hud_offset_from_base(m, base_x, base_y, want_x - base_x, want_y - base_y, margin)
}

/// 已有偏移放进另一块屏：放得下就原样保留，放不下就夹到整条可见。
pub fn hud_offset_keep_within(m: &MonitorInfo, base_x: i32, base_y: i32, dx: i32, dy: i32, margin: i32) -> (i32, i32) {
    hud_offset_from_base(m, base_x, base_y, dx, dy, margin)
}

fn hud_offset_from_base(
    m: &MonitorInfo,
    base_x: i32,
    base_y: i32,
    dx: i32,
    dy: i32,
    margin: i32,
) -> (i32, i32) {
    let s = m.scale_factor.max(0.5);
    let half_w = (200.0 * s) as i32;
    let half_h = (22.0 * s) as i32;
    let min_dx = m.x + margin + half_w - base_x;
    let max_dx = (m.x + m.width as i32 - margin - half_w - base_x).max(min_dx);
    let min_dy = m.y + margin + half_h - base_y;
    let max_dy = (m.y + m.height as i32 - margin - half_h - base_y).max(min_dy);
    (dx.clamp(min_dx, max_dx), dy.clamp(min_dy, max_dy))
}

fn ensure(app: &AppHandle) -> Result<WebviewWindow, String> {
    if let Some(win) = app.get_webview_window("overlay") {
        return Ok(win);
    }
    let win = WebviewWindowBuilder::new(app, "overlay", WebviewUrl::App("index.html".into()))
        .title("Tapper Overlay")
        .transparent(true)
        .decorations(false)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .visible(false)
        .build()
        .map_err(|e| format!("创建悬浮层失败：{e}"))?;

    apply_no_activate(&win);
    let _ = win.set_ignore_cursor_events(true);
    Ok(win)
}

/// 打开/刷新悬浮层到目标显示器。`monitor_index` 是 `monitors()` 列表里的序号，
/// 越界或 None 都退回主屏（换过屏幕、拔过线时旧序号可能已经不存在）。
pub fn open(
    app: &AppHandle,
    monitor_index: Option<usize>,
) -> Result<(OverlayGeometry, Vec<MonitorInfo>), String> {
    let win = ensure(app)?;
    let list = monitors(app);
    log::info(&format!(
        "枚举到 {} 块显示器：{}",
        list.len(),
        list.iter()
            .enumerate()
            .map(|(i, m)| format!(
                "[{}] {} ({},{}) {}x{} x{:.1} {}",
                i,
                m.name,
                m.x,
                m.y,
                m.width,
                m.height,
                m.scale_factor,
                if m.is_primary { "主屏" } else { "副屏" }
            ))
            .collect::<Vec<_>>()
            .join("；")
    ));
    let chosen = monitor_index
        .and_then(|i| list.get(i))
        .cloned()
        .or_else(|| list.iter().find(|m| m.is_primary).cloned())
        .or_else(|| list.first().cloned())
        .ok_or_else(|| "没有检测到可用显示器".to_string())?;

    win.set_position(PhysicalPosition::new(chosen.x, chosen.y))
        .map_err(|e| format!("定位悬浮层失败：{e}"))?;
    win.set_size(PhysicalSize::new(chosen.width, chosen.height))
        .map_err(|e| format!("设置悬浮层尺寸失败：{e}"))?;
    let _ = win.set_always_on_top(true);
    show_without_activating(&win);
    apply_no_activate(&win);
    // 永远穿透：交互在全局钩子里做命中测试。
    win.set_ignore_cursor_events(true)
        .map_err(|e| format!("设置悬浮层鼠标穿透失败：{e}"))?;

    let geometry = OverlayGeometry {
        origin_x: chosen.x,
        origin_y: chosen.y,
        width: chosen.width,
        height: chosen.height,
        scale_factor: chosen.scale_factor,
    };
    log::info(&format!(
        "悬浮层打开：monitor={} ({},{}) {}x{}",
        chosen.name, chosen.x, chosen.y, chosen.width, chosen.height
    ));
    Ok((geometry, list))
}

pub fn hide(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("overlay") {
        let _ = win.hide();
    }
}

/// 悬浮层永远保持鼠标穿透（交互由全局钩子命中测试完成）。
pub fn set_click_through(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("overlay") {
        let _ = win.set_ignore_cursor_events(true);
    }
}

/// 显示悬浮层，但**不**让它变成前台窗口。
///
/// 不能用 `win.show()`：tao 只在第一次显示（靠 `.focused(false)` 设的那次）用 SW_SHOWNOACTIVATE，
/// 之后再显示一律走 SW_SHOW —— 那会把这块全屏透明层顶成前台窗口。
/// 用户的表现就是「一开启悬浮按钮，打字的光标就没了」：隐藏后再显示标记 = 每次显示都抢一次焦点。
#[cfg(target_os = "windows")]
fn show_without_activating(win: &WebviewWindow) {
    use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_SHOWNOACTIVATE};
    if let Ok(hwnd) = win.hwnd() {
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn show_without_activating(win: &WebviewWindow) {
    let _ = win.show();
}

#[cfg(target_os = "windows")]
fn apply_no_activate(win: &WebviewWindow) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    };
    let Ok(handle) = win.hwnd() else { return };
    unsafe {
        let hwnd = HWND(handle.0);
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let next = style | (WS_EX_NOACTIVATE.0 as isize) | (WS_EX_TOOLWINDOW.0 as isize);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, next);
    }
}

#[cfg(not(target_os = "windows"))]
fn apply_no_activate(_win: &WebviewWindow) {}

#[cfg(test)]
mod screen_tests {
    use super::*;

    // 按本机实测的两块屏建模：主屏 2160x1440 x1.5，副屏 (2160,-140) 1600x900 x1.0。
    fn main_screen() -> MonitorInfo {
        MonitorInfo {
            name: "DISPLAY1".into(),
            x: 0,
            y: 0,
            width: 2160,
            height: 1440,
            scale_factor: 1.5,
            is_primary: true,
        }
    }

    fn second_screen() -> MonitorInfo {
        MonitorInfo {
            name: "DISPLAY2".into(),
            x: 2160,
            y: -140,
            width: 1600,
            height: 900,
            scale_factor: 1.0,
            is_primary: false,
        }
    }

    fn list() -> Vec<MonitorInfo> {
        vec![main_screen(), second_screen()]
    }

    #[test]
    fn landing_on_the_other_screen_asks_to_switch() {
        assert_eq!(super::monitor_for(&list(), 0, 2500, 300), 1);
        // 公共边按「左闭右开」归右边那块屏，两块屏不会抢同一个坐标。
        assert_eq!(super::monitor_for(&list(), 0, 2160, 300), 1);
        assert_eq!(super::monitor_for(&list(), 1, 2159, 300), 0);
    }

    #[test]
    fn landing_in_the_gap_between_screens_stays_put() {
        // 副屏从 y=-140 起，主屏上方那块空地不属于任何屏幕：留在当前屏，别乱切。
        assert_eq!(super::monitor_for(&list(), 0, 1000, -200), 0);
        assert_eq!(super::monitor_for(&list(), 1, 5000, 100), 1);
    }

    #[test]
    fn marker_is_clamped_inside_the_screen_it_landed_on() {
        assert_eq!(super::clamp_point_in(&second_screen(), 2500, 300, 24), (2500, 300));
        assert_eq!(
            super::clamp_point_in(&second_screen(), 9999, -9999, 24),
            (3736, -116),
            "越界要夹回副屏内侧，而不是留在主屏"
        );
    }

    #[test]
    fn hud_offset_is_rebased_on_whichever_screen_it_is_dropped_onto() {
        // 拖到副屏正上方 = 偏移归零；同一个绝对坐标在主屏却是不小的偏移。
        let second = second_screen();
        let (base_x, base_y) = super::hud_base(&second);
        assert_eq!(super::hud_offset_in(&second, base_x, base_y, 8), (0, 0));
        assert_eq!(super::hud_offset_in(&second, base_x + 300, base_y, 8), (300, 0));
    }

    #[test]
    fn hud_stays_fully_inside_the_new_screen() {
        let second = second_screen();
        let (dx, dy) = super::hud_offset_in(&second, 999_999, -999_999, 8);
        let (base_x, base_y) = super::hud_base(&second);
        // 副屏缩放 1.0：半宽 200、半高 22，整条右边缘正好停在离边 8px 处。
        assert_eq!(base_x + dx + 200, 2160 + 1600 - 8);
        assert_eq!(base_y + dy - 22, -140 + 8);
    }

    #[test]
    fn hud_offset_survives_a_switch_when_it_still_fits() {
        let second = second_screen();
        let (base_x, base_y) = hud_base(&second);
        // 主屏上偏右 300px，搬到 1600 宽的副屏仍然放得下 → 原样保留，别把用户摆好的位置抹掉。
        assert_eq!(hud_offset_keep_within(&second, base_x, base_y, 300, 40, 8), (300, 40));
        // 放不下就夹到整条可见（副屏基准中心 + 偏移 + 半宽 = 右边缘内缩 8px）。
        let (dx, _) = hud_offset_keep_within(&second, base_x, base_y, 900, 0, 8);
        assert_eq!(base_x + dx + 200, 2160 + 1600 - 8);
    }

    #[test]
    fn a_marker_left_on_the_old_screen_reappears_centred() {
        let second = second_screen();
        // 用户实测的那条：标记还停在主屏右缘，整层切去副屏后它就在画面外了 → 放副屏正中。
        assert_eq!(marker_after_switch(&second, 2136, 754), (2960, 310));
        assert!(contains_point(&second, 2960, 310), "正中当然在这块屏里");
        // 本来就在这块屏里：位置不动，只把贴边的夹进边距。
        assert_eq!(marker_after_switch(&second, 2500, 300), (2500, 300));
        assert_eq!(marker_after_switch(&second, 2166, 300), (2184, 300));
        // 主屏的原点可以是负的（副屏在上半轴），判定不能只按「大于 0」。
        assert!(!contains_point(&second, 2159, 300));
        assert!(contains_point(&second, 2160, -139));
    }
}
