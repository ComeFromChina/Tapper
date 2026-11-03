import { useEffect, useMemo, useState } from 'react'
import { listen } from '@tauri-apps/api/event'

import { useEngineState } from '../hooks/useEngineState'
import { api, isTauri } from '../lib/api'
import { clampCps, formatDuration, formatNumber, intervalLabel } from '../lib/format'
import { applyTheme } from '../lib/theme'
import type { AppStateDto, ClickMode, RunMode, Theme } from '../types'
import {
  AlertIcon,
  CrosshairIcon,
  GaugeIcon,
  PlayIcon,
  PlusIcon,
  MinusIcon,
  RecordIcon,
  ScreenIcon,
  ShieldIcon,
  StopIcon,
} from './Icons'
import { AboutPanel } from './AboutPanel'
import { TitleBar } from './TitleBar'
import { Toggle } from './Toggle'

const THEME_LABEL: Record<Theme, string> = { dark: '深色', light: '浅色', system: '跟随系统' }
const CLICK_MODE_LABEL: Record<ClickMode, string> = {
  standard: '标准（模拟鼠标）',
  message: '兼容（不抢鼠标）',
  both: '双发（两种都发）',
}
const CLICK_MODE_HINT: Record<ClickMode, string> = {
  standard: '先移动光标再真人式点击，最可靠；连点期间鼠标被占用，别手动操作',
  message: '不移动光标，只点启动时那个窗口；目标在后台时不会把它顶到前台，你在别的窗口照常打字；你按住鼠标的每一拍它都让开（松手就继续），你的点击和拖动不会被吃掉；被别的应用盖住就停止并提示',
  both: '标准和兼容各发一次，适合拿不准目标程序认哪种的场景；可能被算成两下，出现重复点击就改回标准',
}
const THEME_ORDER: Theme[] = ['system', 'dark', 'light']

