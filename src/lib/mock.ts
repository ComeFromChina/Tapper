import type { AppStateDto, RecordingSummary, RunMode, Script, Theme } from '../types'

export const mockState: AppStateDto = {
  config: {
    theme: 'dark',
    mode: 'point',
    point: { x: 960, y: 540 },
    cps: 4,
    clickMode: 'standard',
    holdMs: 0,
    countdownSecs: 3,
    cornerFailsafe: true,
    windowGuard: true,
    showMarker: true,
    targetHwnd: null,
    hudOffset: null,
    monitorIndex: null,
    scripts: [],
    activeScriptId: null,
  },
  engine: {
    phase: 'idle',
    countdown: 0,
    clicks: 0,
    elapsedMs: 0,
    actualCps: 0,
    stopReason: null,
    targetTitle: 'MockGame.exe',
    stepIndex: 0,
    stepTotal: 0,
  },
  recording: null,
  isRecording: false,
  overlayMode: 'hidden',
  overlay: { originX: 0, originY: 0, width: 1920, height: 1080, scaleFactor: 1 },
  monitors: [
    { name: 'DISPLAY1', x: 0, y: 0, width: 1920, height: 1080, scaleFactor: 1, isPrimary: true },
  ],
  failedHotkeys: [],
  elevated: false,
  notice: null,
  version: '1.2.4',
  appName: 'Tapper',
  description: '固定点位 / 脚本连点器',
  exeDir: 'D:/Tapper',
  logPath: 'C:/mock/tapper.log',
}

const params = new URLSearchParams(window.location.search)
const mode = params.get('mode')
const themeParam = params.get('theme')
if (themeParam === 'dark' || themeParam === 'light' || themeParam === 'system') {
  mockState.config.theme = themeParam
}
if (params.has('overlay')) {
  mockState.overlayMode = 'passive'
}
{
  if (mode === 'running') {
    mockState.engine.phase = 'running'
    mockState.engine.actualCps = mockState.config.cps
    mockState.engine.clicks = 1288
    mockState.engine.elapsedMs = 322_000
  }
  if (mode === 'script') {
    const steps = [
      { x: 700, y: 420, gapMs: 300 },
      { x: 860, y: 520, gapMs: 260 },
      { x: 1020, y: 430, gapMs: 420 },
    ]
    const script: Script = { id: 's1', name: '副本刷怪', steps, createdAt: Date.now() }
    mockState.config.mode = 'script'
    mockState.config.scripts = [script]
    mockState.config.activeScriptId = 's1'
    mockState.engine.phase = 'running'
    mockState.engine.stepTotal = steps.length
    mockState.engine.stepIndex = 1
    mockState.engine.clicks = 64
    mockState.engine.elapsedMs = 18_000
  }
}
if (mode === 'script' && !params.has('overlay')) {
  const steps = [
    { x: 700, y: 420, gapMs: 300 },
    { x: 860, y: 520, gapMs: 260 },
    { x: 1020, y: 430, gapMs: 420 },
  ]
  const script: Script = { id: 's1', name: '副本刷怪', steps, createdAt: Date.now() }
  const script2: Script = { id: 's2', name: '采集循环', steps: steps.slice(0, 2), createdAt: Date.now() }
  mockState.config.mode = 'script'
  mockState.config.scripts = [script, script2]
  mockState.config.activeScriptId = 's1'
}

let ticker: ReturnType<typeof setInterval> | null = null

function stopTicker() {
  if (ticker !== null) {
    clearInterval(ticker)
    ticker = null
  }
}

function start() {
  mockState.engine.phase = 'countdown'
  mockState.engine.countdown = mockState.config.countdownSecs
  mockState.engine.clicks = 0
  mockState.engine.elapsedMs = 0
  mockState.engine.stopReason = null
  let remaining = mockState.config.countdownSecs
  stopTicker()
  ticker = setInterval(() => {
    if (mockState.engine.phase !== 'countdown') {
      if (mockState.engine.phase === 'idle') stopTicker()
      return
    }
    remaining -= 1
    mockState.engine.countdown = Math.max(remaining, 0)
    if (remaining > 0) return
    mockState.engine.phase = 'running'
    const startedAt = Date.now()
    stopTicker()
    ticker = setInterval(() => {
      mockState.engine.clicks += Math.max(1, Math.round(mockState.config.cps / 4))
      mockState.engine.elapsedMs = Date.now() - startedAt
      mockState.engine.actualCps = mockState.config.cps
      if (mockState.config.mode === 'script') {
        mockState.engine.stepTotal = mockState.config.scripts[0]?.steps.length ?? 0
        mockState.engine.stepIndex =
          (mockState.engine.stepIndex + 1) % Math.max(1, mockState.engine.stepTotal)
      }
    }, 250)
  }, 1000)
}

