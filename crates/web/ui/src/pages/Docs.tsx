import { useEffect, useState } from 'react'
import { BookOpen, ChevronLeft, ChevronRight, FileQuestion } from 'lucide-react'
import { Link, Redirect, useLocation } from 'wouter'
import { Card } from '@/components/ui/card'
import { Skeleton } from '@/components/ui/skeleton'
import { EmptyState } from '@/components/EmptyState'
import { HtmlContent } from '@/components/HtmlContent'
import { SectionLabel } from '@/components/SectionLabel'
import { api, type DocPage, type DocsIndex } from '@/lib/api'
import { cn } from '@/lib/utils'

/** The manual's own words around the pages, per language. */
const WORDS: Record<string, { onThisPage: string; pages: string; prev: string; next: string; missing: string; back: string }> = {
  en: {
    onThisPage: 'On this page',
    pages: 'Pages',
    prev: 'Previous',
    next: 'Next',
    missing: 'This page is not in the manual.',
    back: 'Back to the manual',
  },
  id: {
    onThisPage: 'Di halaman ini',
    pages: 'Halaman',
    prev: 'Sebelumnya',
    next: 'Berikutnya',
    missing: 'Halaman ini tidak ada di panduan.',
    back: 'Kembali ke panduan',
  },
}
const words = (lang: string) => WORDS[lang] ?? WORDS.en

const LANG_KEY = 'linkscope.docs.lang'

/** The reader's language: the one last chosen here, else the browser's, else English. */
function preferredLang(): string {
  try {
    const saved = localStorage.getItem(LANG_KEY)
    if (saved) return saved
  } catch {
    // Storage can be blocked; the browser's language is enough.
  }
  return navigator.language.toLowerCase().startsWith('id') ? 'id' : 'en'
}

function rememberLang(lang: string) {
  try {
    localStorage.setItem(LANG_KEY, lang)
  } catch {
    // Not remembered; nothing else depends on it.
  }
}

function useDocsIndex() {
  const [index, setIndex] = useState<DocsIndex>()
  const [error, setError] = useState<Error>()
  useEffect(() => {
    const ctrl = new AbortController()
    api.docs(ctrl.signal).then(setIndex, (e: Error) => {
      if (!ctrl.signal.aborted) setError(e)
    })
    return () => {
      ctrl.abort()
    }
  }, [])
  return { index, error }
}

/** `/docs` and `/docs/:lang`: the first page in the reader's language. */
export function DocsHome({ lang }: { lang?: string }) {
  const { index, error } = useDocsIndex()
  if (error) return <Missing lang={lang ?? 'en'} />
  if (!index) return <DocsSkeleton />
  const want = lang ?? preferredLang()
  const language = index.languages.find((l) => l.code === want) ?? index.languages[0]
  const first = language?.pages[0]
  if (!first) return <Missing lang={want} />
  return <Redirect to={`/docs/${language.code}/${first.slug}`} replace />
}

