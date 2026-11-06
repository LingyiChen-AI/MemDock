import * as React from "react";
import { Lock, RotateCcw } from "lucide-react";
import { extractPromptDefaults } from "@/api/extract";
import type { ExtractConfig, PromptDefaults } from "@/api/types";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import { fmt } from "@/pages/memory/paths";

/**
 * The extraction rules sent to the model: the built-in text, or the user's own.
 * The fixed clauses (safety, output format) are shown but cannot be edited.
 */
export function PromptSettings({ config, saving, onSave }: { config: ExtractConfig; saving: boolean; onSave: (c: ExtractConfig) => void }) {
  const { t } = useI18n();
  const x = t.extract.prompt;
  const [defaults, setDefaults] = React.useState<PromptDefaults | null>(null);
  const [text, setText] = React.useState("");
  const custom = config.custom_prompt?.trim() ? config.custom_prompt : null;

  React.useEffect(() => {
    let alive = true;
    extractPromptDefaults(config.lang)
      .then((d) => alive && setDefaults(d))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [config.lang]);

  React.useEffect(() => {
    if (defaults) setText(custom ?? defaults.rules);
  }, [defaults, custom]);

  if (!defaults) return null;
  const saved = custom ?? defaults.rules;
  const dirty = text !== saved;
  const tooLong = text.length > defaults.max_chars;
  const empty = !text.trim();
  const save = () => {
    const value = text.trim() === defaults.rules.trim() ? null : text;
    onSave({ ...config, custom_prompt: value });
  };

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2">
        <span
          className={cn(
            "rounded-full px-2 py-0.5 text-[11px] font-medium",
            custom ? "bg-primary/12 text-primary" : "bg-muted text-muted-foreground",
          )}
        >
          {custom ? x.custom : x.builtIn}
        </span>
        <p className="text-xs text-muted-foreground">{fmt(x.hint, { placeholder: defaults.placeholder })}</p>
      </div>
      <label htmlFor="extract-prompt" className="sr-only">
        {x.title}
      </label>
      <Textarea
        id="extract-prompt"
        value={text}
        onChange={(e) => setText(e.target.value)}
        spellCheck={false}
        rows={14}
        className="min-h-56 font-mono text-[12.5px] leading-relaxed"
      />
      {tooLong ? <p className="text-xs text-destructive">{fmt(x.tooLong, { n: defaults.max_chars })}</p> : null}
      {empty ? <p className="text-xs text-muted-foreground">{x.emptyHint}</p> : null}

      <div className="flex flex-col gap-1.5 rounded-md border border-dashed border-border bg-muted/30 p-3">
        <span className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
          <Lock className="size-3" aria-hidden="true" />
          {x.fixedTitle}
        </span>
        <pre className="font-mono text-[12px] leading-relaxed whitespace-pre-wrap text-muted-foreground [overflow-wrap:anywhere]">{defaults.fixed}</pre>
        <p className="text-[11px] text-muted-foreground">{x.appended}</p>
      </div>

      <div className="flex flex-wrap items-center justify-end gap-2">
        {custom || dirty ? (
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={saving}
            onClick={() => (custom ? onSave({ ...config, custom_prompt: null }) : setText(defaults.rules))}
            className="cursor-pointer"
          >
            <RotateCcw aria-hidden="true" />
            {custom ? x.reset : x.discard}
          </Button>
        ) : null}
        <Button type="button" size="sm" disabled={!dirty || tooLong || saving} onClick={save} className="cursor-pointer font-semibold">
          {x.save}
        </Button>
      </div>
    </div>
  );
}
