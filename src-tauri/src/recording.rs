//! 录制缓冲：只记录真实用户点击（钩子侧已过滤注入事件），内存态，不落盘。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use crate::models::{RecordedClick, RecordingSummary};

pub struct Recording {
    active: AtomicBool,
    clicks: Mutex<Vec<RecordedClick>>,
    last_at: Mutex<Option<Instant>>,
}

impl Default for Recording {
    fn default() -> Self {
        Self::new()
    }
}

impl Recording {
    pub fn new() -> Self {
        Self {
            active: AtomicBool::new(false),
            clicks: Mutex::new(Vec::new()),
            last_at: Mutex::new(None),
        }
    }

    fn clicks_lock(&self) -> std::sync::MutexGuard<'_, Vec<RecordedClick>> {
        self.clicks.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn last_lock(&self) -> std::sync::MutexGuard<'_, Option<Instant>> {
        self.last_at.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }

    pub fn start(&self) {
        self.clicks_lock().clear();
        *self.last_lock() = Some(Instant::now());
        self.active.store(true, Ordering::SeqCst);
    }

    pub fn stop(&self) {
        self.active.store(false, Ordering::SeqCst);
    }

    /// 由钩子线程调用。
    pub fn push_click(&self, x: i32, y: i32) {
        if !self.active.load(Ordering::SeqCst) {
            return;
        }
        let now = Instant::now();
        let gap_ms = {
            let last = *self.last_lock();
            last.map(|t| now.duration_since(t).as_millis() as u64)
                .unwrap_or(0)
        };
        *self.last_lock() = Some(now);
        self.clicks_lock().push(RecordedClick { gap_ms, x, y });
    }

    pub fn clear(&self) {
        self.clicks_lock().clear();
        *self.last_lock() = None;
    }

    pub fn summary(&self) -> Option<RecordingSummary> {
        let clicks = self.clicks_lock();
        summary_from(&clicks)
    }

    /// 录制到的原始序列（保存脚本用）。
    pub fn steps(&self) -> Vec<crate::models::ScriptStep> {
        let clicks = self.clicks_lock();
        steps_from(&clicks)
    }
}

/// 把录制点击换算成脚本步骤：第一步的间隔是从开录到首点的等待，压到 1s 内；
/// 其余间隔夹在 20ms–60s，避免出现 0 间隔暴冲或异常长等待。
pub fn steps_from(clicks: &[RecordedClick]) -> Vec<crate::models::ScriptStep> {
    clicks
        .iter()
        .enumerate()
        .map(|(i, c)| crate::models::ScriptStep {
            x: c.x,
            y: c.y,
            gap_ms: if i == 0 {
                c.gap_ms.min(1000)
            } else {
                c.gap_ms.clamp(20, 60_000)
            },
        })
        .collect()
}

/// 纯函数：便于单测 CPS 换算与边界（最后一次点击、平均间隔、偏移）。
pub fn summary_from(clicks: &[RecordedClick]) -> Option<RecordingSummary> {
    let last = clicks.last()?;
    let gaps: Vec<f64> = clicks
        .iter()
        .skip(1)
        .map(|c| c.gap_ms as f64)
        .filter(|g| *g > 0.0)
        .collect();
    let avg_gap = if gaps.is_empty() {
        clicks[0].gap_ms as f64
    } else {
        gaps.iter().sum::<f64>() / gaps.len() as f64
    };
    let avg_gap = if avg_gap <= 0.0 { 250.0 } else { avg_gap };
    let spread = clicks
        .iter()
        .map(|c| (c.x - last.x).abs().max((c.y - last.y).abs()))
        .max()
        .unwrap_or(0);
    Some(RecordingSummary {
        count: clicks.len(),
        avg_gap_ms: (avg_gap * 10.0).round() / 10.0,
        cps: (1000.0 / avg_gap).clamp(1.0, 50.0),
        x: last.x,
        y: last.y,
        spread_px: spread,
        steps: steps_from(clicks),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn click(gap: u64, x: i32, y: i32) -> RecordedClick {
        RecordedClick { gap_ms: gap, x, y }
    }

    #[test]
    fn empty_recording_has_no_summary() {
        assert!(summary_from(&[]).is_none());
    }

    #[test]
    fn uses_last_click_position() {
        let clicks = [click(0, 10, 10), click(200, 50, 60), click(200, 90, 120)];
        let s = summary_from(&clicks).expect("summary");
        assert_eq!((s.x, s.y), (90, 120));
        assert_eq!(s.count, 3);
        assert_eq!(s.spread_px, 110);
    }

    #[test]
    fn computes_cps_from_average_gap() {
        let clicks = [click(0, 0, 0), click(250, 0, 0), click(250, 0, 0), click(250, 0, 0)];
        let s = summary_from(&clicks).expect("summary");
        assert!((s.avg_gap_ms - 250.0).abs() < 0.1);
        assert!((s.cps - 4.0).abs() < 0.01);
    }

    #[test]
    fn cps_is_clamped_to_supported_range() {
        let fast = [click(0, 0, 0), click(1, 0, 0)];
        assert!((summary_from(&fast).unwrap().cps - 50.0).abs() < 0.01);

        let slow = [click(0, 0, 0), click(60_000, 0, 0)];
        assert!((summary_from(&slow).unwrap().cps - 1.0).abs() < 0.01);
    }
}
