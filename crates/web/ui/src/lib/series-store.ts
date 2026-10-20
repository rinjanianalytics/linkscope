import { useEffect, useSyncExternalStore } from 'react'
import { api, ApiError, type Period, type Series } from './api'
import { usePageVisible } from './hooks'

/**
 * Shared, rate-limited cache for small series requests (card sparklines).
 *
 * - At most MAX_CONCURRENT requests run at once; the rest wait in a FIFO.
 * - A card that scrolls away before its turn is dropped from the queue.
 * - Results are cached per (name, period, maxx) and reused for STALE_MS,
 *   so scrolling back, paging or re-filtering does not refetch.
 */
const MAX_CONCURRENT = 6
const STALE_MS = 60_000

export type SeriesState =
  | { status: 'idle' | 'queued' | 'loading'; data?: Series }
  | { status: 'ok'; data: Series }
  | { status: 'empty'; data?: undefined }
  | { status: 'error'; data?: Series; error: string }

interface Entry {
  state: SeriesState
  fetchedAt: number
  listeners: Set<() => void>
  wanted: number
}

const entries = new Map<string, Entry>()
const queue: string[] = []
let active = 0

const IDLE: SeriesState = { status: 'idle' }

function entry(key: string): Entry {
  let e = entries.get(key)
  if (!e) {
    e = { state: IDLE, fetchedAt: 0, listeners: new Set(), wanted: 0 }
    entries.set(key, e)
  }
  return e
}

function set(key: string, state: SeriesState) {
  const e = entry(key)
  e.state = state
  e.listeners.forEach((l) => l())
}

function parse(key: string): [string, Period, number] {
  const [period, maxx, ...rest] = key.split('|')
  return [rest.join('|'), period as Period, Number(maxx)]
}

function pump() {
  while (active < MAX_CONCURRENT && queue.length) {
    const key = queue.shift()!
    const e = entry(key)
    if (e.wanted === 0) {
      set(key, e.state.data ? { status: 'ok', data: e.state.data } : IDLE)
      continue
    }
    active++
    set(key, { status: 'loading', data: e.state.data })
    const [name, period, maxx] = parse(key)
    api
      .series(name, period, maxx)
      .then((data) => set(key, { status: 'ok', data }))
      .catch((err: unknown) => {
        if (err instanceof ApiError && err.status === 404) set(key, { status: 'empty' })
        else set(key, { status: 'error', data: e.state.data, error: String((err as Error).message ?? err) })
      })
      .finally(() => {
        e.fetchedAt = Date.now()
        active--
        pump()
      })
  }
}

function request(key: string) {
  const e = entry(key)
  if (e.state.status === 'queued' || e.state.status === 'loading') return
  if (Date.now() - e.fetchedAt < STALE_MS && e.state.status !== 'idle') return
  queue.push(key)
  set(key, { status: 'queued', data: e.state.data } as SeriesState)
  pump()
}

/**
 * Subscribe to a series. Fetches only while `enabled` (the card is on screen)
 * and the tab is visible, and refreshes every minute in that state.
 */
export function useQueuedSeries(name: string, period: Period, maxx: number, enabled: boolean): SeriesState {
  const key = `${period}|${maxx}|${name}`
  const visible = usePageVisible()
  const state = useSyncExternalStore(
    (cb) => {
      const e = entry(key)
      e.listeners.add(cb)
      return () => e.listeners.delete(cb)
    },
    () => entry(key).state,
  )

  useEffect(() => {
    if (!enabled || !visible) return
    const e = entry(key)
    e.wanted++
    request(key)
    const id = window.setInterval(() => request(key), STALE_MS)
    return () => {
      e.wanted--
      window.clearInterval(id)
    }
  }, [key, enabled, visible])

  return state
}
