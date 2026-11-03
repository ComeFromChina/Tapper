//! 点击引擎：独立线程调度，停止逻辑不依赖 UI。
//!
//! 安全约定：
//! - 停止只置原子标志 + 递增 generation，旧线程的状态写入都会被 generation 拦截；
//! - 任何退出路径都会补发鼠标抬起，避免长按中途停止把左键按死；
//! - 计数与耗时是运行态，不落盘。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use crate::input;
use crate::log;
use crate::window;
use crate::models::{ClickMode, EngineStatus, Phase, StartPlan, TargetWindow};

const _: () = ();

/// 每轮点击前的检查结果：只看「是否被主动停止」（F7 / 面板 / 托盘）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuardState {
    Ok,
    Stop,
}

/// 落点当前归谁。连点只准打在预期的那个窗口上，别的窗口一律不碰。
#[derive(Debug, PartialEq)]
pub(crate) enum Landing {
    OnTarget,
    /// 被 Tapper 自己的窗口（主面板 / 悬浮层）盖住。
    OwnWindow,
    /// 被别的应用盖住，带着那个窗口的标题。
    Covered(String),
    /// 落点上已经没有窗口（目标窗口被移走或最小化）。
    Empty,
}

/// 这一拍能不能注入。
#[derive(Debug, PartialEq)]
enum Gate {
    Go,
    /// 让路等一拍：用户正按着鼠标，或我们的面板挡住了落点。
    Wait,
    /// 停止并提示。
    Stop(String),
}

/// 落点被自家面板挡住时，最多让路多久（毫秒）。超过就停止并提示，不做无声等待。
const OWN_COVER_STOP_MS: u64 = 5_000;

/// 单点连点的运行参数。
struct PointRun {
    x: i32,
    y: i32,
    cps: f64,
    hold_ms: u32,
}

fn epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 兼容模式「互不干扰」契约的裁决表，输入全是已采样好的值，所以能脱离系统状态单测。
///
/// - 落点被别的应用盖住 / 目标窗口被移走：直接停止并提示。过去的兼容模式是按「此刻谁在
///   落点上」选收件窗口，这正是拖动别的窗口时被误点的根因；
/// - 落点被自家面板盖住：先让路，超过 OWN_COVER_STOP_MS 就停止并提示，不做无声等待；
/// - 目标窗口自己正在被你拖动 / 缩放（`target_moving`）：让路一拍。它在模态移动循环里会把
///   按下当成手势的一部分，而且窗口一动，同一屏幕坐标对应的客户端坐标也变了，容易点错控件；
/// - **你正按着鼠标键（`holding`）：所有方式都让路**，包括不动光标的兼容模式。
///   注入的按下 / 抬起会插进你正在做的手势中间，目标窗口看到「两次按下中间没有抬起」
///   就把你的点击当成一次拖动的延续 —— 表现为「微信会话点不动、窗口也拖不动」。
///   用户自己的鼠标优先，连点最多停一拍（40ms 后重新对齐节奏）。
fn decide(
    holding: bool,
    target_moving: bool,
    landing: &Landing,
    own_cover_ms: u64,
) -> Gate {
    match landing {
        Landing::Empty => Gate::Stop(
            "连点落点上已经没有窗口（目标窗口被移走或最小化），已自动停止，请确认后重新开启"
                .to_string(),
        ),
        Landing::Covered(title) => Gate::Stop(format!(
            "连点落点被其他应用遮挡（{title}），已自动停止，请把该窗口移开后手动重新开启"
        )),
        Landing::OwnWindow => {
            if own_cover_ms >= OWN_COVER_STOP_MS {
                Gate::Stop(
                    "连点落点被 Tapper 自己的面板盖住，已自动停止；请把面板挪开或把标记移到目标窗口上，再重新开启"
                        .to_string(),
                )
            } else {
                Gate::Wait
            }
        }
        Landing::OnTarget => {
            if holding || target_moving {
                Gate::Wait
            } else {
                Gate::Go
            }
        }
    }
}

