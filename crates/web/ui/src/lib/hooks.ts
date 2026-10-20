import { useCallback, useEffect, useRef, useState } from 'react'

/** Tracks `document.visibilityState`. */
export function usePageVisible(): boolean {
  const [visible, setVisible] = useState(() => document.visibilityState === 'visible')
  useEffect(() => {
    const on = () => setVisible(document.visibilityState === 'visible')
    document.addEventListener('visibilitychange', on)
    return () => document.removeEventListener('visibilitychange', on)
  }, [])
  return visible
}

export interface Polled<T> {
  data?: T
  error?: Error
  loading: boolean
  /** Unix ms of the last successful fetch. */
  updatedAt?: number
  refresh: () => void
}

/**
 * Fetch now, then every `intervalMs` while the tab is visible. When the tab
 * comes back after being hidden longer than the interval, refetch at once.
 * Keeps the last good data on error.
 */
export function usePolling<T>(
  fetcher: (signal: AbortSignal) => Promise<T>,
  intervalMs: number,
  deps: unknown[],
  enabled = true,
): Polled<T> {
  const visible = usePageVisible()
  const [state, setState] = useState<{ data?: T; error?: Error; loading: boolean; updatedAt?: number }>({
    loading: enabled,
  })
  const [tick, setTick] = useState(0)
  const lastRef = useRef(0)
  const fetcherRef = useRef(fetcher)
  fetcherRef.current = fetcher

  // Reset when the identity of what we fetch changes.
  useEffect(() => {
    lastRef.current = 0
    setState({ loading: enabled })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps)

  useEffect(() => {
    if (!enabled || !visible) return
    const ctrl = new AbortController()
    let timer: number | undefined

    const run = async () => {
      setState((s) => ({ ...s, loading: true }))
      try {
        const data = await fetcherRef.current(ctrl.signal)
        lastRef.current = Date.now()
        setState({ data, error: undefined, loading: false, updatedAt: lastRef.current })
      } catch (e) {
        if (ctrl.signal.aborted) return
        lastRef.current = Date.now()
        setState((s) => ({ ...s, error: e as Error, loading: false }))
      }
      if (!ctrl.signal.aborted) timer = window.setTimeout(run, intervalMs)
    }

    const wait = lastRef.current ? Math.max(0, lastRef.current + intervalMs - Date.now()) : 0
    timer = window.setTimeout(run, wait)
    return () => {
      ctrl.abort()
      window.clearTimeout(timer)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [enabled, visible, intervalMs, tick, ...deps])

  const refresh = useCallback(() => {
    lastRef.current = 0
    setTick((t) => t + 1)
  }, [])

  return { ...state, refresh }
}

/** True once (and while) the element is near the viewport. */
export function useInView<E extends Element>(rootMargin = '200px'): [(el: E | null) => void, boolean] {
  const [inView, setInView] = useState(false)
  const obs = useRef<IntersectionObserver | null>(null)
  const ref = useCallback(
    (el: E | null) => {
      obs.current?.disconnect()
      obs.current = null
      if (!el) return
      if (typeof IntersectionObserver === 'undefined') {
        setInView(true)
        return
      }
      obs.current = new IntersectionObserver(([entry]) => setInView(entry.isIntersecting), { rootMargin })
      obs.current.observe(el)
    },
    [rootMargin],
  )
  useEffect(() => () => obs.current?.disconnect(), [])
  return [ref, inView]
}

export function useDebounced<T>(value: T, ms: number): T {
  const [v, setV] = useState(value)
  useEffect(() => {
    const id = window.setTimeout(() => setV(value), ms)
    return () => window.clearTimeout(id)
  }, [value, ms])
  return v
}

/** Re-render every `ms` so relative times stay fresh. */
export function useNow(ms = 30_000): number {
  const [now, setNow] = useState(() => Date.now() / 1000)
  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now() / 1000), ms)
    return () => window.clearInterval(id)
  }, [ms])
  return now
}
