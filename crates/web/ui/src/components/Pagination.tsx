import { ChevronLeft, ChevronRight } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { formatCount } from '@/lib/format'

interface PaginationProps {
  page: number
  pageCount: number
  total: number
  pageSize: number
  onPage: (page: number) => void
}

export function Pagination({ page, pageCount, total, pageSize, onPage }: PaginationProps) {
  if (pageCount <= 1) return null
  const from = page * pageSize + 1
  const to = Math.min(total, (page + 1) * pageSize)
  return (
    <nav aria-label="Pagination" className="flex flex-wrap items-center justify-between gap-3">
      <p className="text-sm text-muted-foreground">
        Showing {formatCount(from)} to {formatCount(to)} of {formatCount(total)}
      </p>
      <div className="flex items-center gap-2">
        <Button variant="outline" size="sm" onClick={() => onPage(page - 1)} disabled={page === 0}>
          <ChevronLeft aria-hidden />
          Previous
        </Button>
        <span className="min-w-16 text-center text-sm tabular-nums text-muted-foreground">
          {page + 1} / {pageCount}
        </span>
        <Button variant="outline" size="sm" onClick={() => onPage(page + 1)} disabled={page >= pageCount - 1}>
          Next
          <ChevronRight aria-hidden />
        </Button>
      </div>
    </nav>
  )
}
