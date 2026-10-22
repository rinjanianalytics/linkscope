/**
 * Types and fetchers for the LinkScope JSON API.
 *
 * `SanitizedHtml` marks strings the server has already sanitized. Only these
 * may reach `dangerouslySetInnerHTML` (see components/HtmlContent.tsx).
 */
export type SanitizedHtml = string & { readonly __sanitized: unique symbol }

export type Status = 'ok' | 'warn' | 'error' | 'stale' | 'pending'
export const STATUSES: Status[] = ['ok', 'warn', 'error', 'stale', 'pending']

export type Period = 'day' | 'week' | 'month' | 'year'
export const PERIODS: Period[] = ['day', 'week', 'month', 'year']

export interface Summary {
  version: string
  started: number
  now: number
  cycles: number
  last_cycle: { started: number; duration_ms: number; targets: number; ok: number; failed: number } | null
  interval_secs: number
  /** The configuration's file name (not its path). */
  config_name: string
  warnings: string[]
  targets_total: number
  targets_ok: number
  /** warn + failed + stale */
  targets_error: number
  targets_warn: number
  targets_failed: number
  targets_stale: number
  targets_pending: number
}

export interface RateFormat {
  mul: number
  factor: number
  kilo: number
  units: string[]
  integer: boolean
  nopercent: boolean
}

export interface Colour {
  name: string
  rgb: string
}

export interface TargetSummary {
  name: string
  title_html: SanitizedHtml
  directory: string
  status: Status
  error: string | null
  last_poll: number | null
  sys_name: string | null
  uptime: string | null
  max_bytes1: number
  max_bytes2: number
  format: RateFormat
  legend_i: SanitizedHtml
  legend_o: SanitizedHtml
  colours: Colour[]
  no_i: boolean
  no_o: boolean
  /** Latest rates in log units; MRTG stores 0 for unknown, so read them with `status`. */
  cur_in: number | null
  cur_out: number | null
  /** MRTG-formatted, e.g. "12.3 MB/s (4.5%)". */
  cur_in_text: string | null
  cur_out_text: string | null
  growright: boolean
  logscale: boolean
  /** Threshold bounds broken now: firing, or waiting out ThreshFor[]. */
  alerts: ThresholdAlert[]
}

export interface ThresholdAlert {
  direction: 'in' | 'out'
  bound: 'min' | 'max'
  /** As configured, e.g. "30%". */
  threshold: string
  /** The bound and the current rate, in log units like `cur_in`. */
  threshold_rate: number
  rate: number
  /** The rate in percent of MaxBytes, for a percent bound. */
  percent: number | null
  firing: boolean
  /** Broken for `polls` polls in a row; it fires at `polls_needed`. */
  polls: number
  polls_needed: number
  since: number
  fired_at: number | null
}

export interface Thresholds {
  min_i: string | null
  min_o: string | null
  max_i: string | null
  max_o: string | null
  desc: string | null
  /** ThreshFor[]: polls a bound must stay broken before it fires. */
  for_polls: number | null
  /** A ThreshProg*[] is set (which one is not given). */
  programs: boolean
  /** ThreshMailAddress[] is set (the address is not given). */
  mail: boolean
}

/** How a target is read; for gNMI, how the connection is protected. */
export interface SourceView {
  kind: 'snmp' | 'gnmi' | 'exec' | 'expression'
  tls: 'verify' | 'insecure' | 'off' | null
}

export interface TargetDetail extends TargetSummary {
  target_text: string
  source: SourceView
  page_top_html: SanitizedHtml
  page_foot_html: SanitizedHtml
  y_legend: string
  legends: SanitizedHtml[]
  periods: Period[]
  with_peak: Period[]
  unscaled: Period[]
  expscale: boolean
  secondmean: boolean
  dorelpercent: boolean
  avgpeak: boolean
  noinfo: boolean
  nolegend: boolean
  xsize: number
  ysize: number
  ytics: number
  yticsfactor: number
  abs_max: number | null
  timezone: string | null
  thresholds: Thresholds
}

export interface SeriesPoint {
  t: number
  in: number | null
  out: number | null
  in_max: number | null
  out_max: number | null
}

type StatKey = 'max_in' | 'max_out' | 'avg_in' | 'avg_out' | 'cur_in' | 'cur_out' | 'avg_max_in' | 'avg_max_out'

export type SeriesStats = Record<StatKey, number | null> & {
  max_pct: number | null
  avg_pct: number | null
  cur_pct: number | null
  text: Record<StatKey, string>
}

export interface Series {
  period: Period
  step: number
  now: number
  points: SeriesPoint[]
  peak: boolean
  fixed_max: number | null
  stats: SeriesStats
}

export class ApiError extends Error {
  constructor(
    public status: number,
    message: string,
  ) {
    super(message)
  }
}

async function getJson<T>(path: string, signal?: AbortSignal): Promise<T> {
  const res = await fetch(path, { signal, headers: { Accept: 'application/json' } })
  if (!res.ok) {
    const text = await res.text().catch(() => '')
    throw new ApiError(res.status, text || res.statusText)
  }
  return (await res.json()) as T
}

const enc = encodeURIComponent

/** The manual (`/api/docs`): its pages are Markdown rendered and sanitized by the server. */
export interface DocRef {
  slug: string
  title: string
}

export interface DocLanguage {
  code: string
  name: string
  title: string
  pages: DocRef[]
}

export interface DocsIndex {
  languages: DocLanguage[]
}

export interface DocPage {
  lang: string
  slug: string
  title: string
  html: SanitizedHtml
  headings: { level: number; id: string; text: string }[]
  prev: DocRef | null
  next: DocRef | null
}

export const api = {
  summary: (signal?: AbortSignal) => getJson<Summary>('/api/summary', signal),
  targets: (signal?: AbortSignal) => getJson<TargetSummary[]>('/api/targets', signal),
  target: (name: string, signal?: AbortSignal) => getJson<TargetDetail>(`/api/targets/${enc(name)}`, signal),
  series: (name: string, period: Period, maxx: number | undefined, signal?: AbortSignal) =>
    getJson<Series>(
      `/api/targets/${enc(name)}/series?period=${period}${maxx ? `&maxx=${Math.round(maxx)}` : ''}`,
      signal,
    ),
  docs: (signal?: AbortSignal) => getJson<DocsIndex>('/api/docs', signal),
  docPage: (lang: string, slug: string, signal?: AbortSignal) => getJson<DocPage>(`/api/docs/${enc(lang)}/${enc(slug)}`, signal),
}

export const targetHref = (name: string) => `/target/${enc(name)}`
