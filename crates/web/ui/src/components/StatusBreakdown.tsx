import { cn } from '@/lib/utils'
import type { Status } from '@/lib/api'

const DOT: Record<Status, string> = {
  ok: 'bg-status-ok',
  warn: 'bg-status-warn',
  error: 'bg-status-error',
  stale: 'bg-status-stale',
  pending: 'bg-status-pending',
}

export interface BreakdownItem {
  status: Status
  label: string
  count: number
}

/** Compact "● 3 failed  ● 1 warn  ● 0 stale" row; zero counts are dimmed. */
export function StatusBreakdown({ items, className }: { items: BreakdownItem[]; className?: string }) {
  return (
    <ul className={cn('flex flex-wrap gap-x-3 gap-y-1 text-xs', className)}>
      {items.map((it) => (
        <li key={it.label} className={cn('flex items-center gap-1.5', it.count === 0 && 'opacity-60')}>
          <span aria-hidden className={cn('size-1.5 rounded-pill', DOT[it.status])} />
          <span className="font-medium tabular-nums text-foreground">{it.count}</span>
          <span className="text-muted-foreground">{it.label}</span>
        </li>
      ))}
    </ul>
  )
}
