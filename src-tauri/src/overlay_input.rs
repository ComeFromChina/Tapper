//! 悬浮层的鼠标交互全部在 Rust 侧完成。
//!
//! 为什么不能交给网页：悬浮层是一个铺满屏幕的 WebView2 窗口，网页里的透明区域
//! 并不会自动穿透 —— 只要这个窗口开始接收鼠标，整块屏幕的点击都会被它吞掉，
//! 状态条周围就会出现一块看不见却点不动的遮罩。
//! 所以悬浮层窗口永远保持 click-through，交互改成在全局鼠标钩子里做命中测试：
//! 命中悬浮层元素时由钩子吞掉这次点击（游戏收不到），其余情况一律放行给游戏。
//!
//! 命中区用前端实测的渲染矩形（物理像素），保证「元素多大就占多大空间」。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};
use windows::Win32::Foundation::{LPARAM, POINT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
use windows::Win32::UI::WindowsAndMessaging::{
    LoadCursorW, SendMessageW, SetCursor, WindowFromPoint, HCURSOR, IDC_HAND, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETCURSOR,
};

use crate::log;
use crate::overlay;
use crate::models::{
    MonitorInfo, OverlayDragEvent, OverlayHudDragEvent, OverlayHoverEvent, OverlayMenuEvent,
    OverlayMenuHoverEvent, OverlayMode,
};
use crate::state::{AppState, HitZoneKind};

pub const EVT_MARKER_DRAG: &str = "overlay-marker-drag";
pub const EVT_HUD_DRAG: &str = "overlay-hud-drag";
/// 悬浮层换屏后催前端立刻重新取几何（默认 150ms 才轮询一次，跨屏瞬间会按旧屏幕换算）。
pub const EVT_GEOMETRY: &str = "overlay-geometry-changed";
pub const EVT_MENU: &str = "overlay-context-menu";
pub const EVT_MENU_CLOSE: &str = "overlay-context-menu-close";
pub const EVT_MENU_HOVER: &str = "overlay-menu-hover";
pub const EVT_HOVER: &str = "overlay-hover";

/// 双击判定：两次按下的间隔上限（跟系统设置走，但不低于这个值），
/// 位移上限按显示缩放换算，给手抖留余量。
const DOUBLE_CLICK_MIN_MS: u32 = 500;
const DOUBLE_CLICK_CSS_PX: f64 = 24.0;
/// 拖动判定阈值（物理像素）。
const DRAG_THRESHOLD: i32 = 4;
/// 拖动预览的最小推送间隔，避免高刷鼠标刷爆 IPC。
const PREVIEW_INTERVAL_MS: u64 = 16;
/// 光标轮询间隔：Windows 只在鼠标移动时才会重置光标，
/// 所以指针停在状态条上时需要持续把系统光标按回手型。
const CURSOR_POLL_MS: u64 = 8;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Region {
    Hud,
    Marker,
    Menu,
}

/// 一次按下-拖动-抬起的过程。
struct Interaction {
    region: Region,
    menu_id: Option<String>,
    /// 按下位置（物理像素）。
    start_x: i32,
    start_y: i32,
    moved: bool,
    /// 拖动锚点（物理像素绝对坐标）：标记 = 落点本身，状态条 = 按下那一刻整条的中心。
    ///
    /// 用绝对坐标而不是「相对本屏顶部中心的偏移」，是因为偏移换屏就没意义了：
    /// 每块屏的宽度、缩放都不一样，而拖动要跟住手里的光标。
    base_x: i32,
    base_y: i32,
    /// 按下那一刻引擎是否在跑。启停只在**抬起**时判定，见 [`hud_gesture`]。
    active_at_press: bool,
    /// 这次拖动已经请求切到哪块屏了。钩子每帧都会重新判定落点，不记一下就会在
    /// 工作线程真正切过去之前重复推送，悬浮层被反复重建。
    switched_to: Option<usize>,
    last_preview: Instant,
}

struct OverlayInput {
    interaction: Mutex<Option<Interaction>>,
    /// 是否有正在进行的按下过程（避免每帧都去抢锁）。
    dragging: AtomicBool,
    /// 上一次按下标记的时间与位置，用于双击判定。
    last_marker_press: Mutex<Option<(Instant, i32, i32)>>,
    menu_open: AtomicBool,
    hover_item: Mutex<Option<String>>,
    swallow_left: AtomicBool,
    swallow_right: AtomicBool,
}

static INPUT: OnceLock<OverlayInput> = OnceLock::new();

fn input() -> &'static OverlayInput {
    INPUT.get_or_init(|| OverlayInput {
        interaction: Mutex::new(None),
        dragging: AtomicBool::new(false),
        last_marker_press: Mutex::new(None),
        menu_open: AtomicBool::new(false),
        hover_item: Mutex::new(None),
        swallow_left: AtomicBool::new(false),
        swallow_right: AtomicBool::new(false),
    })
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

/// 双击允许的最大间隔：取系统的双击时间设置，太快就按 500ms 兜底。
fn double_click_ms() -> u64 {
    let system = unsafe { GetDoubleClickTime() };
    system.max(DOUBLE_CLICK_MIN_MS) as u64
}

/// 双击允许的最大位移（物理像素）：按显示缩放换算 CSS 像素。
fn double_click_px(state: &AppState) -> i32 {
    (DOUBLE_CLICK_CSS_PX * state.overlay_scale()).round() as i32
}

/// 需要在钩子之外执行的动作（钩子回调里绝不能做耗时操作）。
enum Action {
    StartEngine,
    StopEngine(String),
    SetPoint(i32, i32),
    SetHudOffset(i32, i32),
    ResetHudOffset,
    /// 拖到屏幕边缘之外：整块悬浮层搬到那块屏。
    ///
    /// `cause` 记录这次是拖着谁过去的，换完屏后「没被拖的那个」按它复位：
    /// 拖标记 → 状态条回新屏顶部居中；拖状态条 → 标记放新屏正中。
    SwitchMonitor {
        index: usize,
        cause: crate::commands::SwitchCause,
    },
    SetPointAtCursor,
    ToggleMarker,
    OpenPanel,
}

static ACTION_TX: OnceLock<mpsc::Sender<Action>> = OnceLock::new();

fn push(action: Action) {
    if let Some(tx) = ACTION_TX.get() {
        let _ = tx.send(action);
    }
}

/// 启动悬浮层的动作线程与光标线程。
pub fn start(app: AppHandle) {
    let (tx, rx) = mpsc::channel::<Action>();
    let _ = ACTION_TX.set(tx);
    let worker = app.clone();
    std::thread::Builder::new()
        .name("overlay-actions".into())
        .spawn(move || {
            while let Ok(action) = rx.recv() {
                run_action(&worker, action);
            }
        })
        .ok();

    std::thread::Builder::new()
        .name("overlay-cursor".into())
        .spawn(move || cursor_watcher(app))
        .ok();
}

fn run_action(app: &AppHandle, action: Action) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    match action {
        Action::StartEngine => {
            if let Err(msg) = crate::commands::start_engine_internal(app) {
                crate::commands::set_notice(app, &msg);
                crate::show_main_window(app);
            }
        }
        // 引擎已经停了（比如中途被遮挡自动停止）时不用再停一次，静默跳过即可。
        Action::StopEngine(reason) => state.engine.stop(Some(&reason)),
        Action::SetPoint(x, y) => {
            let _ = crate::commands::set_point_internal(app, &state, x, y);
        }
        Action::SetHudOffset(dx, dy) => {
            let _ = crate::commands::set_hud_offset_internal(app, &state, dx, dy);
        }
        Action::ResetHudOffset => {
            let _ = crate::commands::reset_hud_offset_internal(app, &state);
        }
        Action::SwitchMonitor { index, cause } => {
            if let Err(msg) = crate::commands::switch_monitor_internal(app, &state, index, cause) {
                log::error(&format!("拖到边缘切屏失败：{msg}"));
            }
        }
        Action::SetPointAtCursor => {
            let _ = crate::commands::set_point_at_cursor_internal(app, &state);
        }
        Action::ToggleMarker => {
            if let Err(msg) = crate::commands::toggle_marker_internal(app) {
                crate::commands::set_notice(app, &msg);
            }
        }
        Action::OpenPanel => crate::show_main_window(app),
    }
}