pub(crate) fn landing_at(x: i32, y: i32, expected: isize) -> Landing {
    let root = window::window_at(x, y);
    if root == 0 {
        return Landing::Empty;
    }
    if root == expected {
        return Landing::OnTarget;
    }
    let (pid, title) = input::window_info(root);
    if pid == input::current_pid() {
        return Landing::OwnWindow;
    }
    let name = {
        let t = title.trim();
        if t.is_empty() {
            "无标题窗口".to_string()
        } else {
            t.to_string()
        }
    };
    Landing::Covered(name)
}

/// 点击引擎本体。调度线程每拍先过 `gate` 再注入，保证只打在预期的窗口上。
pub struct Engine {
    phase: AtomicU8,
    generation: AtomicU64,
    stop_flag: AtomicBool,
    clicks: AtomicU64,
    countdown: AtomicU32,
    step_index: AtomicU32,
    step_total: AtomicU32,
    started_at: Mutex<Option<Instant>>,
    final_elapsed_ms: AtomicU64,
    stop_reason: Mutex<Option<String>>,
    pending_reason: Mutex<Option<String>>,
    target: Mutex<Option<TargetWindow>>,
    corner_enabled: AtomicBool,
    yielding: AtomicBool,
    /// 落点被自己的窗口盖住的起始时刻（epoch 毫秒），0 = 当前没被盖住。
    own_cover_since: AtomicU64,
    /// 每个目标窗口上一拍的外接矩形，用来发现「窗口正被用户拖动 / 缩放」。
    last_rect: Mutex<HashMap<isize, (i32, i32, i32, i32)>>,
    main_hwnd: OnceLock<isize>,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    pub fn new() -> Self {
        Self {
            phase: AtomicU8::new(0),
            generation: AtomicU64::new(0),
            stop_flag: AtomicBool::new(false),
            clicks: AtomicU64::new(0),
            countdown: AtomicU32::new(0),
            step_index: AtomicU32::new(0),
            step_total: AtomicU32::new(0),
            started_at: Mutex::new(None),
            final_elapsed_ms: AtomicU64::new(0),
            stop_reason: Mutex::new(None),
            pending_reason: Mutex::new(None),
            target: Mutex::new(None),
            corner_enabled: AtomicBool::new(true),
            yielding: AtomicBool::new(false),
            own_cover_since: AtomicU64::new(0),
            last_rect: Mutex::new(HashMap::new()),
            main_hwnd: OnceLock::new(),
        }
    }

    pub fn set_main_hwnd(&self, hwnd: isize) {
        let _ = self.main_hwnd.set(hwnd);

    }

    pub fn phase(&self) -> Phase {
        Phase::from_u8(self.phase.load(Ordering::SeqCst))
    }

    pub fn is_running(&self) -> bool {
        self.phase() == Phase::Running
    }

    pub fn is_active(&self) -> bool {
        self.phase() != Phase::Idle
    }

    pub fn corner_enabled(&self) -> bool {
        self.corner_enabled.load(Ordering::Relaxed)
    }

    fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        match m.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        }
    }

    fn set_pending_reason(&self, reason: &str) {
        self.note_stop_reason(reason);
        *Self::lock(&self.pending_reason) = Some(reason.to_string());
    }

    /// 记录停止原因。启动失败的路径没有跑起来，需要显式调用，否则界面看不到提示。
    pub fn note_stop_reason(&self, reason: &str) {
        log::info(&format!("停止原因：{reason}"));
        *Self::lock(&self.stop_reason) = Some(reason.to_string());
    }

    pub fn start(
        self: &Arc<Self>,
        plan: StartPlan,
        click_mode: ClickMode,
        countdown_secs: u32,
        corner_failsafe: bool,
        window_guard: bool,
        fixed_target: Option<isize>,
    ) -> Result<(), String> {
        if self.phase() != Phase::Idle {
            return Err("已经在运行中".into());
        }
        match &plan {
            StartPlan::Point { cps, .. } => {
                if !(1.0..=200.0).contains(cps) {
                    return Err("CPS 必须在 1 到 200 之间".into());
                }
            }
            StartPlan::Script { steps, .. } => {
                if steps.is_empty() {
                    return Err("脚本没有可执行的步骤".into());
                }
            }
        }

        if self
            .phase
            .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err("已经在运行中".into());
        }

        let gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.stop_flag.store(false, Ordering::SeqCst);
        self.clicks.store(0, Ordering::SeqCst);
        self.final_elapsed_ms.store(0, Ordering::SeqCst);
        self.step_index.store(0, Ordering::SeqCst);
        *Self::lock(&self.stop_reason) = None;
        *Self::lock(&self.pending_reason) = None;
        *Self::lock(&self.started_at) = None;
        self.countdown.store(countdown_secs, Ordering::SeqCst);
        self.corner_enabled.store(corner_failsafe, Ordering::SeqCst);

        match &plan {
            StartPlan::Point {
                x,
                y,
                cps,
                hold_ms,
            } => {
                self.step_total.store(0, Ordering::SeqCst);
                log::info(&format!(
                    "启动引擎（单点）：point=({x}, {y}) cps={cps} hold={hold_ms}ms countdown={countdown_secs}s"
                ));
            }
            StartPlan::Script { steps, hold_ms } => {
                self.step_total.store(steps.len() as u32, Ordering::SeqCst);
                log::info(&format!(
                    "启动引擎（脚本）：{} 步 hold={hold_ms}ms countdown={countdown_secs}s",
                    steps.len()
                ));
            }
        }

        let engine = Arc::clone(self);
        let worker = thread::Builder::new()
            .name("click-engine".into())
            .spawn(move || {
                engine.worker(gen, plan, click_mode, countdown_secs, window_guard, fixed_target)
            })
            .map_err(|e| format!("无法创建点击线程：{e}"))?;
        drop(worker);
        Ok(())
    }

    pub fn stop(&self, reason: Option<&str>) {
        if self.phase() == Phase::Idle {
            return;
        }
        if let Some(r) = reason {
            *Self::lock(&self.stop_reason) = Some(r.to_string());
            log::info(&format!("停止引擎：{r}"));
        }
        self.stop_flag.store(true, Ordering::SeqCst);
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.countdown.store(0, Ordering::SeqCst);
        self.phase.store(0, Ordering::SeqCst);
        self.record_final_elapsed();
    }

    pub fn status(&self) -> EngineStatus {
        let phase = self.phase();
        let (elapsed_ms, actual_cps) = match phase {
            Phase::Running => {
                let secs = Self::lock(&self.started_at)
                    .map(|t| t.elapsed().as_secs_f64())
                    .unwrap_or(0.0);
                let clicks = self.clicks.load(Ordering::Relaxed);
                (
                    (secs * 1000.0) as u64,
                    if secs > 0.0 { clicks as f64 / secs } else { 0.0 },
                )
            }
            _ => {
                let ms = self.final_elapsed_ms.load(Ordering::Relaxed);
                let clicks = self.clicks.load(Ordering::Relaxed);
                (
                    ms,
                    if ms > 0 {
                        clicks as f64 / (ms as f64 / 1000.0)
                    } else {
                        0.0
                    },
                )
            }
        };
        EngineStatus {
            phase,
            countdown: self.countdown.load(Ordering::Relaxed),
            clicks: self.clicks.load(Ordering::Relaxed),
            elapsed_ms,
            actual_cps,
            stop_reason: Self::lock(&self.stop_reason).clone(),
            target_title: Self::lock(&self.target).as_ref().map(|t| t.title.clone()),
            step_index: self.step_index.load(Ordering::Relaxed),
            step_total: self.step_total.load(Ordering::Relaxed),
        }
    }

    fn record_final_elapsed(&self) {
        if let Some(started) = *Self::lock(&self.started_at) {
            self.final_elapsed_ms
                .store(started.elapsed().as_millis() as u64, Ordering::Relaxed);
        }
    }

    fn is_gen(&self, gen: u64) -> bool {
        self.generation.load(Ordering::SeqCst) == gen
    }

    fn worker(
        &self,
        gen: u64,
        plan: StartPlan,
        click_mode: ClickMode,
        countdown_secs: u32,
        window_guard: bool,
        fixed_target: Option<isize>,
    ) {
        // 连点坐标所在的窗口：仅用于「指定窗口」这种显式需求，
        // 不参与任何自动停止判定。
        let point_window: Option<isize> = match &plan {
            StartPlan::Point { x, y, .. } => Some(window::window_at(*x, *y)).filter(|h| *h != 0),
            StartPlan::Script { steps, .. } => steps
                .first()
                .map(|s| window::window_at(s.x, s.y))
                .filter(|h| *h != 0),
        };

        if countdown_secs > 0 {
            let deadline = Instant::now() + Duration::from_secs(countdown_secs as u64);
            while Instant::now() < deadline {
                if !self.is_gen(gen) || self.stop_flag.load(Ordering::SeqCst) {
                    self.finish(gen, None);
                    return;
                }
                let remain = deadline.saturating_duration_since(Instant::now());
                let secs = (remain.as_millis() as f64 / 1000.0).ceil() as u32;
                self.countdown.store(secs, Ordering::SeqCst);
                thread::sleep(Duration::from_millis(100));
            }
        }

        if !self.is_gen(gen) || self.stop_flag.load(Ordering::SeqCst) {
            self.countdown.store(0, Ordering::SeqCst);
            self.finish(gen, None);
            return;
        }

        // 锁定「连点坐标所在的那个窗口」：停止判定依赖它。
        // 不用前台窗口，因为启动时用户可能已经切到别的小窗口。
        let _ = window_guard;
        let mut target: Option<TargetWindow> = None;
        {
            // 用户显式指定了窗口 → 直接用它，不再依赖前台。
            if let Some(hwnd) = fixed_target {
                let (_, title) = input::window_info(hwnd);
                target = Some(TargetWindow { hwnd, title });
            }
        }
        // 记录目标窗口信息，仅用于界面显示「点击到哪个窗口」。
        if target.is_none() {
            if let Some(hwnd) = point_window {
                let (_, title) = input::window_info(hwnd);
                target = Some(TargetWindow { hwnd, title });
            } else {
                log::info("连点位置不在任何窗口上（桌面或空区域）");
            }
        }
        self.countdown.store(0, Ordering::SeqCst);
        *Self::lock(&self.target) = target.clone();
        // 上一拍的窗口矩形属于上一次运行，留着会把「刚开始」误判成「窗口在动」。
        Self::lock(&self.last_rect).clear();

        {
            let mut started = Self::lock(&self.started_at);
            *started = Some(Instant::now());
        }
        self.phase.store(2, Ordering::SeqCst);
        log::info(&format!(
            "开始点击，目标窗口：{}，方式：{}，此刻前台：{}",
            target
                .as_ref()
                .map(|t| format!("{:?}({})", t.title, t.hwnd))
                .unwrap_or_else(|| "未锁定".into()),
            match click_mode {
                ClickMode::Standard => "标准（移动光标 + 真实点击）",
                ClickMode::Message => "兼容（只投消息，不动光标）",
                ClickMode::Both => "双发",
            },
            input::describe_window(input::foreground_hwnd())
        ));

        // 每一步落点在启动那一刻所在的顶层窗口；运行中谁盖在上面就停，绝不投给陌生窗口。
        let step_targets: Vec<isize> = match &plan {
            StartPlan::Point { x, y, .. } => vec![window::window_at(*x, *y)],
            StartPlan::Script { steps, .. } => {
                steps.iter().map(|s| window::window_at(s.x, s.y)).collect()
            }
        };

        match &plan {
            StartPlan::Point {
                x,
                y,
                cps,
                hold_ms,
            } => self.run_point_loop(
                gen,
                click_mode,
                &PointRun {
                    x: *x,
                    y: *y,
                    cps: *cps,
                    hold_ms: *hold_ms,
                },
                step_targets.first().copied().unwrap_or(0),
            ),
            StartPlan::Script { steps, hold_ms } => {
                self.run_script_loop(gen, click_mode, steps, *hold_ms, &step_targets)
            }
        }
        self.finish(gen, None);
    }

    /// 引擎只关心「是否被主动停止」。
    ///
    /// 不做任何前台窗口判定：分屏使用时常在小窗口之间切换，
    /// 切窗口、鼠标去别处点击都不应打断后台连点。停止只由 F7 / 面板 / 托盘触发。
    ///
    /// 不做任何鼠标位置判定：标记定位（连点坐标）和鼠标实际位置是两回事，
    /// 用户平时移动鼠标、切换窗口都不应打断固定点位的连点。落点上是谁由 `gate` 负责。
    fn guard_ok(&self, gen: u64) -> GuardState {
        if !self.is_gen(gen) || self.stop_flag.load(Ordering::SeqCst) {
            return GuardState::Stop;
        }
        GuardState::Ok
    }

    /// 采样这一拍的输入（用户是否按着鼠标、落点现在归谁、目标窗口动不动），交给 `decide` 裁决。
    /// 自家面板盖住落点的持续时长在这里累计，好让 `decide` 是个不碰系统状态的纯函数。
    fn gate(&self, x: i32, y: i32, expected: isize) -> Gate {
        let holding = input::user_holding_button();
        let landing = if expected == 0 {
            Landing::OnTarget
        } else {
            landing_at(x, y, expected)
        };
        let moving = matches!(landing, Landing::OnTarget) && self.target_is_moving(expected);
        let own_cover_ms = if matches!(landing, Landing::OwnWindow) {
            let now = epoch_ms();
            match self.own_cover_since.load(Ordering::SeqCst) {
                0 => {
                    self.own_cover_since.store(now, Ordering::SeqCst);
                    0
                }
                since => now.saturating_sub(since),
            }
        } else {
            self.own_cover_since.store(0, Ordering::SeqCst);
            0
        };
        let decision = decide(holding, moving, &landing, own_cover_ms);
        match &decision {
            Gate::Wait => {
                let reason = if matches!(landing, Landing::OwnWindow) {
                    "落点被 Tapper 自己的窗口盖住"
                } else if moving {
                    "目标窗口正在被移动或缩放"
                } else {
                    "检测到鼠标按键正被按住"
                };
                // 只在进入让路状态时写一次日志，避免高 CPS 下刷屏。
                if !self.yielding.swap(true, Ordering::SeqCst) {
                    log::info(&format!("{reason}，连点让路（这一拍不注入）"));
                }
            }
            Gate::Go | Gate::Stop(_) => self.yielding.store(false, Ordering::SeqCst),
        }
        decision
    }

    /// 目标窗口自己正在被拖动 / 缩放：外接矩形和上一拍不一样。
    /// 只在「落点还是它」时采样，否则窗口消失了会误报成移动。
    fn target_is_moving(&self, expected: isize) -> bool {
        let Some(rect) = input::window_rect(expected) else {
            return false;
        };
        let mut last = Self::lock(&self.last_rect);
        matches!(last.insert(expected, rect), Some(previous) if previous != rect)
    }

    /// 单点连点循环。`expected` 是启动时落点所在的顶层窗口，用来识别「被别的应用挡住」。
    fn run_point_loop(
        &self,
        gen: u64,
        click_mode: ClickMode,
        run: &PointRun,
        expected: isize,
    ) {
        let PointRun {
            x,
            y,
            cps,
            hold_ms,
        } = *run;
        input::time_begin();
        let interval = Duration::from_secs_f64(1.0 / cps);
        // 高频模式：间隔小于 35ms 时启用更短的按下时长与跳过移动稳定期。
        let fast = interval < Duration::from_millis(35);
        let hold = Duration::from_millis(if fast {
            hold_ms.clamp(1, 8) as u64
        } else {
            hold_ms.max(18) as u64
        });
        let step = interval.max(hold).max(Duration::from_millis(1));
        let mut next = Instant::now();
        let mut watch = ForegroundWatch::new();

        loop {
            match self.guard_ok(gen) {
                GuardState::Stop => break,
                GuardState::Ok => {}
            }
            // 每一拍都采样，让路的那几拍也要看：抢焦点可能就发生在你按鼠标期间。
            watch.observe();
            match self.gate(x, y, expected) {
                Gate::Go => {}
                Gate::Wait => {
                    // 让路期间重新对齐节奏，否则会攒出一串补发点击。
                    if !self.sleep_checked(gen, Duration::from_millis(40)) {
                        break;
                    }
                    next = Instant::now();
                    continue;
                }
                Gate::Stop(reason) => {
                    self.set_pending_reason(&reason);
                    break;
                }
            }
            if !do_click(click_mode, x, y, hold_ms, fast, expected) {
                self.set_pending_reason("鼠标事件注入失败（目标窗口可能拦截了模拟输入）");
                break;
            }
            self.clicks.fetch_add(1, Ordering::Relaxed);

            next += step;
            let now = Instant::now();
            if next > now {
                if !self.sleep_checked(gen, next - now) {
                    break;
                }
            } else if now.duration_since(next) > Duration::from_millis(500) {
                next = now;
            }
        }

        input::release_button();
        input::time_end();
    }

    /// 脚本回放：按录制顺序循环点击；间隔取下一步的间隔，
    /// 最后一步之后用第一步的间隔作为循环间隔。
    ///
    /// `step_targets` 是启动那一刻每一步落点所在的顶层窗口，用于「落点被别的应用遮挡就停止」。
    /// 脚本可以跨窗口，所以每一步各比各的，不用整体锁定的那一个。
    fn run_script_loop(
        &self,
        gen: u64,
        click_mode: ClickMode,
        steps: &[crate::models::ScriptStep],
        hold_ms: u32,
        step_targets: &[isize],
    ) {
        input::time_begin();
        let hold = Duration::from_millis(hold_ms as u64);
        self.step_total.store(steps.len() as u32, Ordering::SeqCst);

        let mut stopped = false;
        while !stopped {
            for (i, step) in steps.iter().enumerate() {
                let gap_before_step_ms = step.gap_ms;
                match self.guard_ok(gen) {
                    GuardState::Stop => {
                        stopped = true;
                        break;
                    }
                    GuardState::Ok => {}
                }
                let expected = step_targets.get(i).copied().unwrap_or(0);
                match self.gate(step.x, step.y, expected) {
                    Gate::Go => {}
                    Gate::Wait => {
                        if !self.sleep_checked(gen, Duration::from_millis(40)) {
                            stopped = true;
                            break;
                        }
                        continue;
                    }
                    Gate::Stop(reason) => {
                        self.set_pending_reason(&reason);
                        stopped = true;
                        break;
                    }
                }
                let fast = gap_before_step_ms < 35;
                if !do_click(click_mode, step.x, step.y, hold_ms, fast, expected) {
                    self.set_pending_reason("鼠标事件注入失败（目标窗口可能拦截了模拟输入）");
                    stopped = true;
                    break;
                }
                self.clicks.fetch_add(1, Ordering::Relaxed);
                self.step_index.store(i as u32, Ordering::Relaxed);

                let gap_ms = if i + 1 < steps.len() {
                    steps[i + 1].gap_ms
                } else {
                    steps[0].gap_ms
                };
                let gap = Duration::from_millis(gap_ms.clamp(20, 60_000)).max(hold);
                if !self.sleep_checked(gen, gap) {
                    stopped = true;
                    break;
                }
            }
        }

        input::release_button();
        input::time_end();
    }

    fn sleep_checked(&self, gen: u64, duration: Duration) -> bool {
        let end = Instant::now() + duration;
        loop {
            if !self.is_gen(gen) || self.stop_flag.load(Ordering::SeqCst) {
                return false;
            }
            let now = Instant::now();
            if now >= end {
                return true;
            }
            let remain = end - now;
            if remain > Duration::from_millis(2) {
                thread::sleep(Duration::from_millis(1));
            } else {
                std::hint::spin_loop();
            }
        }
    }

    fn finish(&self, gen: u64, reason: Option<String>) {
        if !self.is_gen(gen) {
            return;
        }
        let pending = Self::lock(&self.pending_reason).take();
        if let Some(r) = reason.or(pending) {
            *Self::lock(&self.stop_reason) = Some(r.clone());
            log::info(&format!("引擎结束：{r}"));
        }
        self.countdown.store(0, Ordering::SeqCst);
        Self::lock(&self.target).take();
        self.record_final_elapsed();
        self.phase.store(0, Ordering::SeqCst);
    }
}

