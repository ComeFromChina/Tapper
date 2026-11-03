//! 前端命令层：薄，只做参数校验、状态读写与系统调用编排。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::MutexGuard;

use tauri::{AppHandle, Emitter, Manager, State};

use crate::config;
use crate::log;
use crate::models::{
    AppConfig, AppStateDto, ClickMode, ClickPoint, MonitorInfo, OverlayGeometry, OverlayMode,
    OverlayRects,
    OverlayStateDto, RecordingSummary, RunMode, Script, ScriptStep, StartPlan, Theme,
};
use crate::overlay;
use crate::state::AppState;

fn cfg_lock(state: &AppState) -> MutexGuard<'_, AppConfig> {
    state.config.lock().unwrap_or_else(|p| p.into_inner())
}

fn save(app: &AppHandle, cfg: &AppConfig) {
    config::save(app, cfg);
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 记录一条提示，下一次 get_state 时带给界面。
pub fn set_notice(app: &AppHandle, message: &str) {
    if let Some(state) = app.try_state::<AppState>() {
        *state.notice.lock().unwrap_or_else(|p| p.into_inner()) = Some(message.to_string());
    }
    log::error(message);
}

/// 启动（命令、托盘、热键共用同一条路径）。
pub fn start_engine_internal(app: &AppHandle) -> Result<(), String> {
    let state = app.try_state::<AppState>().ok_or("应用状态未初始化")?;
    let cfg = state.config_snapshot();

    let plan = match cfg.mode {
        RunMode::Point => StartPlan::Point {
            x: cfg.point.x,
            y: cfg.point.y,
            cps: cfg.cps,
            hold_ms: cfg.hold_ms,
        },
        RunMode::Script => {
            let script = cfg
                .active_script()
                .ok_or("请先选择或录制一份脚本")?;
            if script.steps.is_empty() {
                return Err("所选脚本没有步骤".into());
            }
            StartPlan::Script {
                steps: script.steps.clone(),
                hold_ms: cfg.hold_ms,
            }
        }
    };

    overlay::set_click_through(app);
    let result = state.engine.start(
        plan,
        cfg.click_mode,
        cfg.countdown_secs,
        cfg.corner_failsafe,
        cfg.window_guard,
        cfg.target_hwnd.map(|h| h as isize),
    );
    if let Err(e) = &result {
        state.engine.note_stop_reason(e);
    }
    result
}

/// 前端首次取状态时记录一次，作为页面真的渲染成功的铁证（release 无控制台）。
static FRONTEND_SEEN: AtomicBool = AtomicBool::new(false);

#[tauri::command]
pub fn frontend_ready(app: AppHandle) {
    log::info("前端挂载完成，显示主窗口");
    crate::show_main_window(&app);
}

#[tauri::command]
pub fn get_state(app: AppHandle, state: State<'_, AppState>) -> AppStateDto {
    if !FRONTEND_SEEN.swap(true, Ordering::Relaxed) {
        log::info("前端已连接（get_state 首次调用）");
    }
    let config = state.config_snapshot();
    let recording = state.recording.summary();
    let is_recording = state.recording.is_active();
    let monitors = state
        .monitors
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    let failed_hotkeys = state
        .failed_hotkeys
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();

    AppStateDto {
        config,
        engine: state.engine.status(),
        recording,
        is_recording,
        overlay_mode: state.overlay_mode(),
        overlay: *state
            .overlay_geometry
            .lock()
            .unwrap_or_else(|p| p.into_inner()),
        monitors,
        failed_hotkeys,
        elevated: crate::input::is_elevated(),
        notice: state.take_notice(),
        version: app.package_info().version.to_string(),
        app_name: app.package_info().name.clone(),
        description: app.package_info().description.to_string(),
        exe_dir: std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_string_lossy().to_string()))
            .unwrap_or_default(),
        log_path: log::path().to_string_lossy().to_string(),
    }
}

