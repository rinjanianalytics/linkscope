import { CalendarDays, CalendarRange, Clock, ChartNoAxesColumn, CalendarClock } from 'lucide-react'
import { Card, CardContent, CardHeader } from '@/components/ui/card'
import { Skeleton } from '@/components/ui/skeleton'
import { SectionLabel } from './SectionLabel'
import { EmptyState } from './EmptyState'
import { TrafficChart } from './TrafficChart'
import { StatsTable } from './StatsTable'
import { HtmlContent, hasHtml } from './HtmlContent'
import { api, ApiError, type Period, type TargetDetail } from '@/lib/api'
import { seriesColours } from '@/lib/chart'
import { formatStep } from '@/lib/format'
import { usePolling } from '@/lib/hooks'
import { Alert, AlertDescription } from '@/components/ui/alert'

const PERIOD_META: Record<Period, { label: string; title: string; icon: React.ReactNode }> = {
  day: { label: 'Daily', title: 'Last day', icon: <Clock aria-hidden /> },
  week: { label: 'Weekly', title: 'Last week', icon: <CalendarDays aria-hidden /> },
  month: { label: 'Monthly', title: 'Last month', icon: <CalendarRange aria-hidden /> },
  year: { label: 'Yearly', title: 'Last year', icon: <CalendarClock aria-hidden /> },
}

const SERIES_REFRESH_MS = 60_000

export function PeriodGraph({ target, period }: { target: TargetDetail; period: Period }) {
  const meta = PERIOD_META[period]
  const series = usePolling(
    (signal) => api.series(target.name, period, target.xsize || undefined, signal),
    SERIES_REFRESH_MS,
    [target.name, period, target.xsize],
  )
  const c = seriesColours(target.colours)
  const notFound = series.error instanceof ApiError && series.error.status === 404
  const s = series.data
  const legends = target.legends.filter(hasHtml)

  return (
    <Card>
      <CardHeader className="gap-3 pb-4">
        <SectionLabel icon={meta.icon}>{meta.label}</SectionLabel>
        <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
          <h2 className="text-title">{meta.title}</h2>
          {s && <p className="text-xs text-muted-foreground">{formatStep(s.step)} average</p>}
        </div>
        {target.y_legend && <p className="text-xs text-muted-foreground">{target.y_legend}</p>}
      </CardHeader>
      <CardContent className="flex flex-col gap-5">
        {s ? (
          <>
            <TrafficChart
              points={s.points}
              step={s.step}
              period={period}
              format={target.format}
              colours={target.colours}
              legendIn={target.legend_i}
              legendOut={target.legend_o}
              growright={target.growright}
              logscale={target.logscale}
              fixedMax={s.fixed_max}
              peak={s.peak}
              noI={target.no_i}
              noO={target.no_o}
              ytics={target.ytics || 4}
              showLegend={!target.nolegend}
            />
            <StatsTable
              stats={s.stats}
              legendIn={target.legend_i}
              legendOut={target.legend_o}
              inColour={c.in}
              outColour={c.out}
              noI={target.no_i}
              noO={target.no_o}
              avgPeak={target.avgpeak}
            />
            {!target.nolegend && legends.length > 0 && (
              <ul className="grid gap-1 border-t border-border pt-4 text-xs text-muted-foreground">
                {legends.map((l, i) => (
                  <li key={i}>
                    <HtmlContent inline html={l} />
                  </li>
                ))}
              </ul>
            )}
          </>
        ) : notFound ? (
          <EmptyState icon={<ChartNoAxesColumn />} title="No data collected yet">
            The first samples appear after the daemon has polled this target. This page refreshes on its own.
          </EmptyState>
        ) : series.error ? (
          <Alert variant="error">
            <AlertDescription>Could not load this graph: {series.error.message}</AlertDescription>
          </Alert>
        ) : (
          <div className="flex flex-col gap-5">
            <Skeleton className="h-chart w-full rounded-lg" />
            <Skeleton className="h-20 w-full rounded-lg" />
          </div>
        )}
      </CardContent>
    </Card>
  )
}