/// 兼容（消息）模式按下与抬起之间留多久。
///
/// 目标窗口从收到按下到收到抬起之间一直认为左键被按着，这段时间就是你的真实鼠标会被
/// 「抢走」的窗口：你的按下落进来，它看到两次按下中间没有抬起，会把你的点击当成拖动的延续。
/// 1.2.9 为了让自绘 tab 认下点击把它拉到 12–60ms，10 CPS 下等于有六分之一到六成的时间
/// 占着这个状态 —— 用户实测「微信会话点不动、窗口也拖不动」。按用户的取舍（保证能点击）
/// 压到 ≤4ms：既让 down / up 的时间戳不同、不至于被当成同一条消息合并掉，
/// 又把抢占窗口缩到原来的十几分之一。
fn post_hold_ms(hold_ms: u32, fast: bool) -> u32 {
    if fast {
        hold_ms.clamp(1, 2)
    } else {
        hold_ms.clamp(1, 4)
    }
}

/// 运行期间盯住前台窗口，专门用来取证「谁在抢焦点」。
/// 100 CPS 下逐拍写日志会把文件刷爆，所以只数切换次数，每 5 秒最多写一行；
/// 「5 秒被切了 37 次，最后一次 微信 → 游戏窗口」这种一行就足够定性。
struct ForegroundWatch {
    last: isize,
    changes: u32,
    last_from: String,
    last_to: String,
    since: Instant,
}