#[tauri::command]
pub fn get_overlay_state(state: State<'_, AppState>) -> OverlayStateDto {
    OverlayStateDto {
        config: state.config_snapshot(),
        engine: state.engine.status(),
        mode: state.overlay_mode(),
        geometry: *state
            .overlay_geometry
            .lock()
            .unwrap_or_else(|p| p.into_inner()),
    }
}

#[tauri::command]
pub fn start_engine(app: AppHandle) -> Result<(), String> {
    start_engine_internal(&app)
}

#[tauri::command]
pub fn stop_engine(state: State<'_, AppState>) {
    state.engine.stop(Some("手动停止"));
}

#[tauri::command]
pub fn set_point(app: AppHandle, state: State<'_, AppState>, x: i32, y: i32) -> Result<(), String> {
    set_point_internal(&app, &state, x, y)
}

/// 写入点击点并落盘（命令与悬浮层拖动共用）。
pub fn set_point_internal(
    app: &AppHandle,
    state: &AppState,
    x: i32,
    y: i32,
) -> Result<(), String> {
    {
        let mut cfg = cfg_lock(state);
        cfg.point = ClickPoint { x, y };
        let snapshot = cfg.clone();
        drop(cfg);
        save(app, &snapshot);
    }
    state.refresh_hit_zones();
    Ok(())
}

/// 把点位放到当前鼠标位置（面板上的快捷定位）。
#[tauri::command]
pub fn set_point_at_cursor(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    set_point_at_cursor_internal(&app, &state)
}

/// 把点位放到当前鼠标位置（命令与悬浮层菜单共用）。
pub fn set_point_at_cursor_internal(app: &AppHandle, state: &AppState) -> Result<(), String> {
    let (x, y) = crate::input::cursor_pos();
    set_point_internal(app, state, x, y)
}

#[tauri::command]
pub fn set_cps(app: AppHandle, state: State<'_, AppState>, cps: f64) -> Result<(), String> {
    if !cps.is_finite() {
        return Err("速率无效".into());
    }
    let cps = (cps.clamp(1.0, 200.0) * 10.0).round() / 10.0;
    let mut cfg = cfg_lock(&state);
    cfg.cps = cps;
    let snapshot = cfg.clone();
    drop(cfg);
    save(&app, &snapshot);
    Ok(())
}

#[tauri::command]
pub fn set_options(
    app: AppHandle,
    state: State<'_, AppState>,
    countdown_secs: u32,
    corner_failsafe: bool,
    window_guard: bool,
    hold_ms: u32,
    click_mode: Option<ClickMode>,
) -> Result<(), String> {
    let mut cfg = cfg_lock(&state);
    cfg.countdown_secs = countdown_secs.min(10);
    cfg.corner_failsafe = corner_failsafe;
    cfg.window_guard = window_guard;
    cfg.hold_ms = hold_ms.min(1000);
    if let Some(mode) = click_mode {
        cfg.click_mode = mode;
    }
    let snapshot = cfg.clone();
    drop(cfg);
    save(&app, &snapshot);
    Ok(())
}

/// 指定点击目标窗口（None = 启动时自动锁定前台窗口）。
#[tauri::command]
pub fn set_target_window(
    app: AppHandle,
    state: State<'_, AppState>,
    hwnd: Option<i64>,
) -> Result<(), String> {
    {
        let mut cfg = cfg_lock(&state);
        cfg.target_hwnd = hwnd;
        let snapshot = cfg.clone();
        drop(cfg);
        save(&app, &snapshot);
    }
    Ok(())
}

/// 以管理员身份重启（目标程序提权运行时需要）。
#[tauri::command]
pub fn restart_as_admin(app: AppHandle) -> Result<(), String> {
    if let Some(state) = app.try_state::<AppState>() {
        state.engine.stop(Some("提权重启"));
    }
    overlay::hide(&app);
    std::thread::sleep(std::time::Duration::from_millis(120));
    crate::input::relaunch_as_admin()?;
    app.exit(0);
    Ok(())
}

