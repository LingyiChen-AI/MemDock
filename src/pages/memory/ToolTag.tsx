import type { Tool } from "@/api/types";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";

/** Design system 7: radius 4, height 18, 600 weight, tool colour at 18% as background. */
export function ToolTag({ tool, className }: { tool: Tool; className?: string }) {
  const { t } = useI18n();
  const claude = tool === "claude";
  return (
    <span
      className={cn(
        "inline-flex h-[18px] shrink-0 items-center rounded-sm px-[5px] text-xs leading-[18px] font-semibold",
        claude ? "bg-tool-claude/18 text-tool-claude" : "bg-tool-codex/18 text-tool-codex",
        className,
      )}
    >
      {claude ? t.memory.toolClaude : t.memory.toolCodex}
    </span>
  );
}
