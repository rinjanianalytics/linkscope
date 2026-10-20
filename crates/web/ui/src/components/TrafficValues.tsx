import { ArrowDownLeft, ArrowUpRight } from 'lucide-react'
import { Swatch } from '@/components/ui/chart'
import { cn } from '@/lib/utils'

interface TrafficValuesProps {
  /** MRTG-formatted current values, e.g. "12.3 MB/s (4.5%)". */
  inValue?: string | null
  outValue?: string | null
  noI?: boolean
  noO?: boolean
  /** The last poll failed or went stale: the stored 0 means "unknown". */
  unknown?: boolean
  inColour: string
  outColour: string
  className?: string
}

/** Split "12.3 MB/s (4.5%)" into the rate and its percentage for a quieter second line. */
function splitPct(text: string): [string, string | null] {
  const m = /^(.*?)\s*(\([^()]*%\))$/.exec(text)
  return m ? [m[1], m[2]] : [text, null]
}

/** Current In / Out pair with series swatches. */
export function TrafficValues({ inValue, outValue, noI, noO, unknown, inColour, outColour, className }: TrafficValuesProps) {
  const row = (label: string, icon: React.ReactNode, text: string | null | undefined, colour: string, kind: 'area' | 'line') => {
    const [rate, pct] = text && !unknown ? splitPct(text) : [null, null]
    return (
      <div className="flex min-w-0 flex-col gap-1">
        <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
          <Swatch indicator={kind} colorVar={colour} />
          {label}
          {icon}
        </span>
        {rate ? (
          <span className="flex min-w-0 flex-wrap items-baseline gap-x-1.5">
            <span className="truncate text-sm font-semibold tabular-nums">{rate}</span>
            {pct && <span className="text-xs tabular-nums text-muted-foreground">{pct}</span>}
          </span>
        ) : (
          <span className="text-sm font-medium text-muted-foreground">{unknown ? 'Unknown' : 'n/a'}</span>
        )}
      </div>
    )
  }
  return (
    <div className={cn('grid grid-cols-2 gap-3', className)}>
      {!noI && row('In', <ArrowDownLeft aria-hidden className="size-3" />, inValue, inColour, 'area')}
      {!noO && row('Out', <ArrowUpRight aria-hidden className="size-3" />, outValue, outColour, 'line')}
    </div>
  )
}
