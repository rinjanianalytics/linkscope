import { ArrowLeft, BellRing, CircleX, Clock, Cpu, FileQuestion, Gauge, Network, Server } from 'lucide-react'
import { Link } from 'wouter'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card, CardContent } from '@/components/ui/card'
import { Skeleton } from '@/components/ui/skeleton'
import { AlertList } from '@/components/AlertList'
import { EmptyState } from '@/components/EmptyState'
import { HtmlContent, hasHtml } from '@/components/HtmlContent'
import { PageHeader } from '@/components/PageHeader'
import { PeriodGraph } from '@/components/PeriodGraph'
import { StatusPill } from '@/components/StatusPill'
import { api, ApiError, PERIODS, type TargetDetail } from '@/lib/api'
import { formatAgo, formatDateTime, formatRate } from '@/lib/format'
import { usePolling, useNow } from '@/lib/hooks'
import { useEffect } from 'react'
import { htmlToText } from '@/lib/text'

const TARGET_REFRESH_MS = 30_000

export function TargetPage({ name }: { name: string }) {
  const target = usePolling((signal) => api.target(name, signal), TARGET_REFRESH_MS, [name])
  const t = target.data

  useEffect(() => {
    document.title = t ? `${htmlToText(t.title_html) || t.name} · LinkScope` : 'LinkScope'
    return () => {
      document.title = 'LinkScope'
    }
  }, [t])

  // A block, not an expression: newer Chrome returns a Promise from scrollTo, and React would
  // call whatever an effect returns as its cleanup.
  useEffect(() => {
    window.scrollTo(0, 0)
  }, [name])

  const back = (
    <Button variant="ghost" size="sm" asChild className="-ml-3 w-fit">
      <Link href="/">
        <ArrowLeft aria-hidden />
        All targets
      </Link>
    </Button>
  )

  if (!t) {
    if (target.error instanceof ApiError && target.error.status === 404) {
      return (
        <div className="flex flex-col gap-section">
          {back}
          <Card>
            <EmptyState icon={<FileQuestion />} title="Target not found">
              There is no target named <span className="font-mono">{name}</span> in the current configuration.
            </EmptyState>
          </Card>
        </div>
      )
    }
    if (target.error) {
      return (
        <div className="flex flex-col gap-section">
          {back}
          <Alert variant="error">
            <CircleX />
            <AlertTitle>Could not load this target</AlertTitle>
            <AlertDescription>
              {target.error.message}{' '}
              <Button variant="link" className="h-auto p-0" onClick={target.refresh}>
                Try again
              </Button>
            </AlertDescription>
          </Alert>
        </div>
      )
    }
    return (
      <div className="flex flex-col gap-section">
        {back}
        <div className="flex flex-col gap-3">
          <Skeleton className="h-6 w-24 rounded-pill" />
          <Skeleton className="h-14 w-3/4 max-w-xl" />
          <Skeleton className="h-4 w-64" />
        </div>
        <Skeleton className="h-96 rounded-card" />
      </div>
    )
  }

  return (
    <div className="flex flex-col gap-section">
      <div className="flex flex-col gap-4">
        {back}
        <PageHeader
          label="Target"
          labelIcon={<Network aria-hidden />}
          title={<HtmlContent inline html={t.title_html} />}
          description={<TargetInfo t={t} />}
          meta={<StatusPill status={t.status} />}
        />
      </div>

      {t.error && (
        <Alert variant="error">
          <CircleX />
          <AlertTitle>The last poll failed</AlertTitle>
          <AlertDescription className="wrap-break-word">{t.error}</AlertDescription>
        </Alert>
      )}

      {t.alerts.length > 0 && (
        <Alert variant={t.alerts.some((a) => a.firing) ? 'warn' : 'default'}>
          <BellRing />
          <AlertTitle>
            {t.alerts.some((a) => a.firing) ? 'Threshold alert firing' : 'Threshold broken, not firing yet'}
          </AlertTitle>
          <AlertDescription>
            <AlertList alerts={t.alerts} format={t.format} detailed className="text-sm" />
          </AlertDescription>
        </Alert>
      )}

      {hasHtml(t.page_top_html) && (
        <Card>
          <CardContent className="pt-card-pad">
            <HtmlContent html={t.page_top_html} className="text-sm leading-relaxed" />
          </CardContent>
        </Card>
      )}

      {t.periods.length === 0 ? (
        <Card>
          <EmptyState icon={<Clock />} title="No graphs configured">
            Suppress[] hides every period for this target.
          </EmptyState>
        </Card>
      ) : (
        <div className="grid gap-6 2xl:grid-cols-2">
          {PERIODS.filter((p) => t.periods.includes(p)).map((p) => (
            <PeriodGraph key={p} target={t} period={p} />
          ))}
        </div>
      )}

      {hasHtml(t.page_foot_html) && (
        <Card>
          <CardContent className="pt-card-pad">
            <HtmlContent html={t.page_foot_html} className="text-sm leading-relaxed text-muted-foreground" />
          </CardContent>
        </Card>
      )}
    </div>
  )
}

function TargetInfo({ t }: { t: TargetDetail }) {
  const now = useNow()
  const items: { icon: React.ReactNode; label: string; value: React.ReactNode }[] = []
  if (!t.noinfo) {
    items.push({ icon: <Server aria-hidden />, label: 'System', value: t.sys_name || 'unknown' })
    items.push({ icon: <Clock aria-hidden />, label: 'Uptime', value: t.uptime || 'unknown' })
    if (t.max_bytes1 > 0)
      items.push({
        icon: <Gauge aria-hidden />,
        label: 'Max speed',
        value:
          t.max_bytes2 > 0 && t.max_bytes2 !== t.max_bytes1
            ? `${formatRate(t.max_bytes1, t.format)} in, ${formatRate(t.max_bytes2, t.format)} out`
            : formatRate(t.max_bytes1, t.format),
      })
  }
  items.push({
    icon: <Cpu aria-hidden />,
    label: 'Last poll',
    value: t.last_poll ? <time dateTime={new Date(t.last_poll * 1000).toISOString()} title={formatDateTime(t.last_poll)}>{formatAgo(t.last_poll, now)}</time> : 'never',
  })
  return (
    <dl className="flex flex-wrap gap-x-5 gap-y-2">
      {items.map((it) => (
        <div key={it.label} className="flex items-center gap-1.5 [&_svg]:size-3.5 [&_svg]:shrink-0">
          {it.icon}
          <dt className="sr-only sm:not-sr-only">{it.label}:</dt>
          <dd className="font-medium text-foreground">{it.value}</dd>
        </div>
      ))}
      <div className="flex w-full items-center gap-1.5 text-xs">
        <dt>Name:</dt>
        <dd className="font-mono">{t.name}</dd>
        <span aria-hidden>·</span>
        <dt className="sr-only">Target</dt>
        <dd className="min-w-0 break-all font-mono">{t.target_text}</dd>
      </div>
    </dl>
  )
}
