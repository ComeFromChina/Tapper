import { useCallback, useEffect, useRef, useState } from 'react'

import { api } from '../lib/api'
import type { AppStateDto } from '../types'

/** 主面板状态轮询：Rust 是唯一真值来源，前端不做本地状态机。 */
export function useEngineState(intervalMs = 200) {
  const [state, setState] = useState<AppStateDto | null>(null)
  const [notice, setNotice] = useState<string | null>(null)
  const alive = useRef(true)

  const refresh = useCallback(async () => {
    try {
      const next = await api.getState()
      if (!alive.current) return
      setState(next)
      if (next.notice) setNotice(next.notice)
    } catch {
      // 启动初期 IPC 尚未就绪，忽略即可。
    }
  }, [])

  useEffect(() => {
    alive.current = true
    void refresh()
    const id = setInterval(() => {
      void refresh()
    }, intervalMs)
    return () => {
      alive.current = false
      clearInterval(id)
    }
  }, [refresh, intervalMs])

  useEffect(() => {
    if (!notice) return
    const id = setTimeout(() => setNotice(null), 6000)
    return () => clearTimeout(id)
  }, [notice])

  return { state, notice, setNotice, refresh }
}
