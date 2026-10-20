import { Bell, BellRing } from 'lucide-react'
import type { RateFormat, ThresholdAlert } from '@/lib/api'
import { formatAgo, formatCountOf, formatDateTime, formatRate } from '@/lib/format'
import { useNow } from '@/lib/hooks'
import { cn } from '@/lib/utils'

/** "In above 30%" or "Out below 12.0 kb/s". */
export function boundText(a: ThresholdAlert, f: RateFormat): string {
  const dir = a.direction === 'in' ? 'In' : 'Out'
  const side = a.bound === 'max' ? 'above' : 'below'
  const limit = a.percent != null ? a.threshold : formatRate(a.threshold_rate, f)
  return `${dir} ${side} ${limit}`
}

/** "41.7%" or "52.1 kb/s". */
export function nowText(a: ThresholdAlert, f: RateFormat): string {
  return a.percent != null ? `${a.percent.toFixed(1)}%` : formatRate(a.rate, f)
}

/**
 * The broken threshold bounds of one target. A firing alert reads "In above 30%, now 41.7%";
 * one still waiting out ThreshFor[] says how many polls it has been broken.
 */
export function AlertList({
  alerts,
  format,
  detailed = false,
  className,
}: {
  alerts: ThresholdAlert[]
  format: RateFormat
  detailed?: boolean
  className?: string
}) {
  const now = useNow()
  if (alerts.length === 0) return null
  return (
    <ul className={cn('flex flex-col gap-1 text-xs', className)}>
      {alerts.map((a) => {
        const Icon = a.firing ? BellRing : Bell
        const at = a.firing ? (a.fired_at ?? a.since) : a.since
        return (
          <li
            key={`${a.bound}_${a.direction}`}
            className={cn('flex gap-1.5', a.firing ? 'text-status-warn' : 'text-muted-foreground')}
          >
            <Icon aria-hidden className="mt-px size-3.5 shrink-0" />
            <span>
              <span className="font-medium">{boundText(a, format)}</span>
              {', now '}
              {nowText(a, format)}
              {a.firing ? (
                detailed && (
                  <>
                    {', firing since '}
                    <time dateTime={new Date(at * 1000).toISOString()} title={formatDateTime(at)}>
                      {formatAgo(at, now)}
                    </time>
                  </>
                )
              ) : (
                <>
                  {', '}
                  {a.polls} of {formatCountOf(a.polls_needed, 'poll')} before it fires
                </>
              )}
            </span>
          </li>
        )
      })}
    </ul>
  )
}
