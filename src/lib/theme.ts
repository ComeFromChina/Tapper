import type { Theme } from '../types'

const QUERY = '(prefers-color-scheme: light)'
let watcher: MediaQueryList | null = null
let listener: (() => void) | null = null

function resolve(theme: Theme): 'light' | 'dark' {
  if (theme === 'system') {
    return window.matchMedia(QUERY).matches ? 'light' : 'dark'
  }
  return theme
}

/** 应用主题；跟随系统时监听系统切换。 */
export function applyTheme(theme: Theme) {
  const set = () => {
    document.documentElement.dataset.theme = resolve(theme)
  }
  set()
  if (listener && watcher) {
    watcher.removeEventListener('change', listener)
    listener = null
    watcher = null
  }
  if (theme === 'system') {
    watcher = window.matchMedia(QUERY)
    listener = set
    watcher.addEventListener('change', listener)
  }
}
