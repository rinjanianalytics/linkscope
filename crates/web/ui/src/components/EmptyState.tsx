import { cn } from '@/lib/utils'

interface EmptyStateProps {
  icon: React.ReactNode
  title: string
  children?: React.ReactNode
  className?: string
}

export function EmptyState({ icon, title, children, className }: EmptyStateProps) {
  return (
    <div className={cn('flex flex-col items-center justify-center gap-2 px-6 py-10 text-center', className)}>
      <span
        aria-hidden
        className="mb-1 flex size-11 items-center justify-center rounded-pill bg-accent text-accent-foreground [&_svg]:size-5"
      >
        {icon}
      </span>
      <p className="font-medium">{title}</p>
      {children && <div className="max-w-sm text-sm text-muted-foreground">{children}</div>}
    </div>
  )
}
