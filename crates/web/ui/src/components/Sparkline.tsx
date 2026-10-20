import { memo, useMemo } from 'react'
import { Area, ComposedChart, Line, XAxis, YAxis } from 'recharts'
import { ChartContainer, type ChartConfig } from '@/components/ui/chart'
import type { Colour, SeriesPoint } from '@/lib/api'
import { plotPoints, seriesColours, seriesMax } from '@/lib/chart'
import { cn } from '@/lib/utils'

interface SparklineProps {
  points: SeriesPoint[]
  colours: Colour[]
  noI?: boolean
  noO?: boolean
  /** false: newest on the left, as MRTG draws by default. */
  growright?: boolean
  logscale?: boolean
  label: string
  className?: string
}

/** Small, non-interactive day graph for target cards. */
export const Sparkline = memo(function Sparkline({ points, colours, noI, noO, growright = true, logscale = false, label, className }: SparklineProps) {
  const c = seriesColours(colours)
  const config = useMemo<ChartConfig>(() => ({ in: { color: c.in }, out: { color: c.out } }), [c.in, c.out])
  const data = useMemo(() => plotPoints(points, logscale), [points, logscale])
  const top = useMemo(() => {
    const m = seriesMax(points, ['in', 'out'])
    return m > 0 ? m * 1.08 : 1
  }, [points])
  const tMin = points.length ? points[0].t : 0
  const tMax = points.length ? points[points.length - 1].t : 1

  return (
    <div role="img" aria-label={label} className={cn('h-spark w-full', className)}>
      <ChartContainer config={config} className="h-full" aria-hidden>
        <ComposedChart data={data} margin={{ top: 2, right: 0, bottom: 0, left: 0 }}>
          <XAxis hide dataKey="t" type="number" domain={[tMin, tMax]} reversed={!growright} />
          {logscale ? (
            <YAxis hide scale="log" domain={['auto', 'auto']} allowDataOverflow />
          ) : (
            <YAxis hide domain={[0, top]} />
          )}
          {!noI && (
            <Area
              dataKey="in"
              type="linear"
              stroke="var(--color-in)"
              fill="var(--color-in)"
              fillOpacity={c.custom ? 0.8 : 0.22}
              strokeWidth={1.25}
              isAnimationActive={false}
              dot={false}
              activeDot={false}
            />
          )}
          {!noO && (
            <Line
              dataKey="out"
              type="linear"
              stroke="var(--color-out)"
              strokeWidth={1.5}
              isAnimationActive={false}
              dot={false}
              activeDot={false}
            />
          )}
        </ComposedChart>
      </ChartContainer>
    </div>
  )
})
