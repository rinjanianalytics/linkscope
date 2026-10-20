import { cn } from '@/lib/utils'
import type { SanitizedHtml } from '@/lib/api'

interface HtmlContentProps {
  /** Must be one of the API's server-sanitized `*_html` / legend fields. */
  html: SanitizedHtml
  inline?: boolean
  className?: string
}

/**
 * The single place the UI renders HTML. The type only accepts strings the API
 * marks as sanitized; everything else is rendered as React text.
 */
export function HtmlContent({ html, inline, className }: HtmlContentProps) {
  const Tag = inline ? 'span' : 'div'
  return <Tag className={cn('mrtg-html', className)} dangerouslySetInnerHTML={{ __html: html }} />
}

/** True when sanitized HTML has visible content. */
export function hasHtml(html: SanitizedHtml | null | undefined): html is SanitizedHtml {
  return !!html && html.replace(/<[^>]*>|&nbsp;|\s/g, '').length > 0
}
