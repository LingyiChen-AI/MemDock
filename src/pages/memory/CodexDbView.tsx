import * as React from "react";
import { LoadingState } from "@/components/ui/loading-state";
import { TriangleAlert } from "lucide-react";
import { readCodexDb } from "@/api/memory";
import type { CodexDbResult } from "@/api/types";
import { useI18n } from "@/i18n/I18nProvider";
import { ToolTag } from "./ToolTag";
import { fmt } from "./paths";

function formatTime(generatedAt: number): string {
  // The Codex db stores unix seconds; tolerate milliseconds too.
  const ms = generatedAt < 1e11 ? generatedAt * 1000 : generatedAt;
  return new Date(ms).toLocaleString();
}

/** Read-only list of the Codex memory store. */
export function CodexDbView() {
  const { t } = useI18n();
  const m = t.memory;
  const [result, setResult] = React.useState<CodexDbResult | null>(null);

  React.useEffect(() => {
    let alive = true;
    readCodexDb().then(
      (r) => alive && setResult(r),
      (e) => alive && setResult({ status: "unavailable", reason: String(e?.message ?? e) }),
    );
    return () => {
      alive = false;
    };
  }, []);

  return (
    <div className="flex h-full min-w-0 flex-1 flex-col">
      <div className="flex h-10 shrink-0 items-center gap-2.5 border-b border-border px-4">
        <ToolTag tool="codex" />
        <span className="truncate font-medium">{m.codexTitle}</span>
        {result?.status === "ok" ? (
          <span className="rounded-full border border-border px-2 text-xs leading-5 whitespace-nowrap text-muted-foreground">
            {fmt(m.codexEntries, { n: result.entries.length })}
          </span>
        ) : null}
      </div>
      <div className="min-h-0 flex-1 overflow-auto p-4">
        {result === null ? (
          <LoadingState label={m.loading} />
        ) : result.status === "unavailable" ? (
          <div className="flex items-center gap-2.5 rounded-md bg-warning/15 px-3 py-2 text-warning">
            <TriangleAlert className="size-4 shrink-0" aria-hidden="true" />
            <span className="text-foreground">{fmt(m.codexUnavailable, { reason: result.reason })}</span>
          </div>
        ) : result.entries.length === 0 ? (
          <p className="text-muted-foreground">{m.codexEmpty}</p>
        ) : (
          <ul className="flex flex-col gap-3">
            {result.entries.map((e) => (
              <li key={e.thread_id} className="rounded-lg border border-border bg-card">
                <div className="flex items-center gap-2 border-b border-border px-3.5 py-2 text-xs text-muted-foreground">
                  <span>{m.codexThread}</span>
                  <span className="truncate font-mono text-[12.5px] text-foreground">{e.thread_id}</span>
                  <span className="ml-auto whitespace-nowrap">{formatTime(e.generated_at)}</span>
                </div>
                <pre className="px-3.5 py-2.5 font-mono text-[12.5px] leading-5 break-words whitespace-pre-wrap">
                  {e.raw_memory}
                </pre>
                {e.rollout_summary ? (
                  <div className="border-t border-border bg-muted px-3.5 py-2 text-xs text-muted-foreground">
                    <span className="font-semibold">{m.codexSummary}</span> {e.rollout_summary}
                  </div>
                ) : null}
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
