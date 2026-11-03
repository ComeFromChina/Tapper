//! 系统托盘。Tauri v2 的托盘在主 crate 里，没有 tauri-plugin-tray。

use std::sync::{Arc, Mutex};

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

pub const MENU_SHOW: &str = "tray-show";
pub const MENU_TOGGLE: &str = "tray-toggle";
pub const MENU_STOP: &str = "tray-stop";
pub const MENU_MARKER: &str = "tray-marker";
pub const MENU_QUIT: &str = "tray-quit";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayState {
    Idle,
    Running,
}

fn icon_image(state: TrayState) -> Image<'static> {
    let bytes: &'static [u8] = match state {
        TrayState::Idle => include_bytes!("../icons/tray-idle.png"),
        TrayState::Running => include_bytes!("../icons/tray-running.png"),
    };
    Image::from_bytes(bytes).expect("内置托盘图标解析失败")
}

#[derive(Clone)]
pub struct TrayController {
    icon: Arc<Mutex<Option<tauri::tray::TrayIcon<Wry>>>>,
    state: Arc<Mutex<Option<TrayState>>>,
}

impl Default for TrayController {
    fn default() -> Self {
        Self::new()
    }
}

impl TrayController {
    pub fn new() -> Self {
        Self {
            icon: Arc::new(Mutex::new(None)),
            state: Arc::new(Mutex::new(None)),
        }
    }

    pub fn setup(
        &self,
        app: &AppHandle,
        on_toggle: impl Fn(&AppHandle) + Send + Sync + 'static,
        on_stop: impl Fn(&AppHandle) + Send + Sync + 'static,
        on_marker: impl Fn(&AppHandle) + Send + Sync + 'static,
    ) -> tauri::Result<()> {
        let show = MenuItem::with_id(app, MENU_SHOW, "显示主面板", true, None::<&str>)?;
        let toggle = MenuItem::with_id(app, MENU_TOGGLE, "开始 / 暂停 (F6)", true, None::<&str>)?;
        let stop = MenuItem::with_id(app, MENU_STOP, "紧急停止 (F7)", true, None::<&str>)?;
        let marker = MenuItem::with_id(app, MENU_MARKER, "显示 / 隐藏标记 (F9)", true, None::<&str>)?;
        let quit = MenuItem::with_id(app, MENU_QUIT, "退出 Tapper", true, None::<&str>)?;
        let sep1 = PredefinedMenuItem::separator(app)?;
        let sep2 = PredefinedMenuItem::separator(app)?;
        let menu = Menu::with_items(
            app,
            &[&show, &sep1, &toggle, &stop, &marker, &sep2, &quit],
        )?;

        let on_toggle = Arc::new(on_toggle);
        let on_stop = Arc::new(on_stop);
        let on_marker = Arc::new(on_marker);

        let mut builder = TrayIconBuilder::with_id("main-tray")
            .icon(icon_image(TrayState::Idle))
            .tooltip("Tapper · 就绪")
            .menu(&menu)
            .show_menu_on_left_click(false)
            .on_menu_event(move |app, event| match event.id().as_ref() {
                MENU_SHOW => crate::show_main_window(app),
                MENU_TOGGLE => on_toggle(app),
                MENU_STOP => on_stop(app),
                MENU_MARKER => on_marker(app),
                MENU_QUIT => {
                    let running = app
                        .try_state::<crate::state::AppState>()
                        .map(|s| s.engine.is_active())
                        .unwrap_or(false);
                    if running {
                        crate::show_main_window(app);
                        let _ = tauri::Emitter::emit(app, crate::EVT_QUIT_CONFIRM, ());
                    } else {
                        app.exit(0);
                    }
                }
                _ => {}
            })
            .on_tray_icon_event(|tray, event| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    crate::show_main_window(tray.app_handle());
                }
            });

        if let Some(icon) = app.default_window_icon() {
            builder = builder.icon(icon.clone());
        }

        let icon = builder.build(app)?;
        *self.icon.lock().unwrap_or_else(|p| p.into_inner()) = Some(icon);
        Ok(())
    }

    pub fn set_state(&self, state: TrayState) {
        {
            let mut current = self.state.lock().unwrap_or_else(|p| p.into_inner());
            if *current == Some(state) {
                return;
            }
            *current = Some(state);
        }
        if let Some(icon) = self.icon.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
            let _ = icon.set_icon(Some(icon_image(state)));
            let tooltip = match state {
                TrayState::Idle => "Tapper · 就绪",
                TrayState::Running => "Tapper · 运行中（F7 停止）",
            };
            let _ = icon.set_tooltip(Some(tooltip));
        }
    }
}
