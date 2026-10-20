import { cn } from '@/lib/utils'
import { Card } from '@/components/ui/card'

type Tone = 'default' | 'ok' | 'warn' | 'error' | 'pending'

const toneIcon: Record<Tone, string> = {
  default: 'bg-accent text-accent-foreground',
  ok: 'bg-status-ok-soft text-status-ok',
  warn: 'bg-status-warn-soft text-status-warn',
  error: 'bg-status-error-soft text-status-error',
  pending: 'bg-status-pending-soft text-status-pending',
}

interface StatCardProps {
  label: string
  value: React.ReactNode
  hint?: React.ReactNode
  icon: React.ReactNode
  tone?: Tone
  className?: string
}

export function StatCard({ label, value, hint, icon, tone = 'default', className }: StatCardProps) {
  return (
    <Card className={cn('flex flex-col gap-3 p-card-pad', className)}>
      <div className="flex items-center justify-between gap-2">
        <span className="text-xs font-medium text-muted-foreground">{label}</span>
        <span
          aria-hidden
          className={cn('flex size-8 items-center justify-center rounded-pill [&_svg]:size-4', toneIcon[tone])}
        >
          {icon}
        </span>
      </div>
      <div className="text-stat tabular-nums">{value}</div>
      {hint && (
        <div className={cn('-mt-1 text-xs text-muted-foreground', typeof hint === 'string' && 'truncate')}>{hint}</div>
      )}
    </Card>
  )
}
