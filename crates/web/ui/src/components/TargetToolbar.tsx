import { ArrowDownAZ, FolderTree, ListFilter, Search, X } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { STATUSES, type Status } from '@/lib/api'
import { STATUS_META } from './StatusPill'
import { cn } from '@/lib/utils'

export type SortKey = 'name' | 'title' | 'status'
export type StatusFilter = 'all' | 'problems' | Status

interface TargetToolbarProps {
  query: string
  onQuery: (q: string) => void
  status: StatusFilter
  onStatus: (s: StatusFilter) => void
  sort: SortKey
  onSort: (s: SortKey) => void
  group: boolean
  onGroup: (g: boolean) => void
  canGroup: boolean
  counts: Record<Status, number> & { all: number; problems: number }
}

export function TargetToolbar(p: TargetToolbarProps) {
  return (
    <div className="flex flex-col gap-3 lg:flex-row lg:items-center">
      <div className="relative min-w-0 flex-1">
        <Search aria-hidden className="pointer-events-none absolute left-4 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
        <Input
          type="search"
          value={p.query}
          onChange={(e) => p.onQuery(e.target.value)}
          placeholder="Search by name or title"
          aria-label="Search targets by name or title"
          className="pl-10 pr-10 [&::-webkit-search-cancel-button]:hidden"
        />
        {p.query && (
          <Button
            variant="ghost"
            size="icon"
            className="absolute right-1 top-1/2 size-8 -translate-y-1/2"
            onClick={() => p.onQuery('')}
            aria-label="Clear search"
          >
            <X aria-hidden />
          </Button>
        )}
      </div>
      <div className="grid grid-cols-2 gap-3 sm:flex sm:flex-wrap sm:items-center">
        <Select value={p.status} onValueChange={(v) => p.onStatus(v as StatusFilter)}>
          <SelectTrigger className="sm:w-44" aria-label="Filter by status">
            <ListFilter aria-hidden />
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">All ({p.counts.all})</SelectItem>
            <SelectItem value="problems">Problems ({p.counts.problems})</SelectItem>
            {STATUSES.map((s) => (
              <SelectItem key={s} value={s} disabled={p.counts[s] === 0}>
                {STATUS_META[s].label} ({p.counts[s]})
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Select value={p.sort} onValueChange={(v) => p.onSort(v as SortKey)}>
          <SelectTrigger className="sm:w-36" aria-label="Sort targets">
            <ArrowDownAZ aria-hidden />
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="name">Name</SelectItem>
            <SelectItem value="title">Title</SelectItem>
            <SelectItem value="status">Status</SelectItem>
          </SelectContent>
        </Select>
        <Button
          variant="outline"
          aria-pressed={p.group}
          disabled={!p.canGroup}
          title={p.canGroup ? 'Group targets by directory' : 'All targets share one directory'}
          onClick={() => p.onGroup(!p.group)}
          className={cn('col-span-2 sm:col-span-1', p.group && p.canGroup && 'border-primary/40 bg-accent text-accent-foreground hover:bg-accent')}
        >
          <FolderTree aria-hidden />
          Group by directory
        </Button>
      </div>
    </div>
  )
}
