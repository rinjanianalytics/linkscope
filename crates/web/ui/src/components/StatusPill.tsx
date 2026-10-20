import { memo } from 'react'
import { CircleAlert, CircleCheck, CircleX, Clock, Hourglass } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import type { Status } from '@/lib/api'

export const STATUS_META: Record<Status, { label: string; icon: typeof CircleCheck; description: string }> = {
  ok: { label: 'OK', icon: CircleCheck, description: 'Polled successfully' },
  warn: { label: 'Warning', icon: CircleAlert, description: 'Polled, but a threshold or check needs attention' },
  error: { label: 'Error', icon: CircleX, description: 'The last poll failed' },
  stale: { label: 'Stale', icon: Clock, description: 'No fresh data for several cycles' },
  pending: { label: 'Pending', icon: Hourglass, description: 'Not polled yet' },
}

export const StatusPill = memo(function StatusPill({ status, className }: { status: Status; className?: string }) {
  const meta = STATUS_META[status] ?? STATUS_META.pending
  const Icon = meta.icon
  return (
    <Badge variant={status} className={className} title={meta.description}>
      <Icon aria-hidden strokeWidth={2.5} />
      <span>{meta.label}</span>
    </Badge>
  )
})
