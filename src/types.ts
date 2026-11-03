export type Phase = 'idle' | 'countdown' | 'running'
export type OverlayMode = 'hidden' | 'passive'
export type Theme = 'dark' | 'light' | 'system'
export type RunMode = 'point' | 'script'
export type ClickMode = 'standard' | 'message' | 'both'

export interface ClickPoint {
  x: number
  y: number
}

export interface ScriptStep {
  x: number
  y: number
  gapMs: number
}

export interface Script {
  id: string
  name: string
  steps: ScriptStep[]
  createdAt: number
}

export interface AppConfig {
  theme: Theme
  mode: RunMode
  point: ClickPoint
  cps: number
  clickMode: ClickMode
  holdMs: number
  countdownSecs: number
  cornerFailsafe: boolean
  windowGuard: boolean
  showMarker: boolean
  targetHwnd: number | null
  hudOffset: [number, number] | null
  monitorIndex: number | null
  scripts: Script[]
  activeScriptId: string | null
}

export interface EngineStatus {
  phase: Phase
  countdown: number
  clicks: number
  elapsedMs: number
  actualCps: number
  stopReason: string | null
  targetTitle: string | null
  stepIndex: number
  stepTotal: number
}

export interface RecordingSummary {
  count: number
  avgGapMs: number
  cps: number
  x: number
  y: number
  spreadPx: number
  steps: ScriptStep[]
}

export interface MonitorInfo {
  name: string
  x: number
  y: number
  width: number
  height: number
  scaleFactor: number
  isPrimary: boolean
}

export interface OverlayGeometry {
  originX: number
  originY: number
  width: number
  height: number
  scaleFactor: number
}

export interface AppStateDto {
  config: AppConfig
  engine: EngineStatus
  recording: RecordingSummary | null
  isRecording: boolean
  overlayMode: OverlayMode
  overlay: OverlayGeometry | null
  monitors: MonitorInfo[]
  failedHotkeys: string[]
  elevated: boolean
  notice: string | null
  version: string
  appName: string
  description: string
  exeDir: string
  logPath: string
}

export interface OverlayStateDto {
  config: AppConfig
  engine: EngineStatus
  mode: OverlayMode
  geometry: OverlayGeometry | null
}

/** 可交互元素的实测视觉矩形（屏幕绝对物理像素，中心 + 半宽半高）。 */
export interface OverlayRect {
  x: number
  y: number
  halfW: number
  halfH: number
}

/** 右键菜单项的实测视觉矩形。 */
export interface OverlayMenuItem extends OverlayRect {
  id: string
}

export interface OverlayRects {
  hud: OverlayRect | null
  marker: OverlayRect | null
  menuItems: OverlayMenuItem[]
}
