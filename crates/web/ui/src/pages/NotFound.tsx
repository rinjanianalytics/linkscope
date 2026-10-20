import { ArrowLeft, MapPinOff } from 'lucide-react'
import { Link } from 'wouter'
import { Button } from '@/components/ui/button'
import { Card } from '@/components/ui/card'
import { EmptyState } from '@/components/EmptyState'

export function NotFound() {
  return (
    <Card>
      <EmptyState icon={<MapPinOff />} title="Page not found">
        <Button variant="link" asChild className="h-auto p-0">
          <Link href="/">
            <ArrowLeft aria-hidden />
            Back to the overview
          </Link>
        </Button>
      </EmptyState>
    </Card>
  )
}
