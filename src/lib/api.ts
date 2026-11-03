import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'

import type {
  AppStateDto,
  ClickMode,
  OverlayRects,
  OverlayStateDto,
  RecordingSummary,
  RunMode,
  Script,
  Theme,
} from '../types'

export const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

/** 悬浮层与主面板共用同一份前端产物，用窗口 label 区分（浏览器调试时用 ?overlay=1）。 */
export function isOverlayWindow(): boolean {
  if (isTauri) {
    try {
      return getCurrentWindow().label === 'overlay'
    } catch {
      return false
    }
  }
  return new URLSearchParams(window.location.search).has('overlay')
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (isTauri) {
    return invoke<T>(command, args)
  }
  if (import.meta.env.DEV) {
    const { mockCall } = await import('./mock')
    return mockCall(command, args) as Promise<T>
  }
  throw new Error('Tapper 必须在桌面环境中运行')
}

export const api = {
  frontendReady: () => call<void>('frontend_ready'),
  getState: () => call<AppStateDto>('get_state'),
  getOverlayState: () => call<OverlayStateDto>('get_overlay_state'),
  start: () => call<void>('start_engine'),
  stop: () => call<void>('stop_engine'),
  setPoint: (x: number, y: number) => call<void>('set_point', { x, y }),
  setPointAtCursor: () => call<void>('set_point_at_cursor'),
  setCps: (cps: number) => call<void>('set_cps', { cps }),
  setOptions: (options: {
    countdownSecs: number
    cornerFailsafe: boolean
    windowGuard: boolean
    holdMs: number
    clickMode?: ClickMode
  }) => call<void>('set_options', options),
  setTargetWindow: (hwnd: number | null) => call<void>('set_target_window', { hwnd }),
  setHudOffset: (dx: number, dy: number) => call<void>('set_hud_offset', { dx, dy }),
  resetHudOffset: () => call<void>('reset_hud_offset'),
  pickTargetWindow: () => call<string | null>('pick_target_window'),
  restartAsAdmin: () => call<void>('restart_as_admin'),
  previewClick: () => call<void>('preview_click'),
  setTheme: (theme: Theme) => call<void>('set_theme', { theme }),
  setMode: (mode: RunMode) => call<void>('set_mode', { mode }),
  setMonitor: (index: number) => call<void>('set_monitor', { index }),
  setMarkerVisible: (visible: boolean) => call<boolean>('set_marker_visible', { visible }),
  toggleMarker: () => call<boolean>('toggle_marker'),
  setOverlayRects: (rects: OverlayRects) => call<void>('set_overlay_rects', { rects }),
  openPanel: () => call<void>('open_panel'),
  refreshMonitors: () => call<AppStateDto['monitors']>('refresh_monitors'),
  startRecording: () => call<void>('start_recording'),
  stopRecording: () => call<RecordingSummary | null>('stop_recording'),
  discardRecording: () => call<void>('discard_recording'),
  applyRecording: () => call<RecordingSummary>('apply_recording'),
  saveScript: (name: string) => call<Script>('save_script', { name }),
  deleteScript: (id: string) => call<void>('delete_script', { id }),
  selectScript: (id: string) => call<void>('select_script', { id }),
  renameScript: (id: string, name: string) => call<void>('rename_script', { id, name }),
  hideMainWindow: () => call<void>('hide_main_window'),
  quit: () => call<void>('quit_app'),
  openLogDir: () => call<void>('open_log_dir'),
}
