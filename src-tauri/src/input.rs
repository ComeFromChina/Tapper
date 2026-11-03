//! Windows 输入注入与窗口查询。所有系统调用收敛在这里。

use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE,
    MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_VIRTUALDESK, MOUSEINPUT,
    VK_LBUTTON, VK_MBUTTON, VK_RBUTTON,
};
use windows::Win32::Foundation::POINT;
use std::os::windows::ffi::OsStrExt;

use std::sync::OnceLock;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::{
    CreateWaitableTimerExW, SetWaitableTimer, WaitForSingleObject, INFINITE,
};
use windows::Win32::UI::Shell::{IsUserAnAdmin, ShellExecuteW};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::WindowsAndMessaging::{
    GetAncestor, GetCursorPos, GetForegroundWindow, GetParent, GetSystemMetrics, GetWindowTextW,
    GetWindowLongPtrW, GetWindowThreadProcessId, PostMessageW, SetCursorPos, WindowFromPoint,
    GA_PARENT, GA_ROOT, GWL_STYLE,
    SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
};

/// 给注入事件打标记，便于日志与后续扩展（不依赖它做判断）。
const INJECT_TAG: usize = 0x5441_5052; // "TAPR"

// Win32 定时器精度：默认 15.6ms 无法支撑高 CPS，运行时临时调到 1ms。
#[link(name = "winmm")]
extern "system" {
    fn timeBeginPeriod(uperiod: u32) -> u32;
    fn timeEndPeriod(uperiod: u32) -> u32;
}

pub fn time_begin() {
    unsafe {
        let _ = timeBeginPeriod(1);
    }
}

pub fn time_end() {
    unsafe {
        let _ = timeEndPeriod(1);
    }
}

/// 虚拟桌面（所有显示器并集）的物理像素边界：x, y, width, height。
pub fn virtual_screen() -> (i32, i32, i32, i32) {
    unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1),
            GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1),
        )
    }
}

/// 屏幕左上角（甩角急停判据）。
pub fn corner_point() -> (i32, i32) {
    let (x, y, _, _) = virtual_screen();
    (x, y)
}

fn move_input(x: i32, y: i32) -> INPUT {
    let (vx, vy, vw, vh) = virtual_screen();
    let nx = (((x - vx) as f64) * 65535.0 / (vw - 1) as f64).round() as i32;
    let ny = (((y - vy) as f64) * 65535.0 / (vh - 1) as f64).round() as i32;
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: nx,
                dy: ny,
                mouseData: 0,
                dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                time: 0,
                dwExtraInfo: INJECT_TAG,
            },
        },
    }
}

fn button_input(up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: 0,
                dwFlags: if up {
                    MOUSEEVENTF_LEFTUP
                } else {
                    MOUSEEVENTF_LEFTDOWN
                },
                time: 0,
                dwExtraInfo: INJECT_TAG,
            },
        },
    }
}

/// 发送输入。被系统拒绝（例如当前会话处于锁屏 / 不允许注入）时重试几次再判定失败，
/// 避免瞬时拒绝导致整个任务被误停。
fn send(inputs: &[INPUT]) -> bool {
    if inputs.is_empty() {
        return true;
    }
    let size = std::mem::size_of::<INPUT>() as i32;
    for attempt in 0..3 {
        let sent = unsafe { SendInput(inputs, size) };
        if sent as usize == inputs.len() {
            return true;
        }
        if attempt < 2 {
            thread::sleep(Duration::from_millis(2));
        }
    }
    false
}

/// 光标移动到物理坐标。返回是否成功。
pub fn move_to(x: i32, y: i32) -> bool {
    send(&[move_input(x, y)])
}

/// 在指定坐标完成一次左键点击。
///
/// `fast = true`（高频模式）：光标已在目标处时不再移动，省掉移动后的 6ms 稳定期，
/// 按下时长压到 1–8ms，才可能做到 10ms / 5ms 的间隔。
/// 代价是少数窗口会丢失过短的按下事件，低频时仍走兼容路径。
pub fn click(x: i32, y: i32, hold_ms: u32, fast: bool) -> bool {
    let (cx, cy) = cursor_pos();
    let need_move = (cx - x).abs() > 1 || (cy - y).abs() > 1;
    if need_move {
        unsafe {
            let _ = SetCursorPos(x, y);
        }
        if !move_to(x, y) {
            return false;
        }
        // 移动后给窗口时间重新命中测试；高频模式只留 1ms。
        if fast {
            sleep_hr(Duration::from_millis(1));
        } else {
            sleep_precise(Duration::from_millis(6));
        }
    }

    let press_ms = if fast {
        hold_ms.clamp(1, 8)
    } else {
        hold_ms.max(18)
    };
    if !send(&[button_input(false)]) {
        let _ = send(&[button_input(true)]);
        return false;
    }
    sleep_hr(Duration::from_millis(press_ms as u64));
    send(&[button_input(true)])
}

