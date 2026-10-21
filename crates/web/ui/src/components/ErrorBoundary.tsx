import { Component, type ErrorInfo, type ReactNode } from 'react'
import { AlertTriangle, RotateCw } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { EmptyState } from '@/components/EmptyState'

interface Props {
  children: ReactNode
}

interface State {
  error: Error | null
}

/**
 * A screen that throws while drawing shows its error and a reload button,
 * instead of leaving the whole page blank. App keys it by location, so
 * moving to another screen tries again.
 */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null }

  static getDerivedStateFromError(error: Error): State {
    return { error }
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error('LinkScope: a screen failed to draw', error, info.componentStack)
  }

  render() {
    const { error } = this.state
    if (!error) return this.props.children
    return (
      <EmptyState icon={<AlertTriangle />} title="This screen failed to draw">
        <p className="mb-4 wrap-break-word font-mono text-xs">{error.message || String(error)}</p>
        <Button variant="outline" size="sm" onClick={() => window.location.reload()}>
          <RotateCw aria-hidden />
          Reload
        </Button>
      </EmptyState>
    )
  }
}
