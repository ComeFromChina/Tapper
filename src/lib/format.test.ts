import { describe, expect, it } from 'vitest'

import {
  clampCps,
  dropDragPreview,
  formatDuration,
  formatNumber,
  intervalLabel,
  toCssPoint,
  toPhysicalPoint,
} from './format'

describe('format helpers', () => {
  it('formats click counts with separators', () => {
    expect(formatNumber(0)).toBe('0')
    expect(formatNumber(1234.6)).toBe('1,235')
  })

  it('formats durations', () => {
    expect(formatDuration(0)).toBe('00:00')
    expect(formatDuration(59_000)).toBe('00:59')
    expect(formatDuration(3_723_000)).toBe('1:02:03')
  })

  it('describes intervals from cps', () => {
    expect(intervalLabel(4)).toBe('每 250 ms 一次')
    expect(intervalLabel(0.5)).toBe('每 2.0 秒一次')
  })

  it('clamps cps into the supported range', () => {
    expect(clampCps(0)).toBe(1)
    expect(clampCps(99)).toBe(99)
    expect(clampCps(500)).toBe(200)
    expect(clampCps(Number.NaN)).toBe(4)
    expect(clampCps(7.26)).toBe(7.3)
  })

  it('converts physical and css coordinates consistently', () => {
    const origin = { originX: 1920, originY: -200, scaleFactor: 1.5 }
    const physical = { x: 2220, y: 100 }
    const css = toCssPoint(physical, origin)
    expect(css).toEqual({ x: 200, y: 200 })
    expect(toPhysicalPoint(css, origin)).toEqual(physical)
  })

  it('drops the drag preview only once the server value has settled', () => {
    // 没有预览 → 不用清
    expect(dropDragPreview(null, [0, 0], null)).toBe(false)
    // 拖动落定：预览值和服务端值一致 → 清掉
    expect(dropDragPreview({ x: 120, y: 40 }, [120, 40], [0, 0])).toBe(true)
    // 拖动中：服务端还没动，预览领先 → 保留，否则标记会跳回去
    expect(dropDragPreview({ x: 120, y: 40 }, [0, 0], [0, 0])).toBe(false)
    // 服务端值变了但和预览差一两个像素（抬起时保存的最终值 ≠ 最后一次预览）→ 也要清掉
    expect(dropDragPreview({ x: 120, y: 40 }, [119, 41], [0, 0])).toBe(true)
    // 右键「状态条回到顶部」：服务端复位，预览是旧的拖拽位置 → 必须清掉，否则点了没反应
    expect(dropDragPreview({ x: 120, y: 40 }, [0, 0], [120, 40])).toBe(true)
    // 首轮询还没有参照值 → 保留预览
    expect(dropDragPreview({ x: 120, y: 40 }, [0, 0], null)).toBe(false)
  })
})