/// 把左键消息投递给落点上那个窗口（兼容模式）。
///
/// 不移动光标，适合不响应 SendInput 的自绘 / 嵌入式窗口。
///
/// 只发给落点上那一个窗口，不再同时发一份给顶层根窗口：一份消息被父子两层各处理一次
/// 等于连点两下，切过去的 tab 会被第二下切回来，表现为「时好时坏」。
/// `hold_ms` 是按下与抬起之间的间隔。
/// `expected` 是连点启动时落点所在的顶层窗口（0 = 不校验，测试点击走这条）。
/// 判定和投递之间总有几微秒的缝隙，这期间别的窗口可能刚好盖到落点上（拖动中的窗口、
/// 刚弹出的悬浮条）；这时改投给预期的那个窗口，绝不把点击送给陌生窗口。
pub fn post_click(x: i32, y: i32, hold_ms: u32, expected: isize, activate: bool) -> bool {
    unsafe {
        let under = WindowFromPoint(POINT { x, y });
        if under.0.is_null() {
            return false;
        }
        let under_root = GetAncestor(under, GA_ROOT);
        let node = if expected != 0 && under_root.0 as isize != expected {
            HWND(expected as *mut _)
        } else {
            under
        };
        let target = click_target(node);
        let mut client = POINT { x, y };
        let _ = ScreenToClient(target, &mut client);
        let lparam = pack_client(client);
        // 激活 + 移动 + 按下：缺前置 move 的窗口会把点击当成未命中。
        // WM_MOUSEACTIVATE 等于「请你把自己激活到前台」，所以要不要发由调用方决定
        // （见 engine::want_activate）：目标在后台时发它，它就会跳到前台，
        // 把你正在打字的窗口的输入焦点顶掉；你正按着鼠标拖别的窗口时更不能发。
        if activate {
            let _ = PostMessageW(Some(target), 0x0021, WPARAM(1), lparam); // WM_MOUSEACTIVATE
        }
        let _ = PostMessageW(Some(target), WM_MOUSEMOVE, WPARAM(0), lparam);
        let _ = PostMessageW(Some(target), WM_LBUTTONDOWN, WPARAM(0x0001), lparam);
        sleep_hr(Duration::from_millis(hold_ms.max(1) as u64));
        let _ = PostMessageW(Some(target), WM_LBUTTONUP, WPARAM(0), lparam);
        true
    }
}

/// 兼容模式一次点击该发给谁。
///
/// 真实鼠标落在被禁用的子控件上时，消息由父窗口处理；而 PostMessage 是直接塞队列，
/// 禁用的子窗口会把消息吃掉，所以要往上找到第一个可用窗口。
unsafe fn click_target(under: HWND) -> HWND {
    let mut cur = under;
    for _ in 0..8 {
        // WS_DISABLED：真实鼠标会把它交给父窗口处理，PostMessage 不会，得自己往上找。
        if GetWindowLongPtrW(cur, GWL_STYLE) & 0x0800_0000 == 0 {
            return cur;
        }
        match GetParent(cur) {
            Ok(parent) if !parent.0.is_null() => cur = parent,
            _ => return cur,
        }
    }
    cur
}

fn pack_client(client: POINT) -> LPARAM {
    LPARAM((client.y as isize) << 16 | (client.x as isize & 0xFFFF))
}

/// 当前前台窗口的句柄（0 = 此刻没有前台窗口）。
pub fn foreground_hwnd() -> isize {
    unsafe { GetForegroundWindow().0 as isize }
}

/// 「标题(句柄)」形式的窗口描述，只给日志用；无标题或没有前台时用可读的占位。
pub fn describe_window(hwnd: isize) -> String {
    if hwnd == 0 {
        return "（无前台窗口）".to_string();
    }
    let (_, title) = window_info(hwnd);
    let t = title.trim().to_string();
    if t.is_empty() {
        format!("无标题窗口({hwnd})")
    } else {
        format!("{t}({hwnd})")
    }
}

/// 前台窗口是不是 `target` 本身，或者是它的子窗口（例如你正在目标窗口的输入框里打字）。
/// `target == 0`（没锁定目标）时返回 true：那种情况下没有「抢谁的焦点」可言，保持原行为。
pub fn foreground_is_target_or_child(target: isize) -> bool {
    if target == 0 {
        return true;
    }
    unsafe {
        let mut cur = GetForegroundWindow().0 as isize;
        // 从前台窗口往上走父链，走到根为止；最多 8 层，防环。
        for _ in 0..8 {
            if cur == 0 {
                return false;
            }
            if cur == target {
                return true;
            }
            let parent = GetAncestor(HWND(cur as *mut _), GA_PARENT).0 as isize;
            if parent == cur {
                return false;
            }
            cur = parent;
        }
        false
    }
}

