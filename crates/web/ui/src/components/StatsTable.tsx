import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { Swatch } from '@/components/ui/chart'
import { HtmlContent } from './HtmlContent'
import type { SanitizedHtml, SeriesStats } from '@/lib/api'

interface StatsTableProps {
  stats: SeriesStats
  legendIn: SanitizedHtml
  legendOut: SanitizedHtml
  inColour: string
  outColour: string
  noI?: boolean
  noO?: boolean
  avgPeak?: boolean
}

type Metric = 'max' | 'avg' | 'avg_max' | 'cur'
type Dir = 'in' | 'out'

/**
 * MRTG's Max / Average / Current table, using the server's preformatted
 * strings. Wide screens use MRTG's layout (a row per direction); narrow ones
 * transpose it so two value columns fit.
 */
export function StatsTable({ stats, legendIn, legendOut, inColour, outColour, noI, noO, avgPeak }: StatsTableProps) {
  const metrics: { key: Metric; label: string }[] = [
    { key: 'max', label: 'Max' },
    { key: 'avg', label: 'Average' },
    ...(avgPeak ? [{ key: 'avg_max' as const, label: 'Average max' }] : []),
    { key: 'cur', label: 'Current' },
  ]
  const dirs: { key: Dir; legend: SanitizedHtml; colour: string; kind: 'area' | 'line' }[] = []
  if (!noI) dirs.push({ key: 'in', legend: legendIn, colour: inColour, kind: 'area' })
  if (!noO) dirs.push({ key: 'out', legend: legendOut, colour: outColour, kind: 'line' })

  const value = (m: Metric, d: Dir) => stats.text[`${m}_${d}`] ?? 'n/a'
  const dirLabel = (d: (typeof dirs)[number]) => (
    <span className="flex items-center gap-2">
      <Swatch indicator={d.kind} colorVar={d.colour} />
      <HtmlContent inline html={d.legend} />
    </span>
  )

  return (
    <>
      <Table className="hidden sm:table">
        <TableHeader>
          <TableRow>
            <TableHead className="pl-0">
              <span className="sr-only">Series</span>
            </TableHead>
            {metrics.map((m) => (
              <TableHead key={m.key} className="text-right">
                {m.label}
              </TableHead>
            ))}
          </TableRow>
        </TableHeader>
        <TableBody>
          {dirs.map((d) => (
            <TableRow key={d.key}>
              <TableHead scope="row" className="pl-0 font-medium text-foreground">
                {dirLabel(d)}
              </TableHead>
              {metrics.map((m) => (
                <TableCell key={m.key} className="whitespace-nowrap text-right tabular-nums">
                  {value(m.key, d.key)}
                </TableCell>
              ))}
            </TableRow>
          ))}
        </TableBody>
      </Table>

      <Table className="sm:hidden">
        <TableHeader>
          <TableRow>
            <TableHead className="pl-0">
              <span className="sr-only">Statistic</span>
            </TableHead>
            {dirs.map((d) => (
              <TableHead key={d.key} className="pr-0 text-right">
                <span className="inline-flex justify-end">{dirLabel(d)}</span>
              </TableHead>
            ))}
          </TableRow>
        </TableHeader>
        <TableBody>
          {metrics.map((m) => (
            <TableRow key={m.key}>
              <TableHead scope="row" className="pl-0 font-medium text-foreground">
                {m.label}
              </TableHead>
              {dirs.map((d) => (
                <TableCell key={d.key} className="pr-0 text-right text-xs tabular-nums">
                  {value(m.key, d.key)}
                </TableCell>
              ))}
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </>
  )
}