impl ForegroundWatch {
    fn new() -> Self {
        ForegroundWatch {
            last: input::foreground_hwnd(),
            changes: 0,
            last_from: String::new(),
            last_to: String::new(),
            since: Instant::now(),
        }
    }

    fn observe(&mut self) {
        let now = input::foreground_hwnd();
        if now != self.last {
            self.changes += 1;
            self.last_from = input::describe_window(self.last);
            self.last_to = input::describe_window(now);
            self.last = now;
        }
        if self.since.elapsed() >= Duration::from_secs(5) {
            if self.changes > 0 {
                log::info(&format!(
                    "5 秒内前台窗口被切换 {} 次，最后一次：{} → {}",
                    self.changes, self.last_from, self.last_to
                ));
            }
            self.changes = 0;
            self.since = Instant::now();
        }
    }
}

/// 兼容（消息）的一次点击要不要带上 `WM_MOUSEACTIVATE`。
///
/// 这条消息的字面意思是「有鼠标按到我这儿了，请你把自己激活到前台」：收件窗口会真的跳到
/// 前台，顺带把输入焦点从你正在打字的窗口抢走。过去每一拍都发它，于是「连点跑着、我打开微信
/// 聊天窗口打字」就变成了光标焦点反复丢失。
///
/// 规矩：**只有前台本来就是目标窗口（或它的子窗口）时才发**。
/// 目标在后台时不发也照样能收 —— 兼容模式投的是 `WM_MOUSEMOVE` + `LBUTTONDOWN/UP`，
/// 消息投递本身不要求窗口在前台，只有个别自绘控件会拿激活状态做判断，那种情况让用户自己把
/// 窗口点到前台再连点。`pinned`（启动时锁定了落点窗口）为 false 时保持旧行为：
/// 「测试一次点击」和未开窗口保护的场景里，用户就是要点当前看到的那一下。
fn want_activate(foreground_is_target: bool, holding: bool, pinned: bool) -> bool {
    !holding && (!pinned || foreground_is_target)
}