/// 用户是否正物理按着鼠标任一键。按住期间引擎让路：他正在拖动窗口边缘或做手势，
/// 这时注入一次按下 / 抬起会被那个窗口当成手势的一部分（窗口跳位、突然变小）。
pub fn user_holding_button() -> bool {
    unsafe {
        [VK_LBUTTON, VK_RBUTTON, VK_MBUTTON]
            .iter()
            .any(|key| GetAsyncKeyState(key.0 as i32) < 0)
    }
}

/// 当前进程是否已提权。目标程序以管理员运行时，未提权的模拟输入会被 UIPI 拦下。
pub fn is_elevated() -> bool {
    unsafe { IsUserAnAdmin().as_bool() }
}

/// 以管理员身份重启自身。
pub fn relaunch_as_admin() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let file: Vec<u16> = exe.as_os_str().encode_wide().chain(Some(0)).collect();
    let verb: Vec<u16> = "runas".encode_utf16().chain(Some(0)).collect();
    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(file.as_ptr()),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    if result.0 as isize <= 32 {
        return Err("提权被取消，或系统拒绝了请求".into());
    }
    Ok(())
}



/// 兜底抬起左键：停止时调用，防止长按中途停止把键“按死”。
pub fn release_button() {
    let _ = send(&[button_input(true)]);
}

/// 前台窗口句柄（0 表示没有）。
/// 屏幕上某点所在的顶层窗口（0 = 无）。
pub fn window_from_point(x: i32, y: i32) -> isize {
    unsafe {
        let under = WindowFromPoint(POINT { x, y });
        if under.0.is_null() {
            return 0;
        }
        let root = GetAncestor(under, GA_ROOT);
        if root.0.is_null() {
            under.0 as isize
        } else {
            root.0 as isize
        }
    }
}

pub fn cursor_pos() -> (i32, i32) {
    let mut pt = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut pt);
    }
    (pt.x, pt.y)
}

/// 句柄归属进程与标题。
pub fn window_info(hwnd: isize) -> (u32, String) {
    use windows::Win32::Foundation::HWND;
    let hwnd = HWND(hwnd as *mut core::ffi::c_void);
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    let mut buf = [0u16; 512];
    let len = unsafe { GetWindowTextW(hwnd, &mut buf) };
    let title = String::from_utf16_lossy(&buf[..len.max(0) as usize]);
    (pid, title)
}

pub fn current_pid() -> u32 {
    std::process::id()
}

/// 窗口的外接矩形（屏幕物理像素）。窗口被用户拖动 / 缩放时它就是变化的信号。
pub fn window_rect(hwnd: isize) -> Option<(i32, i32, i32, i32)> {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
    if hwnd == 0 {
        return None;
    }
    let mut rect = RECT::default();
    unsafe { GetWindowRect(HWND(hwnd as *mut core::ffi::c_void), &mut rect) }.ok()?;
    Some((rect.left, rect.top, rect.right, rect.bottom))
}

/// 高精度等待计时器（Windows 10 1803+）：0.5ms 级等待，比 Sleep 的 1–2ms 抖动小得多。
fn timer() -> Option<HANDLE> {
    static TIMER: OnceLock<isize> = OnceLock::new();
    let raw = *TIMER.get_or_init(|| unsafe {
        match CreateWaitableTimerExW(
            None,
            PCWSTR(std::ptr::null()),
            0x0000_0002, // CREATE_WAITABLE_TIMER_HIGH_RESOLUTION
            0x001F_0003, // TIMER_ALL_ACCESS
        ) {
            Ok(h) => h.0 as isize,
            Err(_) => 0,
        }
    });
    if raw == 0 {
        None
    } else {
        Some(HANDLE(raw as *mut core::ffi::c_void))
    }
}

/// 高精度睡眠；拿不到高精度计时器时退回自旋。
pub fn sleep_hr(duration: Duration) {
    if duration.is_zero() {
        return;
    }
    if let Some(h) = timer() {
        let due: i64 = -((duration.as_nanos() as i64) / 100).max(1);
        unsafe {
            if SetWaitableTimer(h, &due, 0, None, None, false).is_ok() {
                WaitForSingleObject(h, INFINITE);
                return;
            }
        }
    }
    sleep_precise(duration);
}

/// 高精度睡眠（最后 1ms 自旋）。调用方负责把长等待拆成短段以便检查停止标志。
pub fn sleep_precise(duration: Duration) {
    let end = Instant::now() + duration;
    loop {
        let now = Instant::now();
        if now >= end {
            return;
        }
        let remain = end - now;
        if remain > Duration::from_millis(2) {
            thread::sleep(remain - Duration::from_millis(1));
        } else {
            std::hint::spin_loop();
        }
    }
}
