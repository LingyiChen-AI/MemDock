import * as React from "react";
import { CircleCheck, CircleX, Info, TriangleAlert, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";

export type ToastKind = "success" | "info" | "warning" | "error";

export interface Toast {
  id: number;
  kind: ToastKind;
  message: string;
}

const LIFETIME: Record<ToastKind, number> = { success: 4500, info: 4500, warning: 9000, error: 9000 };

export function useToasts() {
  const [toasts, setToasts] = React.useState<Toast[]>([]);
  const next = React.useRef(1);
  const timers = React.useRef(new Map<number, number>());
  const dismiss = React.useCallback((id: number) => {
    window.clearTimeout(timers.current.get(id));
    timers.current.delete(id);
    setToasts((l) => l.filter((x) => x.id !== id));
  }, []);
  // Pending timers must not fire after the page is gone.
  React.useEffect(() => {
    const pending = timers.current;
    return () => {
      pending.forEach((h) => window.clearTimeout(h));
      pending.clear();
    };
  }, []);
  const push = React.useCallback(
    (kind: ToastKind, message: string) => {
      const id = next.current++;
      setToasts((l) => [...l.slice(-3), { id, kind, message }]);
      timers.current.set(
        id,
        window.setTimeout(() => dismiss(id), LIFETIME[kind]),
      );
    },
    [dismiss],
  );
  return { toasts, push, dismiss };
}

const ICON = { success: CircleCheck, info: Info, warning: TriangleAlert, error: CircleX } as const;
const TONE = { success: "text-accent-text", info: "text-accent-text", warning: "text-warning", error: "text-destructive" } as const;

function ToastItem({ toast: x, onDismiss }: { toast: Toast; onDismiss: (id: number) => void }) {
  const { t } = useI18n();
  const [expanded, setExpanded] = React.useState(false);
  const long = x.message.length > 120 || x.message.split("\n").length > 3;
  const Icon = ICON[x.kind];
  return (
    <div className="pointer-events-auto flex items-start gap-2.5 rounded-md border border-border bg-popover px-3 py-2.5 text-sm text-popover-foreground shadow-lg">
      <Icon className={cn("mt-0.5 size-4 shrink-0", TONE[x.kind])} aria-hidden="true" />
      <div className="min-w-0 flex-1">
        <p className={cn("break-words whitespace-pre-wrap", long && !expanded && "line-clamp-3")}>{x.message}</p>
        {long ? (
          <button
            type="button"
            aria-expanded={expanded}
            onClick={() => setExpanded((v) => !v)}
            className="mt-1 cursor-pointer rounded-sm text-xs text-accent-text outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring"
          >
            {expanded ? t.skills.collapse : t.skills.expand}
          </button>
        ) : null}
      </div>
      <button
        type="button"
        aria-label={t.skills.close}
        onClick={() => onDismiss(x.id)}
        className="grid size-5 shrink-0 cursor-pointer place-items-center rounded-sm text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
      >
        <X className="size-3.5" aria-hidden="true" />
      </button>
    </div>
  );
}

export function ToastRegion({ toasts, onDismiss }: { toasts: Toast[]; onDismiss: (id: number) => void }) {
  return (
    <div role="status" aria-live="polite" className="pointer-events-none fixed right-4 bottom-4 z-[60] flex w-[360px] max-w-[calc(100vw-32px)] flex-col gap-2">
      {toasts.map((x) => (
        <ToastItem key={x.id} toast={x} onDismiss={onDismiss} />
      ))}
    </div>
  );
}