/// 按配置的注入方式执行一次点击。
/// 消息模式不移动光标；两者模式两种方式都发，适合极少数对两种输入都只认其一的窗口。
/// 「测试一次点击」也走这里，保证测出来的就是连点实际发的那一下。
pub(crate) fn do_click(
    mode: ClickMode,
    x: i32,
    y: i32,
    hold_ms: u32,
    fast: bool,
    expected: isize,
) -> bool {
    let post_hold = post_hold_ms(hold_ms, fast);
    let activate = want_activate(
        input::foreground_is_target_or_child(expected),
        input::user_holding_button(),
        expected != 0,
    );
    match mode {
        ClickMode::Standard => input::click(x, y, hold_ms, fast),
        ClickMode::Message => input::post_click(x, y, post_hold, expected, activate),
        ClickMode::Both => {
            let a = input::click(x, y, hold_ms, fast);
            let b = input::post_click(x, y, post_hold, expected, activate);
            a || b
        }
    }
}

#[cfg(test)]
mod gate_tests {
    use super::*;

    fn cut(holding: bool, moving: bool, landing: Landing, own_ms: u64) -> Gate {
        decide(holding, moving, &landing, own_ms)
    }

    #[test]
    fn every_mode_yields_while_the_user_holds_the_mouse() {
        // 用户的鼠标优先：注入的按下/抬起插进他的手势中间，目标窗口会把他的点击当成拖动的延续。
        assert_eq!(cut(true, false, Landing::OnTarget, 0), Gate::Wait);
    }

