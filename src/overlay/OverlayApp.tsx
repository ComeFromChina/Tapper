import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react'
import { listen } from '@tauri-apps/api/event'

import { ClickArrowIcon } from '../components/Icons'
import { api, isTauri } from '../lib/api'
import {
  clampCps,
  dropDragPreview,
  formatDuration,
  formatNumber,
  toCssPoint,
  toPhysicalPoint,
} from '../lib/format'
import type { OverlayMenuItem, OverlayRect, OverlayStateDto } from '../types'

interface Ripple {
  id: number
  x: number
  y: number
}

type MenuSource = 'marker' | 'hud'

/** 右键菜单项。id 与 Rust 侧 menu_action 一一对应，按弹出处分别渲染。 */
const MENU_ITEMS: Record<'marker' | 'hud', Array<{ id: string; label: string }>> = {
  marker: [
    { id: 'panel', label: '打开主面板' },
    { id: 'hud-reset', label: '状态条回到顶部' },
    { id: 'hide-marker', label: '隐藏标记（F9）' },
  ],
  hud: [
    { id: 'panel', label: '打开主面板' },
    { id: 'hud-reset', label: '状态条回到顶部' },
  ],
}

/**
 * 悬浮层只负责渲染：鼠标交互（点击 / 拖拽 / 右键菜单 / 光标）全部由 Rust 侧的
 * 全局钩子完成，网页收不到任何指针事件 —— 悬浮层窗口永远是 click-through 的，
 * 否则整块屏幕的点击都会被它吞掉。
 */
