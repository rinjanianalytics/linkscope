import { useCallback, useEffect, useState } from 'react'

export type Theme = 'light' | 'dark'

/** Also used by public/theme-init.js, which applies the theme before paint. */
export const THEME_KEY = 'linkscope-theme'

const query = () => window.matchMedia('(prefers-color-scheme: dark)')

function savedTheme(): Theme | null {
  try {
    const v = localStorage.getItem(THEME_KEY)
    return v === 'light' || v === 'dark' ? v : null
  } catch {
    return null
  }
}

function apply(theme: Theme) {
  document.documentElement.classList.toggle('dark', theme === 'dark')
}

/**
 * The active theme. Follows the system until the user picks one; the
 * choice is remembered in this browser.
 */
export function useTheme() {
  const [theme, setTheme] = useState<Theme>(() =>
    document.documentElement.classList.contains('dark') ? 'dark' : 'light',
  )

  useEffect(() => {
    apply(theme)
  }, [theme])

  // Track the system setting while the user has not chosen.
  useEffect(() => {
    const mq = query()
    const onChange = (e: MediaQueryListEvent) => {
      if (!savedTheme()) setTheme(e.matches ? 'dark' : 'light')
    }
    mq.addEventListener('change', onChange)
    return () => mq.removeEventListener('change', onChange)
  }, [])

  const toggle = useCallback(() => {
    setTheme((t) => {
      const next: Theme = t === 'dark' ? 'light' : 'dark'
      try {
        localStorage.setItem(THEME_KEY, next)
      } catch {
        /* private mode: the choice lasts for this page only */
      }
      return next
    })
  }, [])

  return { theme, toggle }
}