    #[test]
    fn target_being_dragged_or_resized_yields() {
        assert_eq!(cut(false, true, Landing::OnTarget, 0), Gate::Wait);
        assert_eq!(cut(true, true, Landing::OnTarget, 0), Gate::Wait);
    }

    #[test]
    fn clean_landing_injects() {
        assert_eq!(cut(false, false, Landing::OnTarget, 0), Gate::Go);
    }

    #[test]
    fn own_panel_cover_yields_then_stops_after_grace() {
        assert_eq!(cut(false, false, Landing::OwnWindow, OWN_COVER_STOP_MS - 1), Gate::Wait);
        match cut(false, false, Landing::OwnWindow, OWN_COVER_STOP_MS) {
            Gate::Stop(reason) => assert!(reason.contains("自己的面板"), "{reason}"),
            other => panic!("期望停止，实际 {other:?}"),
        }
    }

    #[test]
    fn foreign_cover_stops_with_its_title() {
        match cut(true, false, Landing::Covered("记事本".into()), 0) {
            Gate::Stop(reason) => {
                assert!(reason.contains("记事本") && reason.contains("手动重新开启"), "{reason}")
            }
            other => panic!("期望停止，实际 {other:?}"),
        }
    }

    #[test]
    fn empty_landing_stops() {
        match cut(false, false, Landing::Empty, 0) {
            Gate::Stop(reason) => assert!(reason.contains("没有窗口"), "{reason}"),
            other => panic!("期望停止，实际 {other:?}"),
        }
    }

