//! 应用共享状态。配置是唯一真值来源，运行态只放在 Engine 里。

use std::sync::{Arc, Mutex};

use crate::engine::Engine;
use crate::models::{
    AppConfig, ClickPoint, MonitorInfo, OverlayGeometry, OverlayMode, OverlayRect, OverlayRects,
    RunMode,
};
use crate::recording::Recording;

/// 命中区类型：运行中只允许点 HUD，标记不再可拖。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitZoneKind {
    Marker,
    Hud,
}

/// 可交互命中区（物理像素，中心 + 半宽半高）。
/// 鼠标进入后悬浮层才接收输入，其余时间一律穿透。
#[derive(Debug, Clone, Copy)]
pub struct HitZone {
    pub kind: HitZoneKind,
    pub x: i32,
    pub y: i32,
    pub half_w: i32,
    pub half_h: i32,
}

impl HitZone {
    pub fn contains(&self, cx: i32, cy: i32) -> bool {
        (cx - self.x).abs() <= self.half_w && (cy - self.y).abs() <= self.half_h
    }
}

/// 顶部状态条中心点（物理像素）。默认在屏幕上方居中。
pub fn hud_center(config: &AppConfig, geo: &OverlayGeometry) -> (i32, i32) {
    let s = geo.scale_factor.max(0.5);
    let base_x = geo.origin_x + (geo.width as i32) / 2;
    let base_y = geo.origin_y + ((18.0 + 17.0) * s) as i32;
    match config.hud_offset {
        Some([dx, dy]) => (base_x + dx, base_y + dy),
        None => (base_x, base_y),
    }
}

pub struct AppState {
    pub config: Mutex<AppConfig>,
    pub engine: Arc<Engine>,
    pub recording: Arc<Recording>,
    pub overlay_mode: Mutex<OverlayMode>,
    pub overlay_geometry: Mutex<Option<OverlayGeometry>>,
    pub monitors: Mutex<Vec<MonitorInfo>>,
    pub failed_hotkeys: Mutex<Vec<String>>,
    pub notice: Mutex<Option<String>>,
    pub hit_zones: Mutex<Vec<HitZone>>,
    /// 前端实测的状态条 / 标记矩形，命中区按它生成，避免留下透明遮罩。
    pub overlay_rects: Mutex<OverlayRects>,
}

impl AppState {
    pub fn new(
        config: AppConfig,
        engine: Arc<Engine>,
        recording: Arc<Recording>,
        monitors: Vec<MonitorInfo>,
        failed_hotkeys: Vec<String>,
    ) -> Self {
        let state = Self {
            config: Mutex::new(config),
            engine,
            recording,
            overlay_mode: Mutex::new(OverlayMode::Hidden),
            overlay_geometry: Mutex::new(None),
            monitors: Mutex::new(monitors),
            failed_hotkeys: Mutex::new(failed_hotkeys),
            notice: Mutex::new(None),
            hit_zones: Mutex::new(Vec::new()),
            overlay_rects: Mutex::new(OverlayRects::default()),
        };
        state.refresh_hit_zones();
        state
    }

    pub fn take_notice(&self) -> Option<String> {
        self.notice
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
    }

