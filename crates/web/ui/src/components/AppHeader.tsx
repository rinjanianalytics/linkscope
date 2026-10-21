import { BookOpen, RefreshCw } from "lucide-react";
import { Link } from "wouter";
import { ThemeToggle } from "@/components/ThemeToggle";
import { cn } from "@/lib/utils";

interface AppHeaderProps {
  version?: string;
  updatedAt?: number;
  loading?: boolean;
}

export function AppHeader({ version, updatedAt, loading }: AppHeaderProps) {
  return (
    <div className="border-b border-border bg-card/80 backdrop-blur supports-[backdrop-filter]:bg-card/70">
      <div className="mx-auto flex h-16 max-w-screen-2xl items-center justify-between gap-4 px-gutter">
        <Link
          href="/"
          className="flex items-center gap-2.5 rounded-pill font-semibold tracking-tight focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          {/* The Rinjani Analytics mark (public/logo.svg), the same artwork as PathScope's and FlowScope's. */}
          <img src="/logo.svg" alt="" width={36} height={36} className="size-9 shrink-0 select-none" />
          <span className="text-lg">LinkScope</span>
          {version && (
            <span className="text-xs font-normal text-muted-foreground">
              v{version}
            </span>
          )}
        </Link>
        <div className="flex items-center gap-3">
          <div
            className="flex items-center gap-2 text-xs text-muted-foreground"
            aria-live="polite"
          >
            <RefreshCw
              aria-hidden
              className={cn("size-3.5", loading && "motion-safe:animate-spin")}
            />
            <span className="hidden sm:inline">
              {updatedAt
                ? `Updated ${new Date(updatedAt).toLocaleTimeString()}`
                : "Loading"}
            </span>
          </div>
          <Link
            href="/docs"
            className="flex items-center gap-1.5 rounded-pill px-2 py-1 text-sm text-muted-foreground hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <BookOpen aria-hidden className="size-4" />
            <span className="hidden sm:inline">Docs</span>
            <span className="sr-only sm:hidden">Docs</span>
          </Link>
          <ThemeToggle />
        </div>
      </div>
    </div>
  );
}