    #[test]
    fn compat_press_never_holds_the_button_for_long() {
        // 面板里配的按下时长（含 1.2.9 那套 12–60ms）一律压到 ≤4ms：
        // 目标窗口「以为左键被按着」的时间就是它会吞掉用户真实点击的时间。
        for hold in [0, 1, 5, 18, 50, 120] {
            assert!(post_hold_ms(hold, false) <= 4, "常速 hold={hold} 过长");
            assert!(post_hold_ms(hold, true) <= 2, "高频 hold={hold} 过长");
            assert!(post_hold_ms(hold, false) >= 1, "不能为零，否则 down/up 时间戳相同");
        }
        assert_eq!(post_hold_ms(50, false), 4);
        assert_eq!(post_hold_ms(2, false), 2);
        assert_eq!(post_hold_ms(50, true), 2);
    }

    #[test]
    fn background_target_is_not_activated() {
        // 连点跑着，用户在别的窗口打字：这条消息会把目标窗口拉到前台，把打字的光顶掉。
        assert!(!want_activate(false, false, true));
    }

    #[test]
    fn foreground_target_still_activates() {
        // 目标本来就在前台：激活是无害的 no-op，而且能让个别自绘控件认下这一击。
        assert!(want_activate(true, false, true));
    }

    #[test]
    fn never_activates_while_the_user_holds_a_button() {
        assert!(!want_activate(true, true, true));
        assert!(!want_activate(false, true, false));
    }

    #[test]
    fn unpinned_click_keeps_the_old_behavior() {
        // 「测试一次点击」与未锁定窗口的脚本步骤：用户就是要点眼前这一下，照常激活。
        assert!(want_activate(false, false, false));
        assert!(want_activate(true, false, false));
    }
}
