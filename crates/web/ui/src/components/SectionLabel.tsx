import { cn } from '@/lib/utils'
import { Badge } from '@/components/ui/badge'

interface SectionLabelProps extends React.HTMLAttributes<HTMLSpanElement> {
  icon?: React.ReactNode
}

/** Pill-shaped uppercase label in the pale accent tint, used above titles and sections. */
export function SectionLabel({ icon, className, children, ...props }: SectionLabelProps) {
  return (
    <Badge variant="accent" className={cn('w-fit', className)} {...props}>
      {icon}
      {children}
    </Badge>
  )
}
