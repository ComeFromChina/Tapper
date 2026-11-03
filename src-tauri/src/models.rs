use serde::{Deserialize, Serialize};

/// 点击点（虚拟桌面物理像素，多屏时为绝对坐标）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClickPoint {
    pub x: i32,
    pub y: i32,
}

impl Default for ClickPoint {
    fn default() -> Self {
        Self { x: 0, y: 0 }
    }
}

/// 主题。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Theme {
    Dark,
    Light,
    System,
}

/// 点击注入方式。标准 = SendInput；消息 = 直接投递窗口消息；两者 = 同时发（可能重复）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClickMode {
    Standard,
    Message,
    Both,
}

/// 运行模式：单点 / 脚本。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RunMode {
    Point,
    Script,
}

/// 脚本中的一步：落点 + 与上一步的间隔。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptStep {
    pub x: i32,
    pub y: i32,
    pub gap_ms: u64,
}

/// 一份录制好的脚本。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Script {
    pub id: String,
    pub name: String,
    pub steps: Vec<ScriptStep>,
    /// Unix 毫秒时间戳。
    pub created_at: i64,
}

/// 可持久化的应用配置。运行态（是否正在点击、计数）绝不落盘。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppConfig {
    pub theme: Theme,
    pub mode: RunMode,
    pub point: ClickPoint,
    /// 每秒点击次数（单点模式）。
    pub cps: f64,
    /// 点击注入方式。
    pub click_mode: ClickMode,
    /// 每次按下保持的毫秒数，0 = 瞬时点击。
    pub hold_ms: u32,
    /// 启动倒计时秒数，0 = 立即启动。
    pub countdown_secs: u32,
    /// 鼠标甩到桌面左上角时急停（分屏使用时通常关闭）。
    pub corner_failsafe: bool,
    /// 鼠标移到「另一个窗口」时自动停止。
    pub window_guard: bool,
    /// 是否显示悬浮层点位标记。
    pub show_marker: bool,
    /// 固定点击目标窗口句柄（None = 启动时锁定当时的前台窗口）。
    pub target_hwnd: Option<i64>,
    /// 顶部状态条位置（相对屏幕中心的偏移，物理像素）。None = 默认居中顶部。
    pub hud_offset: Option<[i32; 2]>,
    /// 悬浮层所在显示器在显示器列表里的序号（None = 主显示器）。
    /// 不用名字：Windows 上报的是 DISPLAY1 / DISPLAY65 这种会变、还可能重复或为空的标识，
    /// 按名字挑屏幕经常落回第一块，表现成「选副屏没反应、标记拖不过去」。
    pub monitor_index: Option<usize>,
    /// 已保存的脚本。
    pub scripts: Vec<Script>,
    /// 当前选中的脚本。
    pub active_script_id: Option<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            mode: RunMode::Point,
            point: ClickPoint::default(),
            cps: 4.0,
            click_mode: ClickMode::Standard,
            hold_ms: 0,
            countdown_secs: 3,
            corner_failsafe: false,
            window_guard: true,
            show_marker: true,
            target_hwnd: None,
            hud_offset: None,
            monitor_index: None,
            scripts: Vec::new(),
            active_script_id: None,
        }
    }
}

impl AppConfig {
    pub fn active_script(&self) -> Option<&Script> {
        let id = self.active_script_id.as_ref()?;
        self.scripts.iter().find(|s| &s.id == id)
    }
}

/// 引擎阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Idle,
    Countdown,
    Running,
}

impl Phase {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Phase::Countdown,
            2 => Phase::Running,
            _ => Phase::Idle,
        }
    }
}

/// 目标窗口（启动时锁定）。
#[derive(Debug, Clone)]
pub struct TargetWindow {
    pub hwnd: isize,
    pub title: String,
}

/// 一次录制到的点击。
#[derive(Debug, Clone)]
pub struct RecordedClick {
    /// 相对上一次点击（或录制开始）的毫秒间隔。
    pub gap_ms: u64,
    pub x: i32,
    pub y: i32,
}

