/**
 * shadcn/ui-style chart wrappers around Recharts.
 *
 * Series colours are passed as CSS custom properties (`--color-<key>`) on the
 * container, so Recharts elements reference `var(--color-<key>)` and never
 * carry literal colour values. Unlike upstream shadcn this does not inject a
 * <style> element: the per-chart variables go on the container's style.
 */
import * as React from 'react'
import * as RechartsPrimitive from 'recharts'
import { cn } from '@/lib/utils'

export type ChartConfig = Record<
  string,
  {
    label?: React.ReactNode
    /** Any CSS colour expression, usually `hsl(var(--chart-in))`. */
    color?: string
    /** How the series is drawn, used for the legend/tooltip swatch. */
    indicator?: 'area' | 'line' | 'dashed'
  }
>

const ChartContext = React.createContext<{ config: ChartConfig } | null>(null)

export function useChart() {
  const ctx = React.useContext(ChartContext)
  if (!ctx) throw new Error('useChart must be used within a <ChartContainer />')
  return ctx
}

const ChartContainer = React.forwardRef<
  HTMLDivElement,
  React.ComponentProps<'div'> & {
    config: ChartConfig
    children: React.ComponentProps<typeof RechartsPrimitive.ResponsiveContainer>['children']
  }
>(({ id, className, children, config, style, ...props }, ref) => {
  const uid = React.useId()
  const chartId = `chart-${id || uid.replace(/:/g, '')}`
  const vars = React.useMemo(() => {
    const v: Record<string, string> = {}
    for (const [key, c] of Object.entries(config)) if (c.color) v[`--color-${key}`] = c.color
    return v
  }, [config])

  return (
    <ChartContext.Provider value={{ config }}>
      <div
        data-chart={chartId}
        ref={ref}
        // Recharts needs concrete colours; these CSS variables are the only inline styles.
        style={{ ...vars, ...style } as React.CSSProperties}
        className={cn(
          'flex w-full justify-center text-xs',
          '[&_.recharts-cartesian-axis-tick_text]:fill-muted-foreground [&_.recharts-cartesian-grid_line]:stroke-chart-grid',
          '[&_.recharts-cartesian-axis-line]:stroke-border [&_.recharts-cartesian-axis-tick-line]:stroke-border',
          '[&_.recharts-curve.recharts-tooltip-cursor]:stroke-muted-foreground/40',
          '[&_.recharts-layer]:outline-hidden [&_.recharts-surface]:outline-hidden [&_.recharts-sector]:outline-hidden',
          className,
        )}
        {...props}
      >
        <RechartsPrimitive.ResponsiveContainer debounce={50}>{children}</RechartsPrimitive.ResponsiveContainer>
      </div>
    </ChartContext.Provider>
  )
})
ChartContainer.displayName = 'ChartContainer'

const ChartTooltip = RechartsPrimitive.Tooltip

function Swatch({ indicator, colorVar }: { indicator?: 'area' | 'line' | 'dashed'; colorVar: string }) {
  return (
    <span
      aria-hidden
      className={cn(
        'shrink-0',
        indicator === 'area' && 'size-2.5 rounded-sm bg-(--swatch)',
        indicator === 'line' && 'h-0.5 w-3 rounded-pill bg-(--swatch)',
        indicator === 'dashed' && 'h-0 w-3 border-t-2 border-dashed border-(--swatch)',
        !indicator && 'size-2.5 rounded-pill bg-(--swatch)',
      )}
      style={{ '--swatch': colorVar } as React.CSSProperties}
    />
  )
}

type TooltipPayloadItem = {
  dataKey?: string | number
  name?: string | number
  value?: unknown
  color?: string
  payload?: Record<string, unknown>
}

interface ChartTooltipContentProps {
  active?: boolean
  payload?: TooltipPayloadItem[]
  label?: unknown
  labelFormatter?: (label: unknown, payload: TooltipPayloadItem[]) => React.ReactNode
  valueFormatter?: (value: number, key: string) => React.ReactNode
  hideLabel?: boolean
  className?: string
}

function ChartTooltipContent({
  active,
  payload,
  label,
  labelFormatter,
  valueFormatter,
  hideLabel,
  className,
}: ChartTooltipContentProps) {
  const { config } = useChart()
  if (!active || !payload?.length) return null
  return (
    <div
      className={cn(
        'grid min-w-40 gap-1.5 rounded-md border border-border bg-popover px-3 py-2 text-xs text-popover-foreground shadow-pop',
        className,
      )}
    >
      {!hideLabel && (
        <div className="font-medium">{labelFormatter ? labelFormatter(label, payload) : String(label ?? '')}</div>
      )}
      <div className="grid gap-1">
        {payload.map((item) => {
          const key = String(item.dataKey ?? item.name ?? '')
          const c = config[key]
          if (!c) return null
          const v = typeof item.value === 'number' ? item.value : null
          return (
            <div key={key} className="flex items-center gap-2">
              <Swatch indicator={c.indicator} colorVar={`var(--color-${key})`} />
              <span className="text-muted-foreground">{c.label ?? key}</span>
              <span className="ml-auto pl-3 font-medium tabular-nums text-foreground">
                {v == null ? 'n/a' : valueFormatter ? valueFormatter(v, key) : v}
              </span>
            </div>
          )
        })}
      </div>
    </div>
  )
}

const ChartLegend = RechartsPrimitive.Legend

interface ChartLegendContentProps {
  payload?: { dataKey?: unknown; value?: unknown }[]
  className?: string
  /** Render all configured keys instead of the Recharts payload. */
  keys?: string[]
}

function ChartLegendContent({ payload, className, keys }: ChartLegendContentProps) {
  const { config } = useChart()
  const list = keys ?? (payload ?? []).map((p) => String(p.dataKey ?? p.value ?? ''))
  if (!list.length) return null
  return (
    <ul className={cn('flex flex-wrap items-center justify-center gap-x-4 gap-y-1.5 pt-3', className)}>
      {list.map((key) => {
        const c = config[key]
        if (!c) return null
        return (
          <li key={key} className="flex items-center gap-1.5 text-muted-foreground">
            <Swatch indicator={c.indicator} colorVar={`var(--color-${key})`} />
            <span>{c.label ?? key}</span>
          </li>
        )
      })}
    </ul>
  )
}

export { ChartContainer, ChartTooltip, ChartTooltipContent, ChartLegend, ChartLegendContent, Swatch }