/** `/docs/:lang/:slug`: one page of the manual. */
export function DocsPage({ lang, slug }: { lang: string; slug: string }) {
  const { index } = useDocsIndex()
  const [page, setPage] = useState<DocPage>()
  const [failed, setFailed] = useState(false)
  const [, navigate] = useLocation()
  const w = words(lang)

  useEffect(() => {
    const ctrl = new AbortController()
    setFailed(false)
    api.docPage(lang, slug, ctrl.signal).then(
      (p) => {
        setPage(p)
        document.title = `${p.title} · LinkScope`
      },
      () => {
        if (!ctrl.signal.aborted) setFailed(true)
      },
    )
    return () => {
      ctrl.abort()
    }
  }, [lang, slug])

  // Scroll to the heading the address names, else to the top, once the page is drawn. A block
  // body: an effect returns its cleanup only, and scrollIntoView may return a Promise.
  useEffect(() => {
    if (!page || page.slug !== slug) return
    let id = window.location.hash.slice(1)
    try {
      id = decodeURIComponent(id)
    } catch {
      // A malformed escape: look the raw text up instead of failing the page.
    }
    const el = id ? document.getElementById(id) : null
    if (el) {
      el.scrollIntoView({ block: 'start' })
    } else {
      window.scrollTo({ top: 0 })
    }
  }, [page, slug])

  if (failed) return <Missing lang={lang} />
  const language = index?.languages.find((l) => l.code === lang)

  // Links between pages stay in the app instead of loading the whole UI again.
  const follow = (e: React.MouseEvent<HTMLElement>) => {
    const a = (e.target as HTMLElement).closest('a')
    const href = a?.getAttribute('href')
    if (!href || !href.startsWith('/docs/') || e.metaKey || e.ctrlKey || e.shiftKey || e.button !== 0) return
    e.preventDefault()
    navigate(href)
  }

  const switchTo = (code: string) => {
    rememberLang(code)
    const other = index?.languages.find((l) => l.code === code)
    const same = other?.pages.some((p) => p.slug === slug)
    navigate(`/docs/${code}/${same ? slug : (other?.pages[0]?.slug ?? slug)}`)
  }

  const contents = (
    <ul className="flex flex-col gap-0.5">
      {language?.pages.map((p) => (
        <li key={p.slug}>
          <Link
            href={`/docs/${lang}/${p.slug}`}
            aria-current={p.slug === slug ? 'page' : undefined}
            className={cn(
              'block rounded-md px-3 py-1.5 text-sm text-muted-foreground hover:bg-muted hover:text-foreground',
              p.slug === slug && 'bg-muted font-medium text-foreground',
            )}
          >
            {p.title}
          </Link>
        </li>
      ))}
    </ul>
  )

  return (
    <div className="grid gap-6 lg:grid-cols-[14rem_minmax(0,1fr)] xl:grid-cols-[14rem_minmax(0,1fr)_13rem]">
      <aside className="hidden lg:block">
        <nav aria-label={language?.title ?? 'Manual'} className="sticky top-6 flex flex-col gap-3">
          <SectionLabel icon={<BookOpen aria-hidden />}>{language?.title ?? 'LinkScope'}</SectionLabel>
          {contents}
        </nav>
      </aside>

      <div className="flex min-w-0 flex-col gap-6">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <details className="lg:hidden">
            <summary className="cursor-pointer text-sm font-medium">{w.pages}</summary>
            <nav aria-label={language?.title ?? 'Manual'} className="mt-2">
              {contents}
            </nav>
          </details>
          <div className="ml-auto flex rounded-pill border border-border p-0.5" role="group" aria-label="Language">
            {index?.languages.map((l) => (
              <button
                key={l.code}
                type="button"
                title={l.name}
                aria-pressed={l.code === lang}
                onClick={() => switchTo(l.code)}
                className={cn(
                  'rounded-pill px-3 py-1 text-xs font-semibold uppercase text-muted-foreground hover:text-foreground',
                  l.code === lang && 'bg-primary text-primary-foreground hover:text-primary-foreground',
                )}
              >
                {l.code}
              </button>
            ))}
          </div>
        </div>

        <Card className="p-6 sm:p-8">
          {page && page.slug === slug && page.lang === lang ? (
            <article lang={lang} onClick={follow}>
              <h1 className="text-display break-words">{page.title}</h1>
              <HtmlContent html={page.html} className="doc-prose mt-6" />
            </article>
          ) : (
            <DocsSkeleton />
          )}
        </Card>

        {page && page.slug === slug && (
          <nav className="flex flex-wrap justify-between gap-3 text-sm" aria-label={`${w.prev} / ${w.next}`}>
            {page.prev ? (
              <Link href={`/docs/${lang}/${page.prev.slug}`} className="flex items-center gap-1 text-muted-foreground hover:text-foreground">
                <ChevronLeft aria-hidden className="size-4" />
                <span>
                  <span className="sr-only">{w.prev}: </span>
                  {page.prev.title}
                </span>
              </Link>
            ) : (
              <span />
            )}
            {page.next && (
              <Link href={`/docs/${lang}/${page.next.slug}`} className="ml-auto flex items-center gap-1 text-muted-foreground hover:text-foreground">
                <span>
                  <span className="sr-only">{w.next}: </span>
                  {page.next.title}
                </span>
                <ChevronRight aria-hidden className="size-4" />
              </Link>
            )}
          </nav>
        )}
      </div>

      {page && page.slug === slug && page.headings.length > 0 && (
        <aside className="hidden xl:block">
          <nav aria-label={w.onThisPage} className="sticky top-6 flex flex-col gap-2 text-sm">
            <span className="font-semibold">{w.onThisPage}</span>
            <ul className="flex flex-col gap-1">
              {page.headings.map((h) => (
                <li key={h.id} className={cn(h.level === 3 && 'pl-3')}>
                  <a href={`#${h.id}`} className="text-muted-foreground hover:text-foreground">
                    {h.text}
                  </a>
                </li>
              ))}
            </ul>
          </nav>
        </aside>
      )}
    </div>
  )
}

function DocsSkeleton() {
  return (
    <div className="flex flex-col gap-4">
      <Skeleton className="h-9 w-2/3" />
      <Skeleton className="h-4 w-full" />
      <Skeleton className="h-4 w-11/12" />
      <Skeleton className="h-4 w-4/5" />
    </div>
  )
}

function Missing({ lang }: { lang: string }) {
  const w = words(lang)
  return (
    <Card>
      <EmptyState icon={<FileQuestion />} title={w.missing}>
        <Link href="/docs" className="text-sm font-medium text-primary underline underline-offset-4">
          {w.back}
        </Link>
      </EmptyState>
    </Card>
  )
}