export function MainApp() {
  const { state, notice, setNotice, refresh } = useEngineState()
  const [cpsDraft, setCpsDraft] = useState<number | null>(null)
  const [busy, setBusy] = useState(false)
  const [confirmQuit, setConfirmQuit] = useState(false)
  const [scriptName, setScriptName] = useState('')
  const [aboutOpen, setAboutOpen] = useState(false)

  useEffect(() => {
    void api.frontendReady()
    void api.refreshMonitors().then(() => refresh())
  }, [refresh])

  useEffect(() => {
    if (!isTauri) return
    let unlisten: (() => void) | undefined
    void listen('request-quit-confirm', () => setConfirmQuit(true)).then((fn) => {
      unlisten = fn
    })
    return () => unlisten?.()
  }, [])

  const theme = state?.config.theme ?? 'system'
  // 悬浮层只铺一块屏，选中的是显示器列表里的序号；没选过就用主屏。
  const selectedMonitor =
    state?.config.monitorIndex ?? Math.max(0, state?.monitors.findIndex((m) => m.isPrimary) ?? 0)
  useEffect(() => {
    applyTheme(theme)
  }, [theme])

  const activeScript = state?.config.scripts.find((s) => s.id === state.config.activeScriptId) ?? null
  const scriptDuration = useMemo(
    () => (activeScript ? activeScript.steps.reduce((sum, s) => sum + s.gapMs, 0) : 0),
    [activeScript],
  )

  if (!state) {
    return (
      <div className="app-shell">
        <TitleBar
          subtitle="单点 / 脚本"
          themeLabel="跟随系统"
          onTheme={() => undefined}
          onAbout={() => undefined}
        />
        <div className="loading">
          <span className="spinner" />
          正在连接引擎…
        </div>
      </div>
    )
  }

  const phase = state.engine.phase
  const active = phase !== 'idle'
  const cps = cpsDraft ?? state.config.cps
  const mode = state.config.mode

  async function run(action: () => Promise<unknown>) {
    if (busy) return
    setBusy(true)
    try {
      await action()
    } catch (err) {
      setNotice(String(err))
    } finally {
      setBusy(false)
      void refresh()
    }
  }

  async function commitCps(value: number) {
    const next = clampCps(value)
    setCpsDraft(next)
    try {
      await api.setCps(next)
    } catch (err) {
      setNotice(String(err))
    }
    await refresh()
    setCpsDraft(null)
  }

  function patchOptions(patch: Partial<AppStateDto['config']>) {
    const current = state?.config
    if (!current) return Promise.resolve()
    return run(() =>
      api.setOptions({
        countdownSecs: patch.countdownSecs ?? current.countdownSecs,
        cornerFailsafe: patch.cornerFailsafe ?? current.cornerFailsafe,
        windowGuard: patch.windowGuard ?? current.windowGuard,
        holdMs: patch.holdMs ?? current.holdMs,
        clickMode: patch.clickMode ?? current.clickMode,
      }),
    )
  }

  function cycleTheme() {
    const next = THEME_ORDER[(THEME_ORDER.indexOf(theme) + 1) % THEME_ORDER.length]
    applyTheme(next)
    void run(() => api.setTheme(next))
  }

  const statusText =
    phase === 'running'
      ? mode === 'script'
        ? `运行脚本 · ${state.engine.stepIndex + 1}/${state.engine.stepTotal}`
        : '运行中'
      : phase === 'countdown'
        ? state.engine.countdown > 0
          ? `倒计时 ${state.engine.countdown}`
          : '等待目标窗口'
        : '就绪'

  return (
    <div className="app-shell">
      <TitleBar
        subtitle={mode === 'script' ? '脚本模式' : '单点模式'}
        themeLabel={THEME_LABEL[theme]}
        onTheme={cycleTheme}
        onAbout={() => setAboutOpen(true)}
      />

      <main className="content">
        {state.failedHotkeys.length > 0 && (
          <div className="banner is-danger">
            <AlertIcon />
            <div>
              <b>热键未完全生效</b>
              <span>{state.failedHotkeys.join('、')} 被其他程序占用，请关闭冲突程序后重启 Tapper。</span>
            </div>
          </div>
        )}

        <section className={`hero phase-${phase}`}>
          <div className="hero-top">
            <span className={`status-pill phase-${phase}`}>
              <i className="dot" />
              {statusText}
            </span>
            <span className="hero-keys">
              <kbd>F6</kbd> 启停 <kbd>F7</kbd> 急停 <kbd>F9</kbd> 标记
            </span>
          </div>

          <div className="hero-metrics">
            <div className="metric">
              <span className="metric-value">{formatNumber(state.engine.clicks)}</span>
              <span className="metric-label">累计点击</span>
            </div>
            <div className="metric-divider" />
            <div className="metric">
              <span className="metric-value">{formatDuration(state.engine.elapsedMs)}</span>
              <span className="metric-label">运行时长</span>
            </div>
            <div className="metric-divider" />
            <div className="metric is-cps">
              <span className="metric-value">
                {state.engine.actualCps > 0 ? state.engine.actualCps.toFixed(1) : '—'}
              </span>
              <span className="metric-label">实际 CPS</span>
            </div>
          </div>

          <div className="hero-actions">
            <button
              className="btn btn-primary"
              disabled={active || busy || (mode === 'script' && !activeScript)}
              onClick={() => void run(() => api.start())}
            >
              <PlayIcon size={16} />
              {phase === 'countdown'
                ? state.engine.countdown > 0
                  ? `倒计时 ${state.engine.countdown}`
                  : '等待目标窗口'
                : mode === 'script'
                  ? '启动脚本'
                  : '启动点击'}
              <kbd>F6</kbd>
            </button>
            <button className="btn btn-danger" disabled={!active} onClick={() => void run(() => api.stop())}>
              <StopIcon />
              紧急停止
              <kbd>F7</kbd>
            </button>
          </div>

          {mode === 'script' && activeScript && (
            <p className="hero-note">
              当前脚本 · <b>{activeScript.name}</b> · {activeScript.steps.length} 步 · 单圈{' '}
              {(scriptDuration / 1000).toFixed(1)}s
            </p>
          )}
          {phase === 'countdown' && (
            <p className="hero-note">
              {state.engine.countdown > 0 ? '倒计时内切到游戏窗口，启动后会锁定它' : '正在等待外部窗口成为前台…'}
            </p>
          )}
          {phase === 'running' && state.engine.targetTitle && (
            <p className="hero-note">
              已锁定窗口 · <b>{state.engine.targetTitle}</b>
            </p>
          )}
          {phase === 'idle' && state.engine.stopReason && (
            <p className="hero-note is-warn">
              <AlertIcon size={13} />
              {state.engine.stopReason}
            </p>
          )}
        </section>

        <div className="mode-switch" role="tablist">
          {(['point', 'script'] as RunMode[]).map((m) => (
            <button
              key={m}
              type="button"
              role="tab"
              aria-selected={mode === m}
              className={mode === m ? 'is-active' : ''}
              disabled={active}
              onClick={() => void run(() => api.setMode(m))}
            >
              {m === 'point' ? '单点模式' : '脚本模式'}
            </button>
          ))}
        </div>

        {mode === 'point' ? (
          <>
            <section className="card">
              <div className="card-head">
                <h2>
                  <CrosshairIcon size={15} />
                  点击目标
                </h2>
                {state.monitors.length > 1 ? (
                  <select
                    className="select"
                    value={String(selectedMonitor)}
                    onChange={(e) => void run(() => api.setMonitor(Number(e.target.value)))}
                  >
                    {state.monitors.map((m, i) => (
                      <option key={`${m.name}-${i}`} value={String(i)}>
                        {m.isPrimary ? '主屏' : `副屏 ${i}`} · {m.width}×{m.height}（{m.x},{m.y}）
                      </option>
                    ))}
                  </select>
                ) : (
                  <span className="chip">{state.monitors[0]?.width ?? 0}×{state.monitors[0]?.height ?? 0}</span>
                )}
              </div>

              <div className="coord-row">
                <div className="coord">
                  <span>X</span>
                  <b>{state.config.point.x}</b>
                </div>
                <div className="coord">
                  <span>Y</span>
                  <b>{state.config.point.y}</b>
                </div>
                <button
                  className="btn btn-ghost coord-btn"
                  disabled={active || busy}
                  onClick={() => void run(() => api.setPointAtCursor())}
                >
                  <ScreenIcon size={15} />
                  在鼠标处定位
                </button>
              </div>
              {state.monitors.length > 1 && (
                <span className="hint inline">
                  多屏：把标记或状态条直接拖过屏幕边缘，整块悬浮层会跟过去。
                  <b>拖状态条</b>过去时标记放到那块屏正中；<b>拖标记</b>过去时状态条回到那块屏顶部中间。
                  也可以用上面的下拉切换
                </span>
              )}
              <div className="row">
                <button
                  className="btn btn-ghost btn-sm"
                  disabled={active || busy}
                  onClick={() => {
                    setNotice('2 秒后会在当前点位点一下，请先切到目标窗口')
                    void run(() => api.previewClick())
                  }}
                >
                  测试一次点击
                </button>
                <span className="hint inline">点不动时切到「兼容（不抢鼠标）」再试</span>
              </div>
              <p className="hint">直接拖动屏幕上的手图标即可移动落点，无需打开面板。</p>

              <div className="switch-row">
                <div className="switch-text">
                  <b>显示点位标记</b>
                  <span>随时可拖；右键标记或按 F9 可快速隐藏</span>
                </div>
                <Toggle
                  label="显示点位标记"
                  checked={state.config.showMarker}
                  onChange={(v) => void run(() => api.setMarkerVisible(v))}
                />
              </div>
            </section>

            <section className="card">
              <div className="card-head">
                <h2>
                  <GaugeIcon size={15} />
                  点击速率
                </h2>
                <span className="mono muted">{intervalLabel(cps)}</span>
              </div>

              <div className="rate-row">
                <button className="step-btn" type="button" onClick={() => void commitCps(cps - 1)}>
                  <MinusIcon />
                </button>
                <div className="rate-value">
                  <input
                    type="number"
                    min={1}
                    max={200}
                    step={0.5}
                    value={cps}
                    onChange={(e) => setCpsDraft(Number(e.target.value))}
                    onBlur={(e) => void commitCps(Number(e.target.value))}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter') void commitCps(Number((e.target as HTMLInputElement).value))
                    }}
                  />
                  <span>CPS</span>
                </div>
                <button className="step-btn" type="button" onClick={() => void commitCps(cps + 1)}>
                  <PlusIcon />
                </button>
              </div>

              <input
                className="slider"
                type="range"
                min={1}
                max={200}
                step={0.5}
                value={cps}
                onChange={(e) => setCpsDraft(Number(e.target.value))}
                onPointerUp={() => void commitCps(cps)}
                onKeyUp={() => void commitCps(cps)}
              />

              <div className="presets">
                {[1, 4, 8, 20, 50, 100].map((value) => (
                  <button
                    key={value}
                    type="button"
                    className={`preset ${Math.abs(cps - value) < 0.05 ? 'is-active' : ''}`}
                    onClick={() => void commitCps(value)}
                  >
                    {value}
                  </button>
                ))}
              </div>

              {cps > 50 && (
                <p className="hint is-warn">
                  高于 50 CPS 会自动切到高频模式（更短的按下、跳过移动稳定期）。
                  实际是否能点出来取决于目标程序：渲染 60 帧的游戏通常最多处理约 60 次/秒，
                  超出部分会被合并或丢弃。
                </p>
              )}
              <div className="inline-field">
                <span>按下保持</span>
                <input
                  type="number"
                  min={0}
                  max={1000}
                  step={10}
                  defaultValue={state.config.holdMs}
                  key={`hold-${state.config.holdMs}`}
                  onBlur={(e) => void patchOptions({ holdMs: Number((e.target as HTMLInputElement).value) })}
                />
                <span className="unit">ms</span>
                <em>0 = 瞬时点击</em>
              </div>
            </section>
          </>
        ) : (
          <section className="card">
            <div className="card-head">
              <h2>
                <RecordIcon size={13} />
                已保存脚本
              </h2>
              <span className="chip">{state.config.scripts.length} 份</span>
            </div>
            {state.config.scripts.length === 0 ? (
              <p className="hint">
                还没有脚本。先在游戏里按 <kbd>F8</kbd> 录一段，停止后把录制保存成脚本。
              </p>
            ) : (
              <ul className="script-list">
                {state.config.scripts.map((script) => {
                  const duration = script.steps.reduce((sum, s) => sum + s.gapMs, 0)
                  const selected = script.id === state.config.activeScriptId
                  return (
                    <li key={script.id} className={`script-item ${selected ? 'is-active' : ''}`}>
                      <button
                        type="button"
                        className="script-pick"
                        disabled={active}
                        onClick={() => void run(() => api.selectScript(script.id))}
                      >
                        <span className="script-radio" />
                        <span className="script-name">{script.name}</span>
                        <span className="script-meta">
                          {script.steps.length} 步 · {(duration / 1000).toFixed(1)}s
                        </span>
                      </button>
                      <button
                        type="button"
                        className="icon-btn"
                        title="删除脚本"
                        disabled={active}
                        onClick={() => void run(() => api.deleteScript(script.id))}
                      >
                        ×
                      </button>
                    </li>
                  )
                })}
              </ul>
            )}
            <p className="hint">脚本会按录制时的坐标序列与节奏循环播放，鼠标位置会依次移动。</p>
          </section>
        )}

        <section className="card">
          <div className="card-head">
            <h2>
              <ShieldIcon size={15} />
              安全防护
            </h2>
            <span className="chip is-ok">F7 停止</span>
          </div>

          <div className="switch-row">
            <div className="switch-text">
              <b>甩角急停</b>
              <span>鼠标到屏幕左上角立即停止；分屏使用时建议关闭</span>
            </div>
            <Toggle
              label="甩角急停"
              checked={state.config.cornerFailsafe}
              onChange={(v) => void patchOptions({ cornerFailsafe: v })}
            />
          </div>

          <div className="switch-row">
            <div className="switch-text">
              <b>目标窗口</b>
              <span>
                {state.config.targetHwnd
                  ? '已固定目标窗口（停止判定以它为准）'
                  : '未指定：按连点坐标所在的窗口判定'}
              </span>
            </div>
            <button
              className="btn btn-ghost btn-xs"
              disabled={active || busy}
              onClick={() =>
                void run(async () => {
                  if (state.config.targetHwnd) {
                    await api.setTargetWindow(null)
                    return
                  }
                  const title = await api.pickTargetWindow()
                  if (title) setNotice(`已指定目标窗口：${title}`)
                })
              }
            >
              {state.config.targetHwnd ? '清除' : '指定窗口'}
            </button>
          </div>

          <div className="switch-row">
            <div className="switch-text">
              <b>只按快捷键停止</b>
              <span>移动鼠标、切换窗口都不会停止；只有 F7 或面板上的停止按钮能停</span>
            </div>
            <Toggle
              label="只按快捷键停止"
              checked={state.config.windowGuard}
              onChange={(v) => void patchOptions({ windowGuard: v })}
            />
          </div>

          <div className="switch-row column">
            <div className="switch-text">
              <b>点击方式</b>
              <span>{CLICK_MODE_HINT[state.config.clickMode]}</span>
            </div>
            <div className="segmented">
              {(Object.keys(CLICK_MODE_LABEL) as ClickMode[]).map((m) => (
                <button
                  key={m}
                  type="button"
                  className={state.config.clickMode === m ? 'is-active' : ''}
                  onClick={() => void patchOptions({ clickMode: m })}
                >
                  {CLICK_MODE_LABEL[m]}
                </button>
              ))}
            </div>
          </div>

          <div className="switch-row">
            <div className="switch-text">
              <b>运行权限</b>
              <span>
                {state.elevated
                  ? '已是管理员权限，可以点击以管理员运行的目标程序'
                  : '普通权限：目标程序若以管理员运行，会「光标动了但没反应」'}
              </span>
            </div>
            <button
              className="btn btn-ghost btn-xs"
              disabled={state.elevated || busy}
              title={state.elevated ? '当前已是管理员权限，无需重启' : '提权后重新启动 Tapper'}
              onClick={() => void api.restartAsAdmin()}
            >
              {state.elevated ? '已是管理员' : '以管理员重启'}
            </button>
          </div>

          <div className="switch-row">
            <div className="switch-text">
              <b>启动倒计时</b>
              <span>留出切换到游戏窗口的时间</span>
            </div>
            <select
              className="select"
              value={state.config.countdownSecs}
              onChange={(e) => void patchOptions({ countdownSecs: Number(e.target.value) })}
            >
              <option value={0}>立即</option>
              <option value={1}>1 秒</option>
              <option value={3}>3 秒</option>
              <option value={5}>5 秒</option>
            </select>
          </div>
        </section>

        <section className="card">
          <div className="card-head">
            <h2>
              <RecordIcon size={13} />
              动作录制
            </h2>
            <kbd>F8</kbd>
          </div>

          {state.isRecording ? (
            <div className="recording-live">
              <span className="rec-dot" />
              <div>
                <b>正在录制真实点击</b>
                <span>在游戏里正常点几下，再按 F8 结束</span>
              </div>
              <button className="btn btn-soft btn-sm" onClick={() => void run(() => api.stopRecording())}>
                结束录制
              </button>
            </div>
          ) : state.recording ? (
            <div className="recording-result">
              <div className="rec-stats">
                <div>
                  <b>{state.recording.count}</b>
                  <span>点击</span>
                </div>
                <div>
                  <b>{state.recording.avgGapMs.toFixed(0)}</b>
                  <span>平均间隔 ms</span>
                </div>
                <div>
                  <b>{state.recording.spreadPx}</b>
                  <span>分散 px</span>
                </div>
              </div>
              <div className="save-row">
                <input
                  className="text-input"
                  placeholder="脚本名称，如：副本刷怪"
                  value={scriptName}
                  maxLength={24}
                  onChange={(e) => setScriptName(e.target.value)}
                />
                <button
                  className="btn btn-primary btn-sm"
                  disabled={busy}
                  onClick={() => void run(() => api.saveScript(scriptName))}
                >
                  保存为脚本
                </button>
              </div>
              <div className="row">
                {mode === 'point' && (
                  <button
                    className="btn btn-ghost btn-sm"
                    onClick={() => void run(() => api.applyRecording())}
                  >
                    应用到单点
                  </button>
                )}
                <button
                  className="btn btn-ghost btn-sm"
                  onClick={() => void run(() => api.discardRecording())}
                >
                  丢弃
                </button>
              </div>
            </div>
          ) : (
            <div className="recording-idle">
              <p>
                按 <kbd>F8</kbd> 或点下方按钮，在游戏里真实点几下，坐标和节奏都会录下来。
              </p>
              <button
                className="btn btn-soft btn-sm"
                disabled={active}
                onClick={() => void run(() => api.startRecording())}
              >
                <RecordIcon size={12} />
                开始录制
              </button>
            </div>
          )}
        </section>

        <footer className="app-footer">
          <span>Tapper v{state.version}</span>
          <div className="footer-actions">
            <button type="button" onClick={() => void api.openLogDir()}>
              日志
            </button>
            <button type="button" onClick={() => setConfirmQuit(true)}>
              退出
            </button>
          </div>
        </footer>
      </main>

      {aboutOpen && <AboutPanel state={state} onClose={() => setAboutOpen(false)} />}

      {notice && (
        <div className="toast" role="alert" onClick={() => setNotice(null)}>
          <AlertIcon size={14} />
          <span>{notice}</span>
        </div>
      )}

      {confirmQuit && (
        <div className="modal-backdrop">
          <div className="modal">
            <h3>退出 Tapper？</h3>
            <p>
              {active
                ? '点击正在运行，退出会立即停止并释放鼠标按键。'
                : '关闭面板只是收进托盘。要彻底退出请点“退出”。'}
            </p>
            <div className="modal-actions">
              <button className="btn btn-ghost btn-sm" onClick={() => setConfirmQuit(false)}>
                取消
              </button>
              {active && (
                <button
                  className="btn btn-danger btn-sm"
                  onClick={() => {
                    void api.stop()
                    void api.quit()
                  }}
                >
                  停止并退出
                </button>
              )}
              <button className="btn btn-primary btn-sm" onClick={() => void api.quit()}>
                退出
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