/// 处理全局鼠标事件。返回 true 表示这次事件被悬浮层吃掉，游戏不该再收到。
pub fn handle_mouse(app: &AppHandle, message: u32, pt: POINT) -> bool {
    match message {
        WM_MOUSEMOVE => {
            on_move(app, pt.x, pt.y);
            false
        }
        WM_LBUTTONDOWN => on_left_down(app, pt.x, pt.y),
        WM_LBUTTONUP => on_left_up(app, pt.x, pt.y),
        WM_RBUTTONDOWN => on_right_down(app, pt.x, pt.y),
        WM_RBUTTONUP => input().swallow_right.swap(false, Ordering::SeqCst),
        _ => false,
    }
}

fn on_left_down(app: &AppHandle, x: i32, y: i32) -> bool {
    let Some(state) = app.try_state::<AppState>() else {
        return false;
    };

    // 菜单打开时：点在菜单项上 → 记下来并吞掉；点在菜单外 → 关掉菜单，放行给游戏。
    if input().menu_open.load(Ordering::SeqCst) {
        if let Some(id) = state.menu_item_at(x, y) {
            *lock(&input().interaction) = Some(Interaction {
                region: Region::Menu,
                menu_id: Some(id),
                start_x: x,
                start_y: y,
                moved: false,
                base_x: 0,
                base_y: 0,
                active_at_press: false,
                switched_to: None,
                last_preview: Instant::now(),
            });
            input().dragging.store(true, Ordering::SeqCst);
            input().swallow_left.store(true, Ordering::SeqCst);
            return true;
        }
        close_menu(app);
        // 菜单开着时点标记：关掉菜单后继续走标记的逻辑（顺手拖动标记），
        // 点在别处则当作「点菜单外面」，关掉菜单并把这次点击放行给游戏。
        if state.region_at(x, y) != Some(HitZoneKind::Marker) {
            return false;
        }
    }

    // 双击判定放在命中测试之前：第一次按下如果带着轻微抖动，标记已经被拖动过，
    // 前端上报的命中矩形还没更新，第二次按下按位置找命中区就会落空。
    {
        let now = Instant::now();
        let mut last = lock(&input().last_marker_press);
        let limit = double_click_px(&state);
        let is_double = match *last {
            Some((t, px, py)) => {
                now.duration_since(t) < Duration::from_millis(double_click_ms())
                    && (px - x).abs() <= limit
                    && (py - y).abs() <= limit
            }
            None => false,
        };
        if is_double {
            *last = None;
            drop(last);
            // 双击是明确意图，不管引擎在不在跑都要把主面板叫出来。
            input().swallow_left.store(true, Ordering::SeqCst);
            log::info("双击标记 → 打开主面板");
            push(Action::OpenPanel);
            return true;
        }
    }

    match state.region_at(x, y) {
        Some(HitZoneKind::Hud) => {
            input().swallow_left.store(true, Ordering::SeqCst);
            input().dragging.store(true, Ordering::SeqCst);
            // 锚点 = 状态条此刻的绝对中心，之后一律按「跟住光标」换算偏移，换屏也不跳。
            let (ax, ay) = state.hud_anchor().unwrap_or((x, y));
            // 按下**不**停引擎：启停要等抬起时看这次到底是「点一下」还是「拖一把」，见 [`hud_gesture`]。
            *lock(&input().interaction) = Some(Interaction {
                region: Region::Hud,
                menu_id: None,
                start_x: x,
                start_y: y,
                moved: false,
                base_x: ax,
                base_y: ay,
                active_at_press: state.engine.is_active(),
                switched_to: None,
                last_preview: Instant::now(),
            });
            true
        }
        Some(HitZoneKind::Marker) => {
            // 记下这次按下，第二次按下时（见函数开头）就认成双击。
            *lock(&input().last_marker_press) = Some((Instant::now(), x, y));

            if !state.marker_editable() {
                // 脚本模式 / 运行中：标记只是指示器，点击照旧落到游戏上。
                return false;
            }

            let point = state.point();
            *lock(&input().interaction) = Some(Interaction {
                region: Region::Marker,
                menu_id: None,
                start_x: x,
                start_y: y,
                moved: false,
                base_x: point.x,
                base_y: point.y,
                active_at_press: false,
                switched_to: None,
                last_preview: Instant::now(),
            });
            input().dragging.store(true, Ordering::SeqCst);
            input().swallow_left.store(true, Ordering::SeqCst);
            true
        }
        None => false,
    }
}

