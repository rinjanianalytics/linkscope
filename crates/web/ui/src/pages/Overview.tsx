import { useCallback, useEffect, useMemo, useState } from 'react'
import {
  CircleCheck,
  CircleX,
  Folder,
  Hourglass,
  LayoutGrid,
  SearchX,
  Server,
  Timer,
  TimerReset,
  TriangleAlert,
  Gauge,
} from 'lucide-react'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card } from '@/components/ui/card'
import { Skeleton } from '@/components/ui/skeleton'
import { EmptyState } from '@/components/EmptyState'
import { PageHeader } from '@/components/PageHeader'
import { Pagination } from '@/components/Pagination'
import { SectionLabel } from '@/components/SectionLabel'
import { StatCard } from '@/components/StatCard'
import { StatusBreakdown } from '@/components/StatusBreakdown'
import { TargetCard, TargetCardSkeleton } from '@/components/TargetCard'
import { TargetToolbar, type SortKey, type StatusFilter } from '@/components/TargetToolbar'
import type { Polled } from '@/lib/hooks'
import { useDebounced } from '@/lib/hooks'
import { STATUSES, type Status, type Summary, type TargetSummary } from '@/lib/api'
import { formatAgo, formatCount, formatCountOf, formatDuration, formatInterval, formatMs } from '@/lib/format'
import { htmlToText } from '@/lib/text'

const PAGE_SIZE = 48
const STATUS_RANK: Record<Status, number> = { error: 0, warn: 1, stale: 2, pending: 3, ok: 4 }
const sinceTime = new Intl.DateTimeFormat(undefined, { timeStyle: 'short' })
const sinceDate = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium' })
const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' })

interface Indexed {
  t: TargetSummary
  title: string
  haystack: string
}

/** Filters live in the query string so the back button restores them. */
function readParams() {
  const p = new URLSearchParams(window.location.search)
  const status = (p.get('status') ?? 'all') as StatusFilter
  const sort = (p.get('sort') ?? 'name') as SortKey
  return {
    q: p.get('q') ?? '',
    status: status === 'all' || status === 'problems' || STATUSES.includes(status as Status) ? status : 'all',
    sort: ['name', 'title', 'status'].includes(sort) ? sort : 'name',
    group: p.get('group') !== '0',
    page: Math.max(0, Number(p.get('page') ?? 1) - 1) || 0,
  }
}

interface OverviewProps {
  summary: Polled<Summary>
  targets: Polled<TargetSummary[]>
}

