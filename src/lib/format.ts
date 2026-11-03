export function formatNumber(value: number): string {
  return Math.round(value).toLocaleString('en-US')
}

/** 毫秒 → mm:ss（超过 1 小时显示 h:mm:ss）。 */
export function formatDuration(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000))
  const h = Math.floor(total / 3600)
  const m = Math.floor((total % 3600) / 60)
  const s = total % 60
  const pad = (n: number) => String(n).padStart(2, '0')
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${pad(m)}:${pad(s)}`
}

/** 根据 CPS 给出间隔描述。 */
export function intervalLabel(cps: number): string {
  const ms = 1000 / Math.max(cps, 0.1)
  return ms >= 1000 ? `每 ${(ms / 1000).toFixed(1)} 秒一次` : `每 ${Math.round(ms)} ms 一次`
}

export function clampCps(value: number): number {
  if (!Number.isFinite(value)) return 4
  return Math.min(200, Math.max(1, Math.round(value * 10) / 10))
}

/** 物理坐标 → 悬浮层 CSS 坐标。 */
export function toCssPoint(
  point: { x: number; y: number },
  origin: { originX: number; originY: number; scaleFactor: number },
): { x: number; y: number } {
  const scale = origin.scaleFactor || 1
  return {
    x: (point.x - origin.originX) / scale,
    y: (point.y - origin.originY) / scale,
  }
}

/** 悬浮层 CSS 坐标 → 物理坐标。 */
export function toPhysicalPoint(
  css: { x: number; y: number },
  origin: { originX: number; originY: number; scaleFactor: number },
): { x: number; y: number } {
  const scale = origin.scaleFactor || 1
  return {
    x: Math.round(origin.originX + css.x * scale),
    y: Math.round(origin.originY + css.y * scale),
  }
}

/**
 * 悬浮层的本地拖动预览该不该作废。
 *
 * 拖动过程中 Rust 只在「抬起」时保存最终值，抬起前的最后一次预览和最终值可能差一两个像素；
 * 若只在两者相等时才清预览，预览就会永久盖住服务端值 —— 表现为
 * 右键「状态条回到顶部」点了没反应。所以服务端值一旦变化（落定 / 复位 / 面板改点位）就直接作废。
 */
export function dropDragPreview(
  preview: { x: number; y: number } | null,
  server: [number, number],
  lastServer: [number, number] | null,
): boolean {
  if (!preview) return false
  if (preview.x === server[0] && preview.y === server[1]) return true
  return lastServer !== null && (lastServer[0] !== server[0] || lastServer[1] !== server[1])
}
