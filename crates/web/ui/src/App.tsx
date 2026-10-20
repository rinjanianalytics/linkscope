import { Route, Switch, useLocation } from 'wouter'
import { TooltipProvider } from '@/components/ui/tooltip'
import { AppHeader } from '@/components/AppHeader'
import { ErrorBoundary } from '@/components/ErrorBoundary'
import { Overview } from '@/pages/Overview'
import { TargetPage } from '@/pages/TargetPage'
import { NotFound } from '@/pages/NotFound'
import { api } from '@/lib/api'
import { usePolling } from '@/lib/hooks'

const LIST_REFRESH_MS = 30_000

function decode(s: string) {
  try {
    return decodeURIComponent(s)
  } catch {
    return s
  }
}

export default function App() {
  // Summary and the target list are shared by every screen and refreshed together.
  const summary = usePolling((signal) => api.summary(signal), LIST_REFRESH_MS, [])
  const targets = usePolling((signal) => api.targets(signal), LIST_REFRESH_MS, [])
  const [location] = useLocation()

  return (
    <TooltipProvider delayDuration={200}>
      <a
        href="#main"
        className="sr-only z-50 rounded-pill bg-primary px-4 py-2 text-primary-foreground focus:not-sr-only focus:fixed focus:left-4 focus:top-4"
      >
        Skip to content
      </a>
      <AppHeader
        version={summary.data?.version}
        updatedAt={targets.updatedAt}
        loading={summary.loading || targets.loading}
      />
      <main id="main" className="mx-auto w-full max-w-screen-2xl px-gutter py-section">
        <ErrorBoundary key={location}>
          <Switch>
            <Route path="/">
              <Overview summary={summary} targets={targets} />
            </Route>
            <Route path="/target/:name">{(params) => <TargetPage name={decode(params.name)} />}</Route>
            <Route>
              <NotFound />
            </Route>
          </Switch>
        </ErrorBoundary>
      </main>
      <footer className="mx-auto w-full max-w-screen-2xl px-gutter pb-section text-xs text-muted-foreground">
        <span className="flex flex-wrap gap-x-2 gap-y-1">
          <span>LinkScope{summary.data ? ` ${summary.data.version}` : ''} by Rinjani Analytics</span>
          <span aria-hidden>·</span>
          <span>
            Based on{' '}
            <a
              href="https://oss.oetiker.ch/mrtg/"
              target="_blank"
              rel="noreferrer"
              className="underline underline-offset-4 hover:text-foreground"
            >
              MRTG
            </a>{' '}
            by Tobias Oetiker and contributors
          </span>
          <span aria-hidden>·</span>
          <span>GPL-2.0-or-later</span>
        </span>
      </footer>
    </TooltipProvider>
  )
}