export function Overview({ summary, targets }: OverviewProps) {
  const initial = useMemo(readParams, [])
  const [query, setQuery] = useState(initial.q)
  const [status, setStatus] = useState<StatusFilter>(initial.status)
  const [sort, setSort] = useState<SortKey>(initial.sort)
  const [group, setGroup] = useState(initial.group)
  const [page, setPage] = useState(initial.page)
  const q = useDebounced(query.trim().toLowerCase(), 150)

  useEffect(() => {
    const p = new URLSearchParams()
    if (query) p.set('q', query)
    if (status !== 'all') p.set('status', status)
    if (sort !== 'name') p.set('sort', sort)
    if (!group) p.set('group', '0')
    if (page > 0) p.set('page', String(page + 1))
    const s = p.toString()
    const url = `${window.location.pathname}${s ? `?${s}` : ''}`
    if (url !== `${window.location.pathname}${window.location.search}`) window.history.replaceState(null, '', url)
  }, [query, status, sort, group, page])

  const list = targets.data
  const indexed = useMemo<Indexed[]>(
    () =>
      (list ?? []).map((t) => {
        const title = htmlToText(t.title_html)
        return { t, title, haystack: `${t.name}\n${title}\n${t.sys_name ?? ''}`.toLowerCase() }
      }),
    [list],
  )

  const counts = useMemo(() => {
    const c = { all: indexed.length, problems: 0, ok: 0, warn: 0, error: 0, stale: 0, pending: 0 }
    for (const { t } of indexed) {
      c[t.status] = (c[t.status] ?? 0) + 1
      if (t.status === 'error' || t.status === 'warn' || t.status === 'stale') c.problems++
    }
    return c
  }, [indexed])

  const directories = useMemo(() => new Set(indexed.map((i) => i.t.directory)), [indexed])
  const canGroup = directories.size > 1
  const grouped = group && canGroup

  const filtered = useMemo(() => {
    const out = indexed.filter(({ t, haystack }) => {
      if (status === 'problems' ? !(t.status === 'error' || t.status === 'warn' || t.status === 'stale') : status !== 'all' && t.status !== status) return false
      return !q || haystack.includes(q)
    })
    const cmp = (a: Indexed, b: Indexed) => {
      if (grouped) {
        const d = collator.compare(a.t.directory, b.t.directory)
        if (d) return d
      }
      if (sort === 'status') {
        const s = STATUS_RANK[a.t.status] - STATUS_RANK[b.t.status]
        if (s) return s
      }
      if (sort === 'title') {
        const s = collator.compare(a.title, b.title)
        if (s) return s
      }
      return collator.compare(a.t.name, b.t.name)
    }
    return out.sort(cmp)
  }, [indexed, status, q, sort, grouped])

  const pageCount = Math.max(1, Math.ceil(filtered.length / PAGE_SIZE))
  const safePage = Math.min(page, pageCount - 1)
  const pageItems = filtered.slice(safePage * PAGE_SIZE, (safePage + 1) * PAGE_SIZE)

  const sections = useMemo(() => {
    if (!grouped) return [{ dir: null as string | null, items: pageItems }]
    const out: { dir: string | null; items: Indexed[] }[] = []
    for (const it of pageItems) {
      const last = out[out.length - 1]
      if (last && last.dir === it.t.directory) last.items.push(it)
      else out.push({ dir: it.t.directory, items: [it] })
    }
    return out
  }, [pageItems, grouped])

  const resetPage = useCallback(<T,>(set: (v: T) => void) => (v: T) => {
    set(v)
    setPage(0)
  }, [])

  const goPage = (p: number) => {
    setPage(p)
    document.getElementById('targets')?.scrollIntoView({ block: 'start' })
  }

  const s = summary.data

  return (
    <div className="flex flex-col gap-section">
      <PageHeader
        label="Overview"
        labelIcon={<LayoutGrid aria-hidden />}
        title="Network traffic"
        description={
          s ? (
            <span className="flex flex-wrap items-center gap-x-2 gap-y-1">
              <span>{formatCountOf(s.targets_total, 'target')}</span>
              <span aria-hidden>·</span>
              <span>{formatCountOf(s.cycles, 'poll cycle')}</span>
              <span aria-hidden>·</span>
              <span className="break-all font-mono text-xs">{s.config_path}</span>
            </span>
          ) : (
            <Skeleton className="h-4 w-64" />
          )
        }
      />

      {summary.error && !s && (
        <Alert variant="error">
          <CircleX />
          <AlertTitle>Cannot reach the LinkScope daemon</AlertTitle>
          <AlertDescription>
            {summary.error.message}{' '}
            <Button variant="link" className="h-auto p-0" onClick={summary.refresh}>
              Try again
            </Button>
          </AlertDescription>
        </Alert>
      )}

      <SummaryStats summary={s} />

      {s && s.warnings.length > 0 && (
        <Alert variant="warn">
          <TriangleAlert />
          <AlertTitle>
            {s.warnings.length === 1 ? 'Configuration warning' : `${s.warnings.length} configuration warnings`}
          </AlertTitle>
          <AlertDescription>
            <ul className="mt-2 grid gap-1">
              {s.warnings.map((w, i) => (
                <li key={i} className="break-words">
                  {w}
                </li>
              ))}
            </ul>
          </AlertDescription>
        </Alert>
      )}

      <section id="targets" aria-labelledby="targets-heading" className="flex scroll-mt-4 flex-col gap-5">
        <div className="flex flex-col gap-3">
          <SectionLabel icon={<Server aria-hidden />}>Targets</SectionLabel>
          <div className="flex flex-wrap items-baseline justify-between gap-2">
            <h2 id="targets-heading" className="text-title">
              {list ? `${formatCount(filtered.length)} of ${formatCountOf(indexed.length, 'target')}` : 'Targets'}
            </h2>
          </div>
        </div>

        <TargetToolbar
          query={query}
          onQuery={resetPage(setQuery)}
          status={status}
          onStatus={resetPage(setStatus)}
          sort={sort}
          onSort={resetPage(setSort)}
          group={group}
          onGroup={resetPage(setGroup)}
          canGroup={canGroup}
          counts={counts}
        />

        {targets.error && !list && (
          <Alert variant="error">
            <CircleX />
            <AlertTitle>Could not load targets</AlertTitle>
            <AlertDescription>{targets.error.message}</AlertDescription>
          </Alert>
        )}

        {!list && !targets.error ? (
          <CardGrid>
            {Array.from({ length: 8 }, (_, i) => (
              <TargetCardSkeleton key={i} />
            ))}
          </CardGrid>
        ) : list && filtered.length === 0 ? (
          <Card>
            <EmptyState icon={<SearchX />} title={indexed.length ? 'No targets match' : 'No targets configured'}>
              {indexed.length ? (
                <Button
                  variant="link"
                  className="h-auto p-0"
                  onClick={() => {
                    setQuery('')
                    setStatus('all')
                    setPage(0)
                  }}
                >
                  Clear search and filters
                </Button>
              ) : (
                'Add Target[] lines to the configuration file and restart the daemon.'
              )}
            </EmptyState>
          </Card>
        ) : (
          sections.map((sec) => (
            <div key={sec.dir ?? '_all'} className="flex flex-col gap-4">
              {sec.dir !== null && (
                <h3 className="flex items-center gap-2 text-sm font-semibold text-muted-foreground">
                  <Folder aria-hidden className="size-4" />
                  {sec.dir || 'Top level'}
                </h3>
              )}
              <CardGrid>
                {sec.items.map(({ t }) => (
                  <TargetCard key={t.name} target={t} />
                ))}
              </CardGrid>
            </div>
          ))
        )}

        <Pagination
          page={safePage}
          pageCount={pageCount}
          total={filtered.length}
          pageSize={PAGE_SIZE}
          onPage={goPage}
        />
      </section>
    </div>
  )
}