/// 单次测试点击：2 秒后按当前点击方式点一下，方便在游戏里快速验证哪种方式有效。
#[tauri::command]
pub fn preview_click(state: State<'_, AppState>) -> Result<(), String> {
    if state.engine.is_active() {
        return Err("正在运行，先停止再测试".into());
    }
    let cfg = state.config_snapshot();
    let point = cfg.point;
    let mode = cfg.click_mode;
    let hold = cfg.hold_ms;
    log::info(&format!(
        "测试点击：point=({}, {}) mode={:?}",
        point.x, point.y, mode
    ));
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(2));
        // 用户正按着鼠标时不插这一刀，避免打断他的拖动 / 手势。
        if crate::input::user_holding_button() {
            log::info("测试点击取消：检测到鼠标按键正被按住");
            return;
        }
        // 与连点走同一条注入路径：按下时长同样钳到 ≤4ms，
        // 否则「测试一次点击」能响应、连点却点不动，等于给自己挖坑。
        let ok = crate::engine::do_click(mode, point.x, point.y, hold, false, 0);
        log::info(&format!("测试点击完成：{}", if ok { "已发送" } else { "发送失败" }));
    });
    Ok(())
}

/// 2 秒后读取鼠标所指窗口，作为固定点击目标。
#[tauri::command]
pub async fn pick_target_window(app: AppHandle, state: State<'_, AppState>) -> Result<Option<String>, String> {
    let (x, y) = {
        std::thread::sleep(std::time::Duration::from_millis(200));
        crate::input::cursor_pos()
    };
    let hwnd = crate::input::window_from_point(x, y);
    if hwnd == 0 {
        return Err("鼠标下方没有可用窗口".into());
    }
    let (pid, title) = crate::input::window_info(hwnd);
    if pid == crate::input::current_pid() {
        return Err("不要把 Tapper 自己的窗口设为目标".into());
    }
    {
        let mut cfg = cfg_lock(&state);
        cfg.target_hwnd = Some(hwnd as i64);
        let snapshot = cfg.clone();
        drop(cfg);
        save(&app, &snapshot);
    }
    Ok(Some(title))
}

#[tauri::command]
pub fn set_theme(app: AppHandle, state: State<'_, AppState>, theme: Theme) -> Result<(), String> {
    let mut cfg = cfg_lock(&state);
    cfg.theme = theme;
    let snapshot = cfg.clone();
    drop(cfg);
    save(&app, &snapshot);
    Ok(())
}

#[tauri::command]
pub fn set_mode(app: AppHandle, state: State<'_, AppState>, mode: RunMode) -> Result<(), String> {
    if state.engine.is_active() {
        return Err("运行中不能切换模式".into());
    }
    {
        let mut cfg = cfg_lock(&state);
        cfg.mode = mode;
        let snapshot = cfg.clone();
        drop(cfg);
        save(&app, &snapshot);
    }
    state.refresh_hit_zones();
    Ok(())
}

/// 切换悬浮层所在显示器（面板「点击目标」右上角的下拉，值是显示器列表里的序号）。
#[tauri::command]
pub fn set_monitor(
    app: AppHandle,
    state: State<'_, AppState>,
    index: usize,
) -> Result<(), String> {
    switch_monitor_internal(&app, &state, index, SwitchCause::Panel)
}

/// 这次换屏是被谁触发的 —— 决定换完之后谁需要重新安置、谁由拖动自己定。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwitchCause {
    /// 面板「屏幕」下拉 / 重新显示悬浮层：两个元素都不在手里，标记跑出画面就居中，
    /// 状态条按新屏换算（放得下就保留用户摆好的位置）。
    Panel,
    /// 正在拖悬浮标记：标记的落点紧接着由这次拖动写，别中途改写；
    /// 状态条回到新屏**顶部居中**（它的偏移是相对旧屏顶部中心的，换个屏留着也没意义）。
    MarkerDrag,
    /// 正在拖顶部状态条：状态条的落点由拖动自己定；标记没人管，跑出画面就放新屏**正中**。
    HudDrag,
}