fn on_move(app: &AppHandle, x: i32, y: i32) {
    let menu_open = input().menu_open.load(Ordering::SeqCst);
    if !menu_open && !input().dragging.load(Ordering::Relaxed) {
        return;
    }

    if menu_open {
        if let Some(state) = app.try_state::<AppState>() {
            let id = state.menu_item_at(x, y);
            let mut hover = lock(&input().hover_item);
            if *hover != id {
                *hover = id.clone();
                drop(hover);
                let _ = app.emit(EVT_MENU_HOVER, OverlayMenuHoverEvent { id });
            }
        }
    }

    let mut guard = lock(&input().interaction);
    let Some(i) = guard.as_mut() else {
        return;
    };
    if !i.moved {
        if !is_drag(x - i.start_x, y - i.start_y) {
            return;
        }
        i.moved = true;
    }
    if i.last_preview.elapsed() < Duration::from_millis(PREVIEW_INTERVAL_MS) {
        return;
    }
    i.last_preview = Instant::now();
    match i.region {
        Region::Marker => {
            let Some((nx, ny, switch)) =
                marker_drag(app, i.base_x + (x - i.start_x), i.base_y + (y - i.start_y))
            else {
                return;
            };
            if let Some(index) = switch {
                if i.switched_to != Some(index) {
                    i.switched_to = Some(index);
                    // 拖的是标记本身：落点紧接着由这次拖动定；状态条回新屏顶部居中。
                    push(Action::SwitchMonitor {
                        index,
                        cause: crate::commands::SwitchCause::MarkerDrag,
                    });
                }
            }
            let _ = app.emit(EVT_MARKER_DRAG, OverlayDragEvent { x: nx, y: ny });
        }
        Region::Hud => {
            let Some((dx, dy, switch)) =
                hud_drag(app, i.base_x + (x - i.start_x), i.base_y + (y - i.start_y))
            else {
                return;
            };
            if let Some(index) = switch {
                if i.switched_to != Some(index) {
                    i.switched_to = Some(index);
                    // 拖的是状态条：标记没人管，换屏时安置到那块屏正中，不然它留在旧屏看不见。
                    push(Action::SwitchMonitor {
                        index,
                        cause: crate::commands::SwitchCause::HudDrag,
                    });
                }
            }
            let _ = app.emit(EVT_HUD_DRAG, OverlayHudDragEvent { dx, dy });
        }
        Region::Menu => {}
    }
}

