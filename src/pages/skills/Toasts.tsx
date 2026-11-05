import * as React from "react";
import { CircleCheck, CircleX, TriangleAlert, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";

export type ToastKind = "success" | "warning" | "error";

export interface Toast {
  id: number;
  kind: ToastKind;
  message: string;
}

const LIFETIME: Record<ToastKind, number> = { success: 4500, warning: 9000, error: 9000 };

export function useToasts() {
  const [toasts, setToasts] = React.useState<Toast[]>([]);
  const next = React.useRef(1);
  const dismiss = React.useCallback((id: number) => setToasts((l) => l.filter((x) => x.id !== id)), []);
  const push = React.useCallback(
    (kind: ToastKind, message: string) => {
      const id = next.current++;
      setToasts((l) => [...l.slice(-3), { id, kind, message }]);
      window.setTimeout(() => dismiss(id), LIFETIME[kind]);
    },
    [dismiss],
  );
  return { toasts, push, dismiss };
}

const ICON = { success: CircleCheck, warning: TriangleAlert, error: CircleX } as const;
const TONE = { success: "text-accent-text", warning: "text-warning", error: "text-destructive" } as const;

export function ToastRegion({ toasts, onDismiss }: { toasts: Toast[]; onDismiss: (id: number) => void }) {
  const { t } = useI18n();
  return (
    <div role="status" aria-live="polite" className="pointer-events-none fixed right-4 bottom-4 z-[60] flex w-[360px] max-w-[calc(100vw-32px)] flex-col gap-2">
      {toasts.map((x) => {
        const Icon = ICON[x.kind];
        return (
          <div key={x.id} className="pointer-events-auto flex items-start gap-2.5 rounded-md border border-border bg-popover px-3 py-2.5 text-sm text-popover-foreground shadow-lg">
            <Icon className={cn("mt-0.5 size-4 shrink-0", TONE[x.kind])} aria-hidden="true" />
            <span className="min-w-0 flex-1 break-words">{x.message}</span>
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
      })}
    </div>
  );
}