export function OverlayApp() {
  const [state, setState] = useState<OverlayStateDto | null>(null)
  const [dragPoint, setDragPoint] = useState<{ x: number; y: number } | null>(null)
  const [dragOffset, setDragOffset] = useState<{ x: number; y: number } | null>(null)
  const [ripples, setRipples] = useState<Ripple[]>([])
  const [menu, setMenu] = useState<{ x: number; y: number; source: MenuSource } | null>(null)
  const [hoverId, setHoverId] = useState<string | null>(null)
  const [hoverRegion, setHoverRegion] = useState<string | null>(null)
  const [hudPressed, setHudPressed] = useState(false)
  const hudRef = useRef<HTMLDivElement | null>(null)
  const markerRef = useRef<HTMLDivElement | null>(null)
  const markerLabelRef = useRef<HTMLDivElement | null>(null)
  const menuRef = useRef<HTMLDivElement | null>(null)
  const rippleId = useRef(0)
  const lastMeasure = useRef(0)
  // 拖动预览的镜像 ref：轮询回调只挂一次，读 state 会拿到旧值。
  const dragPointRef = useRef<{ x: number; y: number } | null>(null)
  const dragOffsetRef = useRef<{ x: number; y: number } | null>(null)
  // 上一轮询拿到的服务端值；值一变就说明服务端已落定（拖动结束 / 复位）。
  const lastPointRef = useRef<[number, number] | null>(null)
  const lastHudRef = useRef<[number, number] | null>(null)

  // 预览的 state + ref 一起改，轮询回调才读得到最新值。
  const setPointPreview = (value: { x: number; y: number } | null) => {
    dragPointRef.current = value
    setDragPoint(value)
  }
  const setOffsetPreview = (value: { x: number; y: number } | null) => {
    dragOffsetRef.current = value
    setDragOffset(value)
  }

  const refresh = useCallback(async () => {
    try {
      const next = await api.getOverlayState()
      setState(next)
      // 拖动预览只在「服务端已落定」时作废，否则右键「状态条回到顶部」点了没反应。
      const pointServer: [number, number] = [next.config.point.x, next.config.point.y]
      const hudServer: [number, number] = next.config.hudOffset ?? [0, 0]
      if (dropDragPreview(dragPointRef.current, pointServer, lastPointRef.current)) {
        setPointPreview(null)
      }
      if (dropDragPreview(dragOffsetRef.current, hudServer, lastHudRef.current)) {
        setOffsetPreview(null)
      }
      lastPointRef.current = pointServer
      lastHudRef.current = hudServer
    } catch {
      // 忽略启动初期 IPC 未就绪
    }
  }, [])

  useEffect(() => {
    let alive = true
    const tick = async () => {
      if (alive) await refresh()
    }
    void tick()
    const id = setInterval(() => void tick(), 150)
    return () => {
      alive = false
      clearInterval(id)
    }
  }, [refresh])

  useEffect(() => {
    if (!isTauri) return
    const pending: Array<Promise<() => void>> = []
    const on = (p: Promise<() => void>) => {
      pending.push(p.catch(() => () => undefined))
    }
    on(
      listen<{ x: number; y: number }>('overlay-marker-drag', (event) => {
        setPointPreview({ x: event.payload.x, y: event.payload.y })
      }),
    )
    on(
      listen<{ dx: number; dy: number }>('overlay-hud-drag', (event) => {
        setOffsetPreview({ x: event.payload.dx, y: event.payload.dy })
      }),
    )
    on(
      listen<{ x: number; y: number; source: MenuSource }>('overlay-context-menu', (event) => {
        setHoverId(null)
        setMenu({ x: event.payload.x, y: event.payload.y, source: event.payload.source })
      }),
    )
    on(
      listen('overlay-context-menu-close', () => {
        setMenu(null)
        setHoverId(null)
      }),
    )
    on(
      listen<{ id: string | null }>('overlay-menu-hover', (event) => {
        setHoverId(event.payload.id)
      }),
    )
    on(
      listen<{ region: string | null; pressed: boolean }>('overlay-hover', (event) => {
        setHoverRegion(event.payload.region)
        setHudPressed(Boolean(event.payload.pressed))
      }),
    )
    // 跨屏拖动时 Rust 会把整块悬浮层搬到另一块屏，几何信息一变的瞬间就去取新值，
    // 否则最多要等 150ms 轮询，标记会按旧屏幕的坐标换算跑出画面。
    on(listen('overlay-geometry-changed', () => void refresh()))
    let unlisteners: Array<() => void> = []
    void Promise.all(pending).then((fns) => {
      unlisteners = fns
    })
    return () => {
      unlisteners.forEach((fn) => fn())
    }
  }, [refresh])

  const geometry = state?.geometry ?? null

  /**
   * 上报各元素的实测渲染矩形（物理像素）。
   * Rust 用它做命中测试，保证「元素画多大就只占多大」，不会多出一块透明遮罩。
   */
  const measure = useCallback(
    (force: boolean) => {
      const geo = geometry
      if (!geo) return
      const now = performance.now()
      if (!force && now - lastMeasure.current < 90) return
      lastMeasure.current = now
      const scale = geo.scaleFactor || 1
      const box = (left: number, top: number, right: number, bottom: number): OverlayRect => ({
        x: Math.round(geo.originX + ((left + right) / 2) * scale),
        y: Math.round(geo.originY + ((top + bottom) / 2) * scale),
        halfW: Math.round(((right - left) / 2) * scale),
        halfH: Math.round(((bottom - top) / 2) * scale),
      })
      const toPhys = (el: HTMLElement | null): OverlayRect | null => {
        if (!el) return null
        const r = el.getBoundingClientRect()
        if (r.width <= 0 || r.height <= 0) return null
        return box(r.left, r.top, r.right, r.bottom)
      }
      /**
       * 标记的命中范围 = 图标 + 下方标签 + 描边光晕。
       * 只量图标的话，用户点在「拖动我」那块标签上就命中不到标记，
       * 拖不动也弹不出右键菜单。
       */
      const markerBox = (): OverlayRect | null => {
        const icon = markerRef.current?.getBoundingClientRect()
        if (!icon || icon.width <= 0 || icon.height <= 0) return null
        const label = markerLabelRef.current?.getBoundingClientRect()
        let left = icon.left
        let top = icon.top
        let right = icon.right
        let bottom = icon.bottom
        if (label && label.width > 0 && label.height > 0) {
          left = Math.min(left, label.left)
          top = Math.min(top, label.top)
          right = Math.max(right, label.right)
          bottom = Math.max(bottom, label.bottom)
        }
        const pad = 4
        return box(left - pad, top - pad, right + pad, bottom + pad)
      }
      const items: OverlayMenuItem[] = []
      menuRef.current?.querySelectorAll<HTMLButtonElement>('button[data-id]').forEach((el) => {
        const rect = toPhys(el)
        const id = el.dataset.id
        if (rect && id) items.push({ id, ...rect })
      })
      void api.setOverlayRects({
        hud: toPhys(hudRef.current),
        marker: markerBox(),
        menuItems: items,
      })
    },
    [geometry],
  )

  useLayoutEffect(() => {
    measure(menu !== null)
  }, [state, menu, hoverId, dragPoint, dragOffset, measure])

  useEffect(() => {
    const id = window.setInterval(() => measure(false), 120)
    return () => window.clearInterval(id)
  }, [measure])

  const phase = state?.engine.phase
  const mode = state?.config.mode
  const cps = state?.config.cps ?? 0
  const pointX = state?.config.point.x ?? 0
  const pointY = state?.config.point.y ?? 0

  useEffect(() => {
    if (!geometry || phase !== 'running' || mode !== 'point') {
      setRipples([])
      return
    }
    let timer: number | null = null
    const spawn = () => {
      const id = ++rippleId.current
      setRipples((prev) => {
        const next = [...prev, { id, x: pointX, y: pointY }]
        return next.length > 8 ? next.slice(next.length - 8) : next
      })
      timer = window.setTimeout(() => {
        setRipples((prev) => prev.filter((r) => r.id !== id))
      }, 700)
    }
    spawn()
    const interval = Math.max(80, 1000 / clampCps(cps))
    const handle = window.setInterval(spawn, interval)
    return () => {
      window.clearInterval(handle)
      if (timer !== null) window.clearTimeout(timer)
    }
  }, [geometry, phase, mode, cps, pointX, pointY])

  if (!state || !geometry) return null

  const scale = geometry.scaleFactor || 1
  const running = state.engine.phase === 'running'
  const countdown = state.engine.countdown
  const editable = !running && state.config.mode === 'point' && dragPoint === null

  const base = toCssPoint(state.config.point, geometry)
  const pos = dragPoint ? toCssPoint(dragPoint, geometry) : base
  const physical = toPhysicalPoint(pos, geometry)
  const menuPos = menu ? toCssPoint(menu, geometry) : null

  const hudOffset = dragOffset ?? {
    x: state.config.hudOffset?.[0] ?? 0,
    y: state.config.hudOffset?.[1] ?? 0,
  }

  const cssRipples = ripples.map((r) => ({ ...r, ...toCssPoint(r, geometry) }))

  return (
    <div className={`overlay-root mode-${state.config.mode} ${running ? 'is-running' : ''}`}>
      <div
        ref={hudRef}
        className={`ov-hud ${running ? 'is-running' : ''} ${hoverRegion === 'hud' ? 'is-hover' : ''} ${
          hudPressed ? 'is-active' : ''
        }`}
        role="button"
        tabIndex={0}
        title={
          running
            ? '单击停止（等同 F7）· 拖着改位置不会停 · 右键打开菜单'
            : '单击启动 · 拖拽移动位置 · 右键打开菜单'
        }
        style={{
          left: `calc(50% + ${hudOffset.x / scale}px)`,
          top: `${18 + hudOffset.y / scale}px`,
        }}
      >
        {running ? (
          <>
            <span className="rec-dot" />
            {state.config.mode === 'script' ? (
              <span>
                脚本回放 · {state.engine.stepIndex + 1}/{state.engine.stepTotal} 步 ·{' '}
                {formatNumber(state.engine.clicks)} 次 · {formatDuration(state.engine.elapsedMs)}
              </span>
            ) : (
              <span>
                运行中 · {clampCps(state.config.cps).toFixed(1)} CPS ·{' '}
                {formatNumber(state.engine.clicks)} 次
              </span>
            )}
            <span className="ov-keys">鼠标单击或 F7 停止</span>
          </>
        ) : countdown > 0 ? (
          <>
            <span className="hud-dot" />
            启动倒计时 {countdown}
            <span className="ov-keys">鼠标单击或 F7 停止</span>
          </>
        ) : state.config.mode === 'script' ? (
          <>
            <span className="hud-dot is-idle" />
            鼠标单击或 F6 启动脚本
            <span className="ov-keys">双击标记打开面板</span>
          </>
        ) : (
          <>
            <span className="hud-dot is-idle" />
            鼠标单击或 F6 启动
            <span className="ov-keys">拖动标记定位 · 双击标记打开面板</span>
          </>
        )}
      </div>

      {countdown > 0 && (
        <div className="ov-countdown">
          <span key={countdown}>{countdown}</span>
          <em>正在启动 · 请切换到游戏窗口</em>
        </div>
      )}

      {cssRipples.map((r) => (
        <div key={r.id} className="ov-ripple" style={{ left: r.x, top: r.y }} />
      ))}

      {dragPoint && (
        <>
          <div className="guide-v" style={{ left: pos.x }} />
          <div className="guide-h" style={{ top: pos.y }} />
        </>
      )}

      <div
        ref={markerRef}
        className={`ov-marker ${running ? 'is-running' : ''} ${
          hoverRegion === 'marker' ? 'is-hover' : ''
        }`}
        style={{ left: pos.x, top: pos.y }}
      >
        <div className="ov-ring" />
        <div className="ov-core">
          <ClickArrowIcon size={20} />
        </div>
        <div className="ov-cross" />
        <div className="ov-label" ref={markerLabelRef}>
          <b>
            {state.config.mode === 'script'
              ? '脚本落点'
              : dragPoint
                ? '拖动中'
                : editable
                  ? '拖动我'
                  : '点击点'}
          </b>
          <span>
            {physical.x}, {physical.y}
          </span>
        </div>
      </div>

      {menu && menuPos && (
        <div ref={menuRef} className="ov-menu" style={{ left: menuPos.x, top: menuPos.y }}>
          {MENU_ITEMS[menu.source].map((item) => (
            <button
              key={item.id}
              type="button"
              data-id={item.id}
              className={hoverId === item.id ? 'is-hover' : ''}
            >
              {item.label}
            </button>
          ))}
        </div>
      )}
    </div>
  )
}