/// 按下之后位移到这儿就算「拖动」了（物理像素，曼哈顿距离）。
///
/// 单击时手总会抖几像素，阈值以下仍按单击处理；状态条的启停判定就靠它区分「点一下」和「挪位置」。
fn is_drag(dx: i32, dy: i32) -> bool {
    dx.abs() + dy.abs() >= DRAG_THRESHOLD
}

/// 状态条一次「按下 → 抬起」到底是什么意思。
///
/// 以前运行中一按下去就停（不等抬起）：想把状态条挪个位置，引擎就被当成「单击停止」停掉了，
/// 拖完还得重新开 —— 用户报「我长按只是改变位置，并不是要停」。
/// 现在按下只记下「那一刻引擎在不在跑」，等抬起再看有没有拖过：
/// 拖过 → 只落新位置，引擎照跑；没拖过 → 才按按下那一刻的状态启动 / 停止。
///
/// 按那一刻而不是抬起时看引擎状态，是为了避免「点下去想停，结果中途它自己因遮挡停了，
/// 抬起反倒被重新开起来」。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum HudGesture {
    Start,
    Stop,
    Reposition,
}

fn hud_gesture(moved: bool, active_at_press: bool) -> HudGesture {
    if moved {
        HudGesture::Reposition
    } else if active_at_press {
        HudGesture::Stop
    } else {
        HudGesture::Start
    }
}

fn on_left_up(app: &AppHandle, x: i32, y: i32) -> bool {
    let swallow = input().swallow_left.swap(false, Ordering::SeqCst);
    input().dragging.store(false, Ordering::SeqCst);
    let Some(i) = lock(&input().interaction).take() else {
        return swallow;
    };

    match i.region {
        Region::Hud => match hud_gesture(i.moved, i.active_at_press) {
            HudGesture::Reposition => {
                let want_x = i.base_x + (x - i.start_x);
                let want_y = i.base_y + (y - i.start_y);
                // 状态拿不到就不落新位置，维持服务端已有偏移，别把状态条甩回默认位置。
                if let Some((dx, dy, switch)) = hud_drag(app, want_x, want_y) {
                    // 切屏要排在写偏移之前：偏移是相对新屏幕顶部中心的。
                    if let Some(index) = switch {
                        push(Action::SwitchMonitor {
                            index,
                            cause: crate::commands::SwitchCause::HudDrag,
                        });
                    }
                    push(Action::SetHudOffset(dx, dy));
                }
            }
            HudGesture::Stop => push(Action::StopEngine("单击状态条停止".into())),
            HudGesture::Start => push(Action::StartEngine),
        },
        Region::Marker => {
            if i.moved {
                let want_x = i.base_x + (x - i.start_x);
                let want_y = i.base_y + (y - i.start_y);
                if let Some((nx, ny, switch)) = marker_drag(app, want_x, want_y) {
                    if let Some(index) = switch {
                        push(Action::SwitchMonitor {
                            index,
                            cause: crate::commands::SwitchCause::MarkerDrag,
                        });
                    }
                    push(Action::SetPoint(nx, ny));
                }
            }
        }
        Region::Menu => {
            let still_on_item = match (&i.menu_id, app.try_state::<AppState>()) {
                (Some(id), Some(state)) => state.menu_item_at(x, y).as_deref() == Some(id.as_str()),
                _ => false,
            };
            log::info(&format!("悬浮层菜单按下项={:?} 抬起仍命中={still_on_item}", i.menu_id));
            if still_on_item {
                if let Some(id) = i.menu_id.as_deref().and_then(menu_action) {
                    push(id);
                }
            }
            close_menu(app);
        }
    }
    swallow
}

