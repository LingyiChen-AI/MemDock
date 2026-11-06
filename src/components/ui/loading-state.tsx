import { Loader2 } from "lucide-react";
import { cn } from "@/lib/utils";

/** Loading placeholder for a panel or page: spinner + label, centered both ways in the space it fills. */
export function LoadingState({ label, className }: { label: string; className?: string }) {
  return (
    <div
      role="status"
      aria-busy="true"
      className={cn("flex h-full min-h-32 w-full flex-1 flex-col items-center justify-center gap-2.5 py-8 text-sm text-muted-foreground", className)}
    >
      <Loader2 className="size-5 animate-spin text-primary motion-reduce:animate-none" aria-hidden="true" />
      <span>{label}</span>
    </div>
  );
}
