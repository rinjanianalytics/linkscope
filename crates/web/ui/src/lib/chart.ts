import type { Colour, RateFormat, SeriesPoint } from './api'

/** MRTG's built-in colours: in, out, in max, out max. */
const MRTG_DEFAULT_COLOURS = ['#00cc00', '#0000ff', '#006600', '#ff00ff']

export interface SeriesColours {
  in: string
  out: string
  inPeak: string
  outPeak: string
  custom: boolean
}

const THEME: SeriesColours = {
  in: 'hsl(var(--chart-in))',
  out: 'hsl(var(--chart-out))',
  inPeak: 'hsl(var(--chart-in-peak))',
  outPeak: 'hsl(var(--chart-out-peak))',
  custom: false,
}

/** Use the config's Colours[] only when they differ from MRTG's defaults. */
export function seriesColours(colours: Colour[] | undefined): SeriesColours {
  if (!colours?.length) return THEME
  const rgb = colours.map((c) => c.rgb.toLowerCase())
  const isDefault = MRTG_DEFAULT_COLOURS.every((d, i) => (rgb[i] ?? d) === d)
  if (isDefault) return THEME
  return {
    in: rgb[0] ?? THEME.in,
    out: rgb[1] ?? THEME.out,
    inPeak: rgb[2] ?? THEME.inPeak,
    outPeak: rgb[3] ?? THEME.outPeak,
    custom: true,
  }
}

const NICE_STEPS = [1, 1.2, 1.5, 2, 2.5, 3, 4, 5, 6, 8, 10]

/**
 * A "nice" upper bound in display units, so that `tics` equal divisions land
 * on readable numbers once formatted.
 */
export function niceMax(max: number, f: RateFormat, tics: number): number {
  // No traffic at all: one display unit per tick keeps the labels whole.
  if (!(max > 0)) return tics / (f.mul * f.factor || 1)
  const scale = f.mul * f.factor || 1
  const x = max * scale
  const pow = Math.pow(10, Math.floor(Math.log10(x)))
  const n = x / pow
  const pick = NICE_STEPS.find((s) => s >= n && ((s * 10) / tics) % 1 < 1e-9) ?? NICE_STEPS.find((s) => s >= n) ?? 10
  return (pick * pow) / scale
}

export function linearTicks(max: number, tics: number): number[] {
  const n = Math.max(1, Math.round(tics))
  return Array.from({ length: n + 1 }, (_, i) => (max * i) / n)
}

export interface PlotPoint {
  t: number
  in: number | null
  out: number | null
  in_max: number | null
  out_max: number | null
}

/** Prepare points for plotting; log scales cannot show zero, so it becomes a gap. */
export function plotPoints(points: SeriesPoint[], log: boolean): PlotPoint[] {
  if (!log) return points
  const pos = (v: number | null) => (v != null && v > 0 ? v : null)
  return points.map((p) => ({ t: p.t, in: pos(p.in), out: pos(p.out), in_max: pos(p.in_max), out_max: pos(p.out_max) }))
}

export function seriesMax(points: SeriesPoint[], keys: (keyof SeriesPoint)[]): number {
  let m = 0
  for (const p of points) for (const k of keys) {
    const v = p[k]
    if (v != null && v > m) m = v
  }
  return m
}
