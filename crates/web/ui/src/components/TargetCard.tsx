import { memo } from 'react'
import { Link } from 'wouter'
import { ChevronRight, Server, TriangleAlert, ChartNoAxesColumn } from 'lucide-react'
import { Card } from '@/components/ui/card'
import { Skeleton } from '@/components/ui/skeleton'
import { HtmlContent } from './HtmlContent'
import { StatusPill } from './StatusPill'
import { Sparkline } from './Sparkline'
import { TrafficValues } from './TrafficValues'
import { targetHref, type TargetSummary } from '@/lib/api'
import { seriesColours } from '@/lib/chart'
import { formatRate } from '@/lib/format'
import { useInView } from '@/lib/hooks'
import { useQueuedSeries } from '@/lib/series-store'

const SPARK_POINTS = 96

function TargetCardImpl({ target }: { target: TargetSummary }) {
  const [ref, inView] = useInView<HTMLDivElement>('240px')
  const series = useQueuedSeries(target.name, 'day', SPARK_POINTS, inView)
  const c = seriesColours(target.colours)
  const data = series.data
  const unknown = target.status === 'error' || target.status === 'stale' || target.status === 'pending'

  return (
    <Card
      ref={ref}
      className="group relative flex flex-col gap-4 p-card-pad transition-shadow focus-within:shadow-card-hover hover:shadow-card-hover"
    >
      <div className="flex items-start justify-between gap-3">
        <StatusPill status={target.status} />
        <ChevronRight
          aria-hidden
          className="size-4 text-muted-foreground transition-transform group-hover:translate-x-0.5"
        />
      </div>

      <div className="flex min-w-0 flex-col gap-1">
        <h3 className="line-clamp-2 text-base font-semibold leading-snug tracking-tight">
          <Link
            href={targetHref(target.name)}
            className="rounded-sm outline-none after:absolute after:inset-0 after:rounded-card focus-visible:after:ring-2 focus-visible:after:ring-ring"
          >
            <HtmlContent inline html={target.title_html} />
          </Link>
        </h3>
        <p className="flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground">
          <Server aria-hidden className="size-3.5 shrink-0" />
          <span className="truncate">{target.sys_name || 'Unknown system'}</span>
          <span aria-hidden>·</span>
          <span className="truncate font-mono">{target.name}</span>
        </p>
      </div>

      {target.error && (
        <p className="flex gap-1.5 text-xs text-status-error">
          <TriangleAlert aria-hidden className="mt-px size-3.5 shrink-0" />
          <span className="line-clamp-2">{target.error}</span>
        </p>
      )}

      <div className="mt-auto flex flex-col gap-3">
        <TrafficValues
          unknown={unknown}
          inValue={target.cur_in_text ?? (target.cur_in != null ? formatRate(target.cur_in, target.format) : null)}
          outValue={target.cur_out_text ?? (target.cur_out != null ? formatRate(target.cur_out, target.format) : null)}
          noI={target.no_i}
          noO={target.no_o}
          inColour={c.in}
          outColour={c.out}
        />
        {data ? (
          <Sparkline
            points={data.points}
            colours={target.colours}
            noI={target.no_i}
            noO={target.no_o}
            growright={target.growright}
            logscale={target.logscale}
            label={`Traffic over the last day for ${target.name}`}
          />
        ) : series.status === 'empty' || series.status === 'error' ? (
          <div className="flex h-spark items-center justify-center gap-1.5 rounded-md bg-muted text-xs text-muted-foreground">
            <ChartNoAxesColumn aria-hidden className="size-3.5" />
            {series.status === 'empty' ? 'No data collected yet' : 'Graph unavailable'}
          </div>
        ) : (
          <Skeleton className="h-spark w-full" />
        )}
      </div>
    </Card>
  )
}

/** Only re-render when the fields the card shows change (the list is refetched every 30 s). */
export const TargetCard = memo(TargetCardImpl, (a, b) => {
  const x = a.target
  const y = b.target
  return (
    x === y ||
    (x.name === y.name &&
      x.title_html === y.title_html &&
      x.status === y.status &&
      x.error === y.error &&
      x.sys_name === y.sys_name &&
      x.no_i === y.no_i &&
      x.no_o === y.no_o &&
      x.cur_in_text === y.cur_in_text &&
      x.cur_out_text === y.cur_out_text &&
      x.cur_in === y.cur_in &&
      x.cur_out === y.cur_out &&
      x.growright === y.growright &&
      x.logscale === y.logscale &&
      JSON.stringify(x.format) === JSON.stringify(y.format) &&
      JSON.stringify(x.colours) === JSON.stringify(y.colours))
  )
})

export function TargetCardSkeleton() {
  return (
    <Card className="flex flex-col gap-4 p-card-pad">
      <Skeleton className="h-6 w-16 rounded-pill" />
      <div className="flex flex-col gap-2">
        <Skeleton className="h-5 w-3/4" />
        <Skeleton className="h-3 w-1/2" />
      </div>
      <div className="grid grid-cols-2 gap-3">
        <Skeleton className="h-9" />
        <Skeleton className="h-9" />
      </div>
      <Skeleton className="h-spark w-full" />
    </Card>
  )
}
