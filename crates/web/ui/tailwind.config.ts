import type { Config } from 'tailwindcss'

/** Every colour, radius and shadow resolves to a CSS variable from src/index.css. */
const token = (name: string) => `hsl(var(--${name}) / <alpha-value>)`

export default {
  darkMode: 'selector',
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  theme: {
    container: { center: true, padding: { DEFAULT: '1rem', sm: '1.5rem', lg: '2rem' }, screens: { '2xl': '1400px' } },
    extend: {
      fontFamily: {
        sans: ['Poppins', 'ui-sans-serif', 'system-ui', 'sans-serif'],
        mono: ['ui-monospace', 'SFMono-Regular', 'Menlo', 'monospace'],
      },
      fontSize: {
        display: ['var(--text-display)', { lineHeight: '1.05', letterSpacing: 'var(--tracking-display)', fontWeight: '600' }],
        title: ['var(--text-title)', { lineHeight: '1.15', letterSpacing: 'var(--tracking-title)', fontWeight: '600' }],
        stat: ['var(--text-stat)', { lineHeight: '1.1', letterSpacing: 'var(--tracking-title)', fontWeight: '600' }],
        label: ['var(--text-label)', { lineHeight: '1', letterSpacing: 'var(--tracking-label)', fontWeight: '600' }],
      },
      colors: {
        border: token('border'),
        input: token('input'),
        ring: token('ring'),
        background: token('background'),
        foreground: token('foreground'),
        primary: { DEFAULT: token('primary'), foreground: token('primary-foreground') },
        secondary: { DEFAULT: token('secondary'), foreground: token('secondary-foreground') },
        muted: { DEFAULT: token('muted'), foreground: token('muted-foreground') },
        accent: { DEFAULT: token('accent'), foreground: token('accent-foreground') },
        popover: { DEFAULT: token('popover'), foreground: token('popover-foreground') },
        card: { DEFAULT: token('card'), foreground: token('card-foreground') },
        brand: {
          teal: token('brand-teal'),
          mid: token('brand-mid'),
          sage: token('brand-sage'),
          slate: token('brand-slate'),
          forest: token('brand-forest'),
        },
        status: {
          ok: token('status-ok'),
          'ok-soft': token('status-ok-soft'),
          warn: token('status-warn'),
          'warn-soft': token('status-warn-soft'),
          error: token('status-error'),
          'error-soft': token('status-error-soft'),
          stale: token('status-stale'),
          'stale-soft': token('status-stale-soft'),
          pending: token('status-pending'),
          'pending-soft': token('status-pending-soft'),
        },
        chart: {
          in: token('chart-in'),
          out: token('chart-out'),
          'in-peak': token('chart-in-peak'),
          'out-peak': token('chart-out-peak'),
          grid: token('chart-grid'),
        },
      },
      borderRadius: {
        card: 'var(--radius-card)',
        lg: 'var(--radius-lg)',
        md: 'var(--radius-md)',
        sm: 'var(--radius-sm)',
        pill: '9999px',
      },
      boxShadow: {
        card: 'var(--shadow-card)',
        'card-hover': 'var(--shadow-card-hover)',
        pop: 'var(--shadow-pop)',
      },
      spacing: {
        gutter: 'var(--space-gutter)',
        section: 'var(--space-section)',
        'card-pad': 'var(--space-card)',
      },
      height: { chart: 'var(--chart-height)', spark: 'var(--spark-height)' },
      keyframes: {
        'fade-in': { from: { opacity: '0' }, to: { opacity: '1' } },
      },
      animation: { 'fade-in': 'fade-in 160ms ease-out' },
    },
  },
  plugins: [],
} satisfies Config
