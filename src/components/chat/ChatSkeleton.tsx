import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";

/** Loading state for any conversation view: alternating placeholder bubbles. */
export function ChatSkeleton({ label, rows = 4, className }: { label: string; rows?: number; className?: string }) {
  const widths = ["w-2/5", "w-3/4", "w-1/3", "w-2/3", "w-1/2", "w-3/5"];
  return (
    <div role="status" aria-busy="true" aria-label={label} className={cn("flex flex-col gap-5 py-2", className)}>
      {Array.from({ length: rows }, (_, i) => {
        const user = i % 2 === 0;
        return (
          <div key={i} className={cn("flex gap-2.5", user && "flex-row-reverse")}>
            <Skeleton className="size-7 shrink-0 rounded-full" />
            <div className={cn("flex min-w-0 flex-1 flex-col gap-1.5", user && "items-end")}>
              <Skeleton className="h-3 w-24" />
              <Skeleton className={cn("rounded-2xl", user ? "h-10" : "h-20", widths[i % widths.length])} />
            </div>
          </div>
        );
      })}
      <span className="sr-only">{label}</span>
    </div>
  );
}