function stop(reason: string) {
  stopTicker()
  mockState.engine.phase = 'idle'
  mockState.engine.countdown = 0
  mockState.engine.actualCps = 0
  mockState.engine.stopReason = reason
}

export async function mockCall(command: string, args?: Record<string, unknown>): Promise<unknown> {
  switch (command) {
    case 'get_state': {
      const notice = mockState.notice
      mockState.notice = null
      return { ...mockState, notice }
    }
    case 'get_overlay_state':
      return {
        config: mockState.config,
        engine: mockState.engine,
        mode: mockState.overlayMode,
        geometry: mockState.overlay,
      }
    case 'start_engine':
      start()
      return undefined
    case 'stop_engine':
      stop('手动停止')
      return undefined
    case 'set_point': {
      const { x, y } = args as { x: number; y: number }
      mockState.config.point = { x: Math.round(x), y: Math.round(y) }
      return undefined
    }
    case 'set_point_at_cursor':
      mockState.config.point = { x: 700, y: 480 }
      return undefined
    case 'set_cps':
      mockState.config.cps = (args as { cps: number }).cps
      return undefined
    case 'set_options':
      Object.assign(mockState.config, args)
      return undefined
    case 'set_theme':
      mockState.config.theme = (args as { theme: Theme }).theme
      return undefined
    case 'set_mode':
      mockState.config.mode = (args as { mode: RunMode }).mode
      return undefined
    case 'set_hud_offset': {
      const { dx, dy } = args as { dx: number; dy: number }
      mockState.config.hudOffset = [dx, dy]
      return undefined
    }
    case 'reset_hud_offset':
      mockState.config.hudOffset = null
      return undefined
    case 'set_target_window':
      mockState.config.targetHwnd = (args as { hwnd: number | null }).hwnd
      return undefined
    case 'preview_click':
      return undefined
    case 'restart_as_admin':
      return undefined
    case 'pick_target_window':
      mockState.config.targetHwnd = 123456
      return 'WeChatAppEx'
    case 'set_monitor':
      mockState.config.monitorIndex = (args as { index: number }).index
      return undefined
    case 'set_marker_visible':
      mockState.config.showMarker = Boolean((args as { visible: boolean }).visible)
      mockState.overlayMode = mockState.config.showMarker ? 'passive' : 'hidden'
      return mockState.config.showMarker
    case 'toggle_marker':
      mockState.config.showMarker = !mockState.config.showMarker
      mockState.overlayMode = mockState.config.showMarker ? 'passive' : 'hidden'
      return mockState.config.showMarker
    case 'set_overlay_rects':
      return undefined
    case 'refresh_monitors':
      return mockState.monitors
    case 'start_recording':
      mockState.isRecording = true
      mockState.recording = null
      return undefined
    case 'stop_recording': {
      mockState.isRecording = false
      const summary: RecordingSummary = {
        count: 3,
        avgGapMs: 240,
        cps: 4.2,
        x: mockState.config.point.x,
        y: mockState.config.point.y,
        spreadPx: 12,
        steps: [
          { x: 700, y: 420, gapMs: 200 },
          { x: 760, y: 460, gapMs: 260 },
          { x: 820, y: 500, gapMs: 240 },
        ],
      }
      mockState.recording = summary
      return summary
    }
    case 'discard_recording':
      mockState.isRecording = false
      mockState.recording = null
      return undefined
    case 'apply_recording': {
      const summary = mockState.recording
      if (summary) {
        mockState.config.point = { x: summary.x, y: summary.y }
        mockState.config.cps = summary.cps
      }
      mockState.recording = null
      return summary
    }
    case 'save_script': {
      const summary = mockState.recording
      const script: Script = {
        id: `s${Date.now()}`,
        name: (args as { name: string }).name || '新脚本',
        steps: summary?.steps ?? [],
        createdAt: Date.now(),
      }
      mockState.config.scripts = [...mockState.config.scripts, script]
      mockState.config.activeScriptId = script.id
      mockState.config.mode = 'script'
      mockState.recording = null
      return script
    }
    case 'delete_script': {
      const id = (args as { id: string }).id
      mockState.config.scripts = mockState.config.scripts.filter((s) => s.id !== id)
      if (mockState.config.activeScriptId === id) {
        mockState.config.activeScriptId = mockState.config.scripts[0]?.id ?? null
      }
      return undefined
    }
    case 'select_script':
      mockState.config.activeScriptId = (args as { id: string }).id
      return undefined
    case 'rename_script': {
      const { id, name } = args as { id: string; name: string }
      mockState.config.scripts = mockState.config.scripts.map((s) =>
        s.id === id ? { ...s, name } : s,
      )
      return undefined
    }
    default:
      return undefined
  }
}
