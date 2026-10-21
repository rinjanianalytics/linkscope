import { cn } from '@/lib/utils'
import { SectionLabel } from './SectionLabel'

interface PageHeaderProps {
  label: React.ReactNode
  labelIcon?: React.ReactNode
  /** Shown beside the section label, e.g. a status pill. */
  meta?: React.ReactNode
  title: React.ReactNode
  description?: React.ReactNode
  actions?: React.ReactNode
  className?: string
}

/** Section pill, large tight-tracked title and optional supporting line. */
export function PageHeader({ label, labelIcon, meta, title, description, actions, className }: PageHeaderProps) {
  return (
    <header className={cn('flex flex-col gap-4 md:flex-row md:items-end md:justify-between', className)}>
      <div className="flex min-w-0 flex-col gap-3">
        <div className="flex flex-wrap items-center gap-2">
          <SectionLabel icon={labelIcon}>{label}</SectionLabel>
          {meta}
        </div>
        <h1 className="text-display wrap-break-word">{title}</h1>
        {description && <div className="text-sm text-muted-foreground">{description}</div>}
      </div>
      {actions && <div className="flex shrink-0 flex-wrap items-center gap-2">{actions}</div>}
    </header>
  )
}