/// 启动计划：由命令层按当前模式组装。
#[derive(Debug, Clone)]
pub enum StartPlan {
    Point { x: i32, y: i32, cps: f64, hold_ms: u32 },
    Script { steps: Vec<ScriptStep>, hold_ms: u32 },
}

/// 前端展示用：引擎状态快照。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    pub phase: Phase,
    pub countdown: u32,
    pub clicks: u64,
    pub elapsed_ms: u64,
    pub actual_cps: f64,
    pub stop_reason: Option<String>,
    pub target_title: Option<String>,
    /// 脚本模式：当前步（0 起）与总步数。
    pub step_index: u32,
    pub step_total: u32,
}

/// 前端展示用：录制摘要。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingSummary {
    pub count: usize,
    pub avg_gap_ms: f64,
    pub cps: f64,
    pub x: i32,
    pub y: i32,
    pub spread_px: i32,
    /// 录到的点击序列（保存脚本 / 预览用）。
    pub steps: Vec<ScriptStep>,
}

/// 前端展示用：显示器。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorInfo {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f64,
    pub is_primary: bool,
}

/// 悬浮层几何信息（物理像素 + 缩放）。
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayGeometry {
    pub origin_x: i32,
    pub origin_y: i32,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f64,
}

/// 前端上报的可交互元素视觉矩形（屏幕绝对物理像素，中心 + 半宽半高）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayRect {
    pub x: i32,
    pub y: i32,
    pub half_w: i32,
    pub half_h: i32,
}

/// 悬浮层里菜单项的实测矩形（物理像素，中心 + 半宽半高）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayMenuItem {
    pub id: String,
    pub x: i32,
    pub y: i32,
    pub half_w: i32,
    pub half_h: i32,
}

/// 悬浮层里可交互元素的实测矩形，用来让命中区和渲染严格一致。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayRects {
    pub hud: Option<OverlayRect>,
    pub marker: Option<OverlayRect>,
    #[serde(default)]
    pub menu_items: Vec<OverlayMenuItem>,
}

/// 拖动标记时的预览位置（物理像素）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayDragEvent {
    pub x: i32,
    pub y: i32,
}

/// 拖动状态条时的预览偏移（物理像素）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayHudDragEvent {
    pub dx: i32,
    pub dy: i32,
}

/// 右键菜单位置（物理像素）与来源（"marker" = 悬浮标记，"hud" = 顶部状态条）。
/// 菜单项由前端按来源决定，Rust 侧只做命中测试。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayMenuEvent {
    pub x: i32,
    pub y: i32,
    pub source: String,
}

/// 鼠标当前悬停在悬浮层的哪个元素上（None = 没有悬停）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayHoverEvent {
    pub region: Option<String>,
    /// 状态条当前是否被按下（用于按下时的视觉反馈）。
    pub pressed: bool,
}

/// 当前悬停的菜单项（None = 没有悬停项）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayMenuHoverEvent {
    pub id: Option<String>,
}

/// 一次性返回给前端的完整状态。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStateDto {
    pub config: AppConfig,
    pub engine: EngineStatus,
    pub recording: Option<RecordingSummary>,
    pub is_recording: bool,
    pub overlay_mode: OverlayMode,
    pub overlay: Option<OverlayGeometry>,
    pub monitors: Vec<MonitorInfo>,
    pub failed_hotkeys: Vec<String>,
    pub elevated: bool,
    pub notice: Option<String>,
    pub version: String,
    /// 应用名与简介（关于面板展示）。
    pub app_name: String,
    pub description: String,
    /// 可执行文件所在目录（手动启动/排障时提示用）。
    pub exe_dir: String,
    pub log_path: String,
}

/// 悬浮层窗口专用状态（轻量，避免与主面板争抢 notice）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayStateDto {
    pub config: AppConfig,
    pub engine: EngineStatus,
    pub mode: OverlayMode,
    pub geometry: Option<OverlayGeometry>,
}

/// 悬浮层模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OverlayMode {
    Hidden,
    /// 穿透显示；空闲时鼠标移到标记上即可拖动（Rust 侧命中测试切换）。
    Passive,
}