fn on_right_down(app: &AppHandle, x: i32, y: i32) -> bool {
    if input().menu_open.load(Ordering::SeqCst) {
        close_menu(app);
        input().swallow_right.store(true, Ordering::SeqCst);
        return true;
    }
    let Some(state) = app.try_state::<AppState>() else {
        return false;
    };
    // 标记和状态条各弹自己的菜单，菜单项由前端按来源决定。
    let source = match state.region_at(x, y) {
        Some(HitZoneKind::Marker) => "marker",
        Some(HitZoneKind::Hud) => "hud",
        None => return false,
    };
    input().menu_open.store(true, Ordering::SeqCst);
    input().swallow_right.store(true, Ordering::SeqCst);
    let _ = app.emit(
        EVT_MENU,
        OverlayMenuEvent {
            x,
            y,
            source: source.to_string(),
        },
    );
    true
}

fn close_menu(app: &AppHandle) {
    if input().menu_open.swap(false, Ordering::SeqCst) {
        *lock(&input().hover_item) = None;
        let _ = app.emit(EVT_MENU_CLOSE, ());
    }
}

fn menu_action(id: &str) -> Option<Action> {
    match id {
        "panel" => Some(Action::OpenPanel),
        "cursor" => Some(Action::SetPointAtCursor),
        "hud-reset" => Some(Action::ResetHudOffset),
        "hide-marker" => Some(Action::ToggleMarker),
        _ => None,
    }
}

/// 拖动标记：想要的落点 → 该落在哪块屏的哪个坐标，以及要不要整层切过去。
fn marker_drag(app: &AppHandle, want_x: i32, want_y: i32) -> Option<(i32, i32, Option<usize>)> {
    let (monitor, switch) = drag_monitor(app, want_x, want_y)?;
    let (x, y) = overlay::clamp_point_in(&monitor, want_x, want_y, overlay::POINT_MARGIN);
    Some((x, y, switch))
}

/// 拖动状态条：想要的中心点 → 相对所在屏顶部中心的偏移，以及要不要整层切过去。
fn hud_drag(app: &AppHandle, want_x: i32, want_y: i32) -> Option<(i32, i32, Option<usize>)> {
    let (monitor, switch) = drag_monitor(app, want_x, want_y)?;
    let (dx, dy) = overlay::hud_offset_in(&monitor, want_x, want_y, overlay::HUD_MARGIN);
    Some((dx, dy, switch))
}

/// 这次拖动落在哪块屏上。落点进了别的显示器就带上要切过去的序号，
/// 落在两块屏之间的空隙（对角摆放、有留白）则留在当前屏。
fn drag_monitor(app: &AppHandle, x: i32, y: i32) -> Option<(MonitorInfo, Option<usize>)> {
    let state = app.try_state::<AppState>()?;
    let list = state.monitors_snapshot();
    let current = state.current_monitor_index();
    if list.is_empty() {
        let geo = *state
            .overlay_geometry
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        return Some((overlay::monitor_of(geo?), None));
    }
    let index = overlay::monitor_for(&list, current, x, y);
    let monitor = list.get(index).cloned()?;
    Some((monitor, (index != current).then_some(index)))
}