/// 换屏：面板下拉与「把标记 / 状态条拖到屏幕边缘」共用。
pub fn switch_monitor_internal(
    app: &AppHandle,
    state: &AppState,
    index: usize,
    cause: SwitchCause,
) -> Result<(), String> {
    {
        let mut cfg = cfg_lock(state);
        cfg.monitor_index = Some(index);
        let snapshot = cfg.clone();
        drop(cfg);
        save(app, &snapshot);
    }
    if state.overlay_mode() != OverlayMode::Hidden {
        let cfg = state.config_snapshot();
        let (geometry, monitors) = overlay::open(app, cfg.monitor_index)?;
        *state
            .overlay_geometry
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = Some(geometry);
        *state.monitors.lock().unwrap_or_else(|p| p.into_inner()) = monitors;
        state.refresh_hit_zones();
        // 悬浮层前端是 150ms 轮询取几何的，跨屏瞬间主动催一次，免得标记按旧屏幕换算跑出画面。
        let _ = app.emit(crate::overlay_input::EVT_GEOMETRY, ());
        if let Err(err) = confine_to_monitor(app, state, geometry, cause) {
            // 安置失败不拦换屏：悬浮层已经在新屏上了，最多是标记位置不理想。
            log::error(&format!("换屏后安置标记/状态条失败：{err}"));
        }
    }
    Ok(())
}

/// 换屏后把标记和状态条安置进新屏幕 —— 谁复位看 `cause`，见 [`SwitchCause`]。
///
/// 两个元素的坐标都还是上一块屏幕的，切过去了却什么都看不见：
/// 标记是整屏绝对坐标，跑出去了直接放新屏正中；
/// 状态条存的是「相对本屏顶部中心的偏移」，换屏后这个基准就没了，
/// 所以要么由拖动重新算（拖它时）、要么回新屏顶部居中的默认位（拖标记时）、
/// 要么按新屏换算后尽量保留（下拉换屏时）。
fn confine_to_monitor(
    app: &AppHandle,
    state: &AppState,
    geo: OverlayGeometry,
    cause: SwitchCause,
) -> Result<(), String> {
    let monitor = state
        .monitors_snapshot()
        .into_iter()
        .nth(state.current_monitor_index())
        .unwrap_or_else(|| overlay::monitor_of(geo));

    if cause != SwitchCause::MarkerDrag {
        let point = state.point();
        let placed = overlay::marker_after_switch(&monitor, point.x, point.y);
        if placed != (point.x, point.y) {
            set_point_internal(app, state, placed.0, placed.1)?;
        }
    }

    match cause {
        SwitchCause::MarkerDrag => {
            let [dx, dy] = state.hud_offset().unwrap_or([0, 0]);
            if dx != 0 || dy != 0 {
                set_hud_offset_internal(app, state, 0, 0)?;
            }
        }
        SwitchCause::Panel | SwitchCause::HudDrag => {
            let (base_x, base_y) = overlay::hud_base(&monitor);
            let [dx, dy] = state.hud_offset().unwrap_or([0, 0]);
            let kept =
                overlay::hud_offset_keep_within(&monitor, base_x, base_y, dx, dy, overlay::HUD_MARGIN);
            if kept != (dx, dy) {
                set_hud_offset_internal(app, state, kept.0, kept.1)?;
            }
        }
    }
    Ok(())
}

/// 保存顶部状态条的位置偏移（物理像素，相对默认位置）。
#[tauri::command]
pub fn set_hud_offset(
    app: AppHandle,
    state: State<'_, AppState>,
    dx: i32,
    dy: i32,
) -> Result<(), String> {
    set_hud_offset_internal(&app, &state, dx, dy)
}