function CardGrid({ children }: { children: React.ReactNode }) {
  return <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3 2xl:grid-cols-4">{children}</div>
}

function SummaryStats({ summary: s }: { summary: Summary | undefined }) {
  if (!s) {
    return (
      <div className="grid grid-cols-2 gap-4 md:grid-cols-3 xl:grid-cols-6">
        {Array.from({ length: 6 }, (_, i) => (
          <Skeleton key={i} className="h-32 rounded-card" />
        ))}
      </div>
    )
  }
  const lc = s.last_cycle
  return (
    <div className="grid grid-cols-2 gap-4 md:grid-cols-3 xl:grid-cols-6">
      <StatCard
        label="Targets OK"
        tone="ok"
        icon={<CircleCheck />}
        value={formatCount(s.targets_ok)}
        hint={`of ${formatCount(s.targets_total)} configured`}
      />
      <StatCard
        label="With problems"
        tone={(s.targets_failed ?? s.targets_error) > 0 ? 'error' : s.targets_error > 0 ? 'warn' : 'default'}
        icon={<TriangleAlert />}
        value={formatCount(s.targets_error)}
        hint={
          <StatusBreakdown
            items={[
              { status: 'error', label: 'failed', count: s.targets_failed ?? s.targets_error },
              { status: 'warn', label: 'warn', count: s.targets_warn ?? 0 },
              { status: 'stale', label: 'stale', count: s.targets_stale ?? 0 },
            ]}
          />
        }
      />
      <StatCard
        label="Pending"
        tone="pending"
        icon={<Hourglass />}
        value={formatCount(s.targets_pending)}
        hint="not polled yet"
      />
      <StatCard
        label="Last cycle"
        icon={<Timer />}
        value={lc ? formatMs(lc.duration_ms) : 'n/a'}
        hint={lc ? `${lc.ok} ok · ${lc.failed} failed · ${formatAgo(lc.started, s.now)}` : 'waiting for the first cycle'}
      />
      <StatCard label="Interval" icon={<TimerReset />} value={formatInterval(s.interval_secs)} hint="between poll cycles" />
      <StatCard
        label="Uptime"
        icon={<Gauge />}
        value={formatDuration(s.now - s.started)}
        hint={`since ${(s.now - s.started < 86400 ? sinceTime : sinceDate).format(s.started * 1000)}`}
      />
    </div>
  )
}
