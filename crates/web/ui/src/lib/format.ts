import type { Period, RateFormat } from './api'

/**
 * Format a rate the way MRTG does: scale by mul*factor, then step through
 * `units` while the value is >= 10*kilo (or the unit is a "-" placeholder).
 */
export function formatRate(v: number | null | undefined, f: RateFormat): string {
  if (v == null || !Number.isFinite(v)) return 'n/a'
  let x = v * f.mul * f.factor
  let i = 0
  const last = f.units.length - 1
  while ((x >= 10 * f.kilo || (f.units[i] ?? '').startsWith('-')) && i < last) {
    x /= f.kilo
    i++
  }
  return `${x.toFixed(f.integer ? 0 : 1)} ${f.units[i] ?? ''}`.trim()
}

/** Short tick label: the MRTG unit rules, but drop a trailing ".0" to save width. */
export function formatTick(v: number, f: RateFormat): string {
  return formatRate(v, f).replace(/\.0(?= |$)/, '')
}

const dtf = (opts: Intl.DateTimeFormatOptions) => new Intl.DateTimeFormat(undefined, opts)
const fmtHour = dtf({ hour: '2-digit', minute: '2-digit', hourCycle: 'h23' })
const fmtWeekday = dtf({ weekday: 'short' })
const fmtDate = dtf({ day: 'numeric', month: 'short' })
const fmtMonth = dtf({ month: 'short' })
const fmtFull = dtf({ weekday: 'short', day: 'numeric', month: 'short', year: 'numeric', hour: '2-digit', minute: '2-digit', hourCycle: 'h23' })
const fmtDay = dtf({ weekday: 'short', day: 'numeric', month: 'short', year: 'numeric' })

export const formatDateTime = (t: number) => fmtFull.format(t * 1000)

/** Tooltip label for a point: daily-resolution periods do not need a clock time. */
export const formatPointTime = (t: number, step: number) => (step >= 86400 ? fmtDay : fmtFull).format(t * 1000)

export function formatAxisTime(t: number, period: Period): string {
  const d = t * 1000
  switch (period) {
    case 'day':
      return fmtHour.format(d)
    case 'week':
      return fmtWeekday.format(d)
    case 'month':
      return fmtDate.format(d)
    case 'year':
      return fmtMonth.format(d)
  }
}

/** Tick positions on local-time boundaries: hours, days or months. */
export function timeTicks(min: number, max: number, period: Period): number[] {
  const ticks: number[] = []
  const d = new Date(min * 1000)
  const push = () => {
    const t = d.getTime() / 1000
    if (t >= min && t <= max) ticks.push(t)
  }
  if (period === 'day') {
    d.setMinutes(0, 0, 0)
    const step = 2
    d.setHours(Math.ceil(d.getHours() / step) * step)
    while (d.getTime() / 1000 <= max) {
      push()
      d.setHours(d.getHours() + step)
    }
  } else if (period === 'week' || period === 'month') {
    d.setHours(0, 0, 0, 0)
    d.setDate(d.getDate() + 1)
    const step = period === 'week' ? 1 : 3
    while (d.getTime() / 1000 <= max) {
      push()
      d.setDate(d.getDate() + step)
    }
  } else {
    d.setHours(0, 0, 0, 0)
    d.setDate(1)
    d.setMonth(d.getMonth() + 1)
    while (d.getTime() / 1000 <= max) {
      push()
      d.setMonth(d.getMonth() + 1)
    }
  }
  return ticks
}

/** "5 minute", "2 hour", "1 day": the averaging interval as MRTG words it. */
export function formatStep(step: number): string {
  if (step % 86400 === 0) return plural(step / 86400, 'day')
  if (step % 3600 === 0) return plural(step / 3600, 'hour')
  if (step % 60 === 0) return plural(step / 60, 'minute')
  return plural(step, 'second')
}
const plural = (n: number, unit: string) => `${n} ${unit}`

export function formatDuration(secs: number): string {
  if (!Number.isFinite(secs) || secs < 0) return 'n/a'
  const s = Math.floor(secs)
  const d = Math.floor(s / 86400)
  const h = Math.floor((s % 86400) / 3600)
  const m = Math.floor((s % 3600) / 60)
  if (d > 0) return `${d}d ${h}h`
  if (h > 0) return `${h}h ${m}m`
  if (m > 0) return `${m}m ${s % 60}s`
  return `${s}s`
}

export function formatMs(ms: number): string {
  return ms >= 1000 ? `${(ms / 1000).toFixed(1)} s` : `${Math.round(ms)} ms`
}

export function formatInterval(secs: number): string {
  if (secs % 3600 === 0) return `${secs / 3600} h`
  if (secs % 60 === 0) return `${secs / 60} min`
  return `${secs} s`
}

export function formatAgo(t: number, now = Date.now() / 1000): string {
  const diff = Math.max(0, now - t)
  if (diff < 45) return 'just now'
  return `${formatDuration(diff).split(' ')[0]} ago`
}

const nf = new Intl.NumberFormat()
export const formatCount = (n: number) => nf.format(n)

const pr = new Intl.PluralRules('en')

/** "1 poll cycle", "3 poll cycles". Pass `plural` for irregular nouns. */
export function formatCountOf(n: number, singular: string, plural = `${singular}s`): string {
  return `${formatCount(n)} ${pr.select(n) === 'one' ? singular : plural}`
}
