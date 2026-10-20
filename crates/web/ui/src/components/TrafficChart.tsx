import { memo, useMemo } from 'react'
import { Area, CartesianGrid, ComposedChart, Line, XAxis, YAxis } from 'recharts'
import {
  ChartContainer,
  ChartLegend,
  ChartLegendContent,
  ChartTooltip,
  ChartTooltipContent,
  type ChartConfig,
} from '@/components/ui/chart'
import { HtmlContent } from './HtmlContent'
import type { Period, RateFormat, SanitizedHtml, SeriesPoint } from '@/lib/api'
import { linearTicks, niceMax, plotPoints, seriesColours, seriesMax } from '@/lib/chart'
import { formatAxisTime, formatPointTime, formatRate, formatTick, timeTicks } from '@/lib/format'
import type { Colour } from '@/lib/api'
import { cn } from '@/lib/utils'

export interface TrafficChartProps {
  points: SeriesPoint[]
  step: number
  period: Period
  format: RateFormat
  colours: Colour[]
  legendIn: SanitizedHtml
  legendOut: SanitizedHtml
  /** MRTG default is to grow to the left (newest on the left). */
  growright: boolean
  logscale?: boolean
  fixedMax?: number | null
  peak?: boolean
  noI?: boolean
  noO?: boolean
  ytics?: number
  showLegend?: boolean
  className?: string
}

/**
 * One MRTG graph: "In" is a filled area, "Out" a line, optional thin peak
 * lines. Honours growright, logscale, fixed max and custom Colours[].
 */
export const TrafficChart = memo(function TrafficChart({
  points,
  step,
  period,
  format,
  colours,
  legendIn,
  legendOut,
  growright,
  logscale = false,
  fixedMax = null,
  peak = false,
  noI = false,
  noO = false,
  ytics = 4,
  showLegend = true,
  className,
}: TrafficChartProps) {
  const c = seriesColours(colours)

  const keys = useMemo(() => {
    const k: (keyof SeriesPoint)[] = []
    if (!noI) k.push('in')
    if (!noO) k.push('out')
    if (peak && !noI) k.push('in_max')
    if (peak && !noO) k.push('out_max')
    return k
  }, [noI, noO, peak])

  const config = useMemo<ChartConfig>(
    () => ({
      in: { label: <HtmlContent inline html={legendIn} />, color: c.in, indicator: 'area' },
      out: { label: <HtmlContent inline html={legendOut} />, color: c.out, indicator: 'line' },
      in_max: { label: <>Peak <HtmlContent inline html={legendIn} /></>, color: c.inPeak, indicator: 'dashed' },
      out_max: { label: <>Peak <HtmlContent inline html={legendOut} /></>, color: c.outPeak, indicator: 'dashed' },
    }),
    [c.in, c.out, c.inPeak, c.outPeak, legendIn, legendOut],
  )

  const data = useMemo(() => plotPoints(points, logscale), [points, logscale])
  const tMin = points.length ? points[0].t : 0
  const tMax = points.length ? points[points.length - 1].t : 1
  const xTicks = useMemo(() => timeTicks(tMin, tMax, period), [tMin, tMax, period])

  const y = useMemo(() => {
    if (logscale) return { domain: ['auto', 'auto'] as [string, string], ticks: undefined }
    const top = fixedMax && fixedMax > 0 ? fixedMax : niceMax(seriesMax(points, keys), format, ytics)
    return { domain: [0, top] as [number, number], ticks: linearTicks(top, ytics) }
  }, [logscale, fixedMax, points, keys, format, ytics])

  const curveType = 'linear' as const

  return (
    <ChartContainer config={config} className={cn('h-chart', className)}>
      <ComposedChart data={data} margin={{ top: 8, right: 8, bottom: 0, left: 0 }} accessibilityLayer>
        <CartesianGrid vertical={false} strokeDasharray="3 4" />
        <XAxis
          dataKey="t"
          type="number"
          scale="time"
          domain={[tMin, tMax]}
          ticks={xTicks}
          tickFormatter={(t: number) => formatAxisTime(t, period)}
          reversed={!growright}
          tickLine={false}
          axisLine={false}
          tickMargin={8}
          minTickGap={18}
        />
        <YAxis
          scale={logscale ? 'log' : 'linear'}
          domain={y.domain}
          ticks={y.ticks}
          allowDataOverflow={!!fixedMax}
          tickFormatter={(v: number) => formatTick(v, format)}
          tickLine={false}
          axisLine={false}
          width={84}
          tickMargin={4}
        />
        <ChartTooltip
          cursor={{ strokeWidth: 1 }}
          isAnimationActive={false}
          content={
            <ChartTooltipContent
              labelFormatter={(t) => formatPointTime(Number(t), step)}
              valueFormatter={(v) => formatRate(v, format)}
            />
          }
        />
        {!noI && (
          <Area
            dataKey="in"
            type={curveType}
            stroke="var(--color-in)"
            fill="var(--color-in)"
            fillOpacity={c.custom ? 0.85 : 0.28}
            strokeWidth={1.5}
            isAnimationActive={false}
            dot={false}
            activeDot={{ r: 3 }}
          />
        )}
        {peak && !noI && (
          <Line
            dataKey="in_max"
            type={curveType}
            stroke="var(--color-in_max)"
            strokeWidth={1}
            strokeDasharray="4 3"
            isAnimationActive={false}
            dot={false}
            activeDot={false}
          />
        )}
        {peak && !noO && (
          <Line
            dataKey="out_max"
            type={curveType}
            stroke="var(--color-out_max)"
            strokeWidth={1}
            strokeDasharray="4 3"
            isAnimationActive={false}
            dot={false}
            activeDot={false}
          />
        )}
        {!noO && (
          <Line
            dataKey="out"
            type={curveType}
            stroke="var(--color-out)"
            strokeWidth={2}
            isAnimationActive={false}
            dot={false}
            activeDot={{ r: 3 }}
          />
        )}
        {showLegend && <ChartLegend content={<ChartLegendContent keys={keys as string[]} />} />}
      </ComposedChart>
    </ChartContainer>
  )
})
