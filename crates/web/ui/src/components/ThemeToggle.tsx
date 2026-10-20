import { Moon, Sun } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { useTheme } from '@/lib/theme'
import { cn } from '@/lib/utils'

const faceClass =
  'absolute inset-0 flex items-center justify-center rounded-pill backface-hidden [&_svg]:size-[1.125rem]'

/**
 * One coin: the sun on its face, the moon on its back. Clicking flips it
 * over and switches the theme.
 */
export function ThemeToggle() {
  const { theme, toggle } = useTheme()
  const dark = theme === 'dark'
  const label = dark ? 'Switch to light mode' : 'Switch to dark mode'

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant="outline"
          size="icon"
          onClick={toggle}
          aria-label={label}
          aria-pressed={dark}
          className="perspective-coin size-9 border-0 bg-transparent p-0 hover:bg-transparent"
        >
          <span
            className={cn(
              'relative size-9 preserve-3d transition-transform duration-flip ease-out',
              dark && 'rotate-y-180',
            )}
          >
            <span className={cn(faceClass, 'border border-input bg-card text-primary')}>
              <Sun aria-hidden />
            </span>
            <span className={cn(faceClass, 'rotate-y-180 border border-input bg-accent text-accent-foreground')}>
              <Moon aria-hidden />
            </span>
          </span>
        </Button>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  )
}
