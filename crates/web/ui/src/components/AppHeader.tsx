import { Activity, RefreshCw } from "lucide-react";
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
          <span className="flex size-9 items-center justify-center rounded-pill bg-primary text-primary-foreground">
            <Activity className="size-5" aria-hidden strokeWidth={2.25} />
          </span>
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
          <ThemeToggle />
        </div>
      </div>
    </div>
  );
}