/// 保存顶部状态条的位置偏移（命令与悬浮层拖动共用）。
pub fn set_hud_offset_internal(
    app: &AppHandle,
    state: &AppState,
    dx: i32,
    dy: i32,
) -> Result<(), String> {
    {
        let mut cfg = cfg_lock(state);
        cfg.hud_offset = Some([dx, dy]);
        let snapshot = cfg.clone();
        drop(cfg);
        save(app, &snapshot);
    }
    state.refresh_hit_zones();
    Ok(())
}

/// 重置顶部状态条到默认位置。
#[tauri::command]
pub fn reset_hud_offset(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    reset_hud_offset_internal(&app, &state)
}

/// 重置顶部状态条到默认位置（命令与悬浮层菜单共用）。
pub fn reset_hud_offset_internal(app: &AppHandle, state: &AppState) -> Result<(), String> {
    {
        let mut cfg = cfg_lock(state);
        cfg.hud_offset = None;
        let snapshot = cfg.clone();
        drop(cfg);
        save(app, &snapshot);
    }
    state.refresh_hit_zones();
    Ok(())
}

/// 显示 / 隐藏悬浮标记。返回最终状态。
#[tauri::command]
pub fn set_marker_visible(
    app: AppHandle,
    state: State<'_, AppState>,
    visible: bool,
) -> Result<bool, String> {
    {
        let mut cfg = cfg_lock(&state);
        cfg.show_marker = visible;
        let snapshot = cfg.clone();
        drop(cfg);
        save(&app, &snapshot);
    }
    if visible {
        let cfg = state.config_snapshot();
        let (geometry, monitors) = overlay::open(&app, cfg.monitor_index)?;
        *state
            .overlay_geometry
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = Some(geometry);
        *state.monitors.lock().unwrap_or_else(|p| p.into_inner()) = monitors;
        state.set_overlay_mode(OverlayMode::Passive);
        confine_to_monitor(&app, &state, geometry, SwitchCause::Panel)?;
        state.refresh_hit_zones();
    } else {
        overlay::hide(&app);
        state.set_overlay_mode(OverlayMode::Hidden);
    }
    Ok(visible)
}

/// 托盘 / 热键用：翻转标记显示。
pub fn toggle_marker_internal(app: &AppHandle) -> Result<bool, String> {
    let state = app.try_state::<AppState>().ok_or("应用状态未初始化")?;
    let visible = !state.config_snapshot().show_marker;
    set_marker_visible(app.clone(), state, visible)
}

#[tauri::command]
pub fn toggle_marker(app: AppHandle) -> Result<bool, String> {
    toggle_marker_internal(&app)
}

/// 前端上报状态条 / 标记的实际渲染矩形，命中区按实测值生成（消除透明遮罩）。
#[tauri::command]
pub fn set_overlay_rects(state: State<'_, AppState>, rects: OverlayRects) {
    state.set_overlay_rects(rects);
}

#[tauri::command]
pub fn open_panel(app: AppHandle) {
    crate::show_main_window(&app);
}

#[tauri::command]
pub fn refresh_monitors(app: AppHandle, state: State<'_, AppState>) -> Vec<MonitorInfo> {
    let list = overlay::monitors(&app);
    *state.monitors.lock().unwrap_or_else(|p| p.into_inner()) = list.clone();
    list
}

#[tauri::command]
pub fn start_recording(state: State<'_, AppState>) -> Result<(), String> {
    if state.engine.is_active() {
        return Err("点击运行中不能录制".into());
    }
    state.recording.start();
    Ok(())
}

#[tauri::command]
pub fn stop_recording(state: State<'_, AppState>) -> Option<RecordingSummary> {
    state.recording.stop();
    state.recording.summary()
}

#[tauri::command]
pub fn discard_recording(state: State<'_, AppState>) {
    state.recording.stop();
    state.recording.clear();
}

