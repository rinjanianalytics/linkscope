import { Route, Switch, useLocation } from 'wouter'
import { TooltipProvider } from '@/components/ui/tooltip'
import { AppHeader } from '@/components/AppHeader'
import { AppFooter } from '@/components/AppFooter'
import { ErrorBoundary } from '@/components/ErrorBoundary'
import { Overview } from '@/pages/Overview'
import { TargetPage } from '@/pages/TargetPage'
import { DocsHome, DocsPage } from '@/pages/Docs'
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
      {/* A full-height column, so the footer sits at the bottom of short pages. */}
      <div className="flex min-h-svh flex-col">
        <AppHeader
          version={summary.data?.version}
          updatedAt={targets.updatedAt}
          loading={summary.loading || targets.loading}
        />
        <main id="main" className="mx-auto w-full flex-1 max-w-(--breakpoint-2xl) px-gutter py-section">
          <ErrorBoundary key={location}>
            <Switch>
              <Route path="/">
                <Overview summary={summary} targets={targets} />
              </Route>
              <Route path="/target/:name">{(params) => <TargetPage name={decode(params.name)} />}</Route>
              <Route path="/docs">
                <DocsHome />
              </Route>
              <Route path="/docs/:lang">{(params) => <DocsHome lang={decode(params.lang)} />}</Route>
              <Route path="/docs/:lang/:slug">{(params) => <DocsPage lang={decode(params.lang)} slug={decode(params.slug)} />}</Route>
              <Route>
                <NotFound />
              </Route>
            </Switch>
          </ErrorBoundary>
        </main>
        <AppFooter version={summary.data?.version} />
      </div>
    </TooltipProvider>
  )
}