/// 让鼠标停在悬浮层可交互元素上时显示手型光标。
///
/// Windows 只在鼠标移动时才会按目标窗口的类光标重置一次，
/// 所以这里持续轮询并把系统光标按回手型；离开时让下层窗口自己把光标恢复回去
/// （直接设成箭头会覆盖游戏自己的光标）。
fn cursor_watcher(app: AppHandle) {
    // HCURSOR 内部是裸指针，不能放进全局静态，这里缓存成整数。
    let hand: isize = unsafe {
        match LoadCursorW(None, IDC_HAND) {
            Ok(h) => h.0 as isize,
            Err(_) => 0,
        }
    };
    let mut showing_hand = false;
    let mut hover_region: Option<&'static str> = None;
    let mut hover_pressed = false;
    loop {
        std::thread::sleep(Duration::from_millis(CURSOR_POLL_MS));
        let Some(state) = app.try_state::<AppState>() else {
            continue;
        };
        let hidden = state.overlay_mode() == OverlayMode::Hidden;
        if hidden {
            if showing_hand {
                restore_cursor();
                showing_hand = false;
            }
            if hover_region.is_some() || hover_pressed {
                hover_region = None;
                hover_pressed = false;
                let _ = app.emit(EVT_HOVER, OverlayHoverEvent { region: None, pressed: false });
            }
            continue;
        }
        let (cx, cy) = crate::input::cursor_pos();
        let in_menu = state.menu_item_at(cx, cy).is_some();
        let region = if in_menu {
            Some("menu")
        } else {
            match state.interactive_region_at(cx, cy) {
                Some(HitZoneKind::Hud) => Some("hud"),
                Some(HitZoneKind::Marker) => Some("marker"),
                None => None,
            }
        };
        let pressed = lock(&input().interaction)
            .as_ref()
            .map(|i| i.region == Region::Hud)
            .unwrap_or(false);
        if region != hover_region || pressed != hover_pressed {
            hover_region = region;
            hover_pressed = pressed;
            let _ = app.emit(
                EVT_HOVER,
                OverlayHoverEvent {
                    region: region.map(|r| r.to_string()),
                    pressed,
                },
            );
        }
        if region.is_some() && hand != 0 {
            set_cursor(hand);
            showing_hand = true;
        } else if showing_hand {
            restore_cursor();
            showing_hand = false;
        }
    }
}

/// 把系统光标按成指定形状（raw = 0 表示标准箭头）。
fn set_cursor(raw: isize) {
    unsafe {
        let cursor = if raw == 0 {
            None
        } else {
            Some(HCURSOR(raw as *mut core::ffi::c_void))
        };
        let _ = SetCursor(cursor);
    }
}

/// 让鼠标下方的窗口自己把光标恢复成它该有的样子，避免把游戏的光标改成箭头。
fn restore_cursor() {
    unsafe {
        let (x, y) = crate::input::cursor_pos();
        let hwnd = WindowFromPoint(POINT { x, y });
        if hwnd.0.is_null() {
            set_cursor(0);
            return;
        }
        let _ = SendMessageW(
            hwnd,
            WM_SETCURSOR,
            Some(WPARAM(hwnd.0 as usize)),
            Some(LPARAM(0)),
        );
    }
}

#[cfg(test)]
mod hud_gesture_tests {
    use super::hud_gesture;
    use super::is_drag;
    use super::HudGesture::{Reposition, Start, Stop};

    #[test]
    fn dragging_only_moves_it() {
        // 运行中长按拖位置：不算停止。
        assert_eq!(hud_gesture(true, true), Reposition);
        assert_eq!(hud_gesture(true, false), Reposition);
    }

    #[test]
    fn clean_tap_toggles_by_the_state_at_press() {
        assert_eq!(hud_gesture(false, true), Stop);
        assert_eq!(hud_gesture(false, false), Start);
    }

    #[test]
    fn a_tap_still_stops_even_if_it_lasted_a_while() {
        // 只要没越过拖动阈值，按多久都还是「单击」—— 时长不参与判定，位置才参与。
        for active in [true, false] {
            assert_eq!(hud_gesture(false, active), if active { Stop } else { Start });
        }
    }

    #[test]
    fn hand_jitter_is_not_a_drag() {
        // 阈值以下：点一下时的手抖仍算单击。
        assert!(!is_drag(0, 0));
        assert!(!is_drag(2, 1));
        assert!(!is_drag(-1, 2));
        // 阈值及以上：挪位置算拖动，引擎不该被停。
        assert!(is_drag(4, 0));
        assert!(is_drag(0, -4));
        assert!(is_drag(2, 2));
        assert!(is_drag(120, 30));
    }
}
