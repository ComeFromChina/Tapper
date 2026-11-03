import { getCurrentWindow } from '@tauri-apps/api/window'

import { api, isTauri } from '../lib/api'
import { CloseIcon, HandIcon, MinusIcon } from './Icons'

interface TitleBarProps {
  subtitle: string
  themeLabel: string
  onTheme: () => void
  onAbout: () => void
}

async function minimize() {
  if (!isTauri) return
  try {
    await getCurrentWindow().minimize()
  } catch {
    // 无权限时静默：不影响主流程。
  }
}

export function TitleBar({ subtitle, themeLabel, onTheme, onAbout }: TitleBarProps) {
  return (
    <header className="titlebar" data-tauri-drag-region>
      <div className="brand" data-tauri-drag-region>
        <div className="brand-mark">
          <HandIcon size={17} />
        </div>
        <div className="brand-text" data-tauri-drag-region>
          <span className="brand-name">Tapper</span>
          <span className="brand-sub">{subtitle}</span>
        </div>
      </div>
      <div className="titlebar-actions">
        <button className="theme-btn" type="button" title={`主题：${themeLabel}（点击切换）`} onClick={onTheme}>
          {themeLabel}
        </button>
        <button className="theme-btn" type="button" title="使用手册与常见问题" onClick={onAbout}>
          关于我们
        </button>
        <button className="win-btn" type="button" title="最小化" onClick={() => void minimize()}>
          <MinusIcon />
        </button>
        <button
          className="win-btn is-close"
          type="button"
          title="隐藏到托盘（点击继续运行）"
          onClick={() => void api.hideMainWindow()}
        >
          <CloseIcon />
        </button>
      </div>
    </header>
  )
}
