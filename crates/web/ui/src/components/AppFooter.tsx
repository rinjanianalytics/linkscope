import { Github, Globe } from "lucide-react";

const WEBSITE = "https://rinjanianalytics.com";
const SOURCE = "https://github.com/rinjanianalytics/linkscope";

const link =
  "rounded-pill underline-offset-4 hover:text-foreground hover:underline focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring";

interface AppFooterProps {
  version?: string;
}

export function AppFooter({ version }: AppFooterProps) {
  return (
    <footer className="border-t border-border">
      <div className="mx-auto flex w-full max-w-(--breakpoint-2xl) flex-col gap-4 px-gutter py-6 text-xs text-muted-foreground sm:flex-row sm:items-center sm:justify-between">
        <div className="flex flex-col gap-1.5">
          <div className="flex items-center gap-2">
            {/* The Rinjani Analytics mark, as in the header. */}
            <img src="/logo.svg" alt="" width={20} height={20} className="size-5 shrink-0 select-none" />
            <span>
              LinkScope{version ? ` ${version}` : ""} by{" "}
              <a href={WEBSITE} target="_blank" rel="noreferrer" className={`font-medium text-foreground ${link}`}>
                Rinjani Analytics
              </a>
            </span>
          </div>
          <span className="flex flex-col gap-x-2 gap-y-1 sm:flex-row sm:flex-wrap">
            <span>
              Based on{" "}
              <a href="https://oss.oetiker.ch/mrtg/" target="_blank" rel="noreferrer" className={`underline ${link}`}>
                MRTG
              </a>{" "}
              by Tobias Oetiker and contributors
            </span>
            <span aria-hidden className="hidden sm:inline">·</span>
            <span>GPL-2.0-or-later</span>
          </span>
        </div>
        <nav aria-label="Rinjani Analytics" className="flex flex-wrap items-center gap-x-5 gap-y-2">
          <a href={SOURCE} target="_blank" rel="noreferrer" className={`flex items-center gap-1.5 ${link}`}>
            <Github aria-hidden className="size-4" />
            GitHub
          </a>
          <a href={WEBSITE} target="_blank" rel="noreferrer" className={`flex items-center gap-1.5 ${link}`}>
            <Globe aria-hidden className="size-4" />
            rinjanianalytics.com
          </a>
        </nav>
      </div>
    </footer>
  );
}