    pub fn config_snapshot(&self) -> AppConfig {
        self.config
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub fn overlay_mode(&self) -> OverlayMode {
        *self
            .overlay_mode
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    pub fn set_overlay_mode(&self, mode: OverlayMode) {
        *self
            .overlay_mode
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = mode;
    }

    /// 只有单点模式才允许拖动标记；脚本模式的落点来自录制，不可拖。
    pub fn refresh_hit_zones(&self) {
        let config = self.config_snapshot();
        let geo = *self
            .overlay_geometry
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let scale = geo
            .as_ref()
            .map(|g| g.scale_factor)
            .unwrap_or(1.0)
            .max(0.5);
        let rects = self
            .overlay_rects
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        let mut zones = Vec::new();

        // 标记：实测矩形优先（标记画多大就占多大），实测值缺失时按 46px 视觉尺寸兜底。
        let marker = rects.marker.unwrap_or(OverlayRect {
            x: config.point.x,
            y: config.point.y,
            half_w: (23.0 * scale) as i32,
            half_h: (23.0 * scale) as i32,
        });
        zones.push(HitZone {
            kind: HitZoneKind::Marker,
            x: marker.x,
            y: marker.y,
            half_w: marker.half_w.max(1),
            half_h: marker.half_h.max(1),
        });

        // 顶部 HUD 条：同样用实测矩形，未上报时回退到较小的默认尺寸（宁可小一点也不留遮罩）。
        if let Some(g) = geo {
            let s = g.scale_factor.max(0.5);
            let (hx, hy) = hud_center(&config, &g);
            let hud = rects.hud.unwrap_or(OverlayRect {
                x: hx,
                y: hy,
                half_w: (200.0 * s) as i32,
                half_h: (22.0 * s) as i32,
            });
            zones.push(HitZone {
                kind: HitZoneKind::Hud,
                x: hud.x,
                y: hud.y,
                half_w: hud.half_w.max(1),
                half_h: hud.half_h.max(1),
            });
        }
        *self
            .hit_zones
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = zones;
    }

    /// 记录前端实测的矩形并重建命中区。
    pub fn set_overlay_rects(&self, rects: OverlayRects) {
        *self
            .overlay_rects
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = rects;
        self.refresh_hit_zones();
    }

    pub fn hit_zones_snapshot(&self) -> Vec<HitZone> {
        self.hit_zones
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// 鼠标位置落在哪个悬浮层区域（状态条 / 标记）。
    pub fn region_at(&self, x: i32, y: i32) -> Option<HitZoneKind> {
        self.hit_zones_snapshot()
            .iter()
            .find(|z| z.contains(x, y))
            .map(|z| z.kind)
    }

    /// 当前真正能响应的区域：状态条任何时候可点；标记按当前模式决定。
    pub fn interactive_region_at(&self, x: i32, y: i32) -> Option<HitZoneKind> {
        match self.region_at(x, y) {
            Some(HitZoneKind::Hud) => Some(HitZoneKind::Hud),
            Some(HitZoneKind::Marker) if self.marker_interactive() => Some(HitZoneKind::Marker),
            _ => None,
        }
    }

    /// 当前模式（不克隆整份配置，鼠标热路径用）。
    pub fn mode(&self) -> RunMode {
        self.config.lock().unwrap_or_else(|p| p.into_inner()).mode
    }

    /// 当前点击落点（不克隆整份配置）。
    pub fn point(&self) -> ClickPoint {
        self.config.lock().unwrap_or_else(|p| p.into_inner()).point
    }

    /// 状态条位置偏移（不克隆整份配置）。
    pub fn hud_offset(&self) -> Option<[i32; 2]> {
        self.config.lock().unwrap_or_else(|p| p.into_inner()).hud_offset
    }

    /// 悬浮层所在显示器的缩放比例（鼠标判定要用，物理像素 = CSS 像素 × 缩放）。
    pub fn overlay_scale(&self) -> f64 {
        self.overlay_geometry
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .map(|g| g.scale_factor)
            .unwrap_or(1.0)
            .max(1.0)
    }

    /// 标记上是否会出现手型光标：单点模式下它就是点击目标；
    /// 脚本模式下空闲时双击能唤起面板，所以也算可交互。
    pub fn marker_interactive(&self) -> bool {
        self.mode() == RunMode::Point || !self.engine.is_active()
    }

    /// 标记是否可拖动：单点模式 + 引擎空闲（倒计时也不算）+ 不在录制中。
    pub fn marker_editable(&self) -> bool {
        self.mode() == RunMode::Point && !self.engine.is_active() && !self.recording.is_active()
    }

    /// 状态条默认位置的中心点与半宽半高（物理像素）。
    /// 拖动范围限制要用它，保证整条状态条始终留在屏幕内。
    pub fn hud_metrics(&self) -> Option<(i32, i32, i32, i32)> {
        let geo = *self
            .overlay_geometry
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let geo = geo?;
        let s = geo.scale_factor.max(0.5);
        let base_x = geo.origin_x + (geo.width as i32) / 2;
        let base_y = geo.origin_y + ((18.0 + 17.0) * s) as i32;
        Some((base_x, base_y, (200.0 * s) as i32, (22.0 * s) as i32))
    }

    /// 右键菜单项命中测试。
    pub fn menu_item_at(&self, x: i32, y: i32) -> Option<String> {
        let rects = self
            .overlay_rects
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        rects
            .menu_items
            .into_iter()
            .find(|m| (x - m.x).abs() <= m.half_w && (y - m.y).abs() <= m.half_h)
            .map(|m| m.id)
    }

    pub fn monitors_snapshot(&self) -> Vec<MonitorInfo> {
        self.monitors
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// 悬浮层当前铺在哪块屏上（按位置 + 尺寸比对；配置里存的序号可能已经过期）。
    pub fn current_monitor_index(&self) -> usize {
        let list = self.monitors_snapshot();
        let geo = *self
            .overlay_geometry
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if let Some(g) = geo {
            if let Some(i) = list
                .iter()
                .position(|m| m.x == g.origin_x && m.y == g.origin_y && m.width == g.width && m.height == g.height)
            {
                return i;
            }
        }
        list.iter().position(|m| m.is_primary).unwrap_or(0)
    }

    /// 状态条当前的实际中心（物理像素绝对坐标），拖动以它为锚点。
    pub fn hud_anchor(&self) -> Option<(i32, i32)> {
        let (base_x, base_y, _, _) = self.hud_metrics()?;
        let [dx, dy] = self.hud_offset().unwrap_or([0, 0]);
        Some((base_x + dx, base_y + dy))
    }
}
