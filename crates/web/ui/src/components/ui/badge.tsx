import * as React from 'react'
import { cva, type VariantProps } from 'class-variance-authority'
import { cn } from '@/lib/utils'

const badgeVariants = cva(
  'inline-flex items-center gap-1.5 rounded-pill px-2.5 py-1 text-label uppercase [&_svg]:size-3 [&_svg]:shrink-0',
  {
    variants: {
      variant: {
        accent: 'bg-accent text-accent-foreground',
        outline: 'border border-border text-muted-foreground',
        secondary: 'bg-secondary text-secondary-foreground',
        ok: 'bg-status-ok-soft text-status-ok',
        warn: 'bg-status-warn-soft text-status-warn',
        error: 'bg-status-error-soft text-status-error',
        stale: 'bg-status-stale-soft text-status-stale',
        pending: 'bg-status-pending-soft text-status-pending',
      },
    },
    defaultVariants: { variant: 'accent' },
  },
)

export interface BadgeProps extends React.HTMLAttributes<HTMLSpanElement>, VariantProps<typeof badgeVariants> {}

function Badge({ className, variant, ...props }: BadgeProps) {
  return <span className={cn(badgeVariants({ variant }), className)} {...props} />
}

export { Badge, badgeVariants }