/// 把录制结果应用到单点点位：用最后一次点击的位置与平均节奏。
#[tauri::command]
pub fn apply_recording(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<RecordingSummary, String> {
    if state.engine.is_active() {
        return Err("请先停止点击".into());
    }
    let summary = state.recording.summary().ok_or("没有录到点击")?;
    {
        let mut cfg = cfg_lock(&state);
        cfg.point = ClickPoint {
            x: summary.x,
            y: summary.y,
        };
        cfg.cps = summary.cps;
        let snapshot = cfg.clone();
        drop(cfg);
        save(&app, &snapshot);
    }
    state.refresh_hit_zones();
    state.recording.stop();
    state.recording.clear();
    Ok(summary)
}

/// 把当前录制保存成一份脚本（保留完整坐标序列与节奏），并选中它。
#[tauri::command]
pub fn save_script(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
) -> Result<Script, String> {
    let steps: Vec<ScriptStep> = state.recording.steps();
    if steps.is_empty() {
        return Err("没有可保存的录制内容".into());
    }
    let name = name.trim();
    let name = if name.is_empty() {
        format!("脚本 {}", now_ms() % 10_000)
    } else {
        name.chars().take(24).collect()
    };

    let script = Script {
        id: format!("s{}", now_ms()),
        name,
        steps,
        created_at: now_ms(),
    };

    {
        let mut cfg = cfg_lock(&state);
        cfg.scripts.push(script.clone());
        if cfg.scripts.len() > 30 {
            let overflow = cfg.scripts.len() - 30;
            cfg.scripts.drain(0..overflow);
        }
        cfg.active_script_id = Some(script.id.clone());
        cfg.mode = RunMode::Script;
        let snapshot = cfg.clone();
        drop(cfg);
        save(&app, &snapshot);
    }
    state.refresh_hit_zones();
    state.recording.stop();
    state.recording.clear();
    log::info(&format!("已保存脚本：{}（{} 步）", script.name, script.steps.len()));
    Ok(script)
}

#[tauri::command]
pub fn delete_script(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<(), String> {
    {
        let mut cfg = cfg_lock(&state);
        cfg.scripts.retain(|s| s.id != id);
        if cfg.active_script_id.as_deref() == Some(id.as_str()) {
            cfg.active_script_id = cfg.scripts.first().map(|s| s.id.clone());
        }
        let snapshot = cfg.clone();
        drop(cfg);
        save(&app, &snapshot);
    }
    Ok(())
}

#[tauri::command]
pub fn select_script(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    {
        let mut cfg = cfg_lock(&state);
        if !cfg.scripts.iter().any(|s| s.id == id) {
            return Err("脚本不存在".into());
        }
        cfg.active_script_id = Some(id);
        let snapshot = cfg.clone();
        drop(cfg);
        save(&app, &snapshot);
    }
    Ok(())
}

#[tauri::command]
pub fn rename_script(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    name: String,
) -> Result<(), String> {
    let name: String = name.trim().chars().take(24).collect();
    if name.is_empty() {
        return Err("名称不能为空".into());
    }
    {
        let mut cfg = cfg_lock(&state);
        if let Some(script) = cfg.scripts.iter_mut().find(|s| s.id == id) {
            script.name = name;
        } else {
            return Err("脚本不存在".into());
        }
        let snapshot = cfg.clone();
        drop(cfg);
        save(&app, &snapshot);
    }
    Ok(())
}

#[tauri::command]
pub fn hide_main_window(app: AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.hide();
    }
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    if let Some(state) = app.try_state::<AppState>() {
        state.engine.stop(Some("应用退出"));
    }
    overlay::hide(&app);
    std::thread::sleep(std::time::Duration::from_millis(100));
    app.exit(0);
}

#[tauri::command]
pub fn open_log_dir(app: AppHandle) -> Result<(), String> {
    let path = log::path();
    let Some(dir) = path.parent() else {
        return Err("日志目录不可用".into());
    };
    let _ = std::fs::create_dir_all(dir);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new("explorer.exe")
            .arg(dir)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = dir;
    }
    let _ = app.emit("noop", ());
    Ok(())
}
