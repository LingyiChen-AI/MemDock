import * as React from "react";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import { usePrefs } from "@/prefs/PrefsProvider";

interface Option {
  value: string;
  label: string;
}

function Segmented({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: string;
  options: Option[];
  onChange: (value: string) => void;
}) {
  return (
    <div className="flex items-center gap-3">
      <span className="w-14 shrink-0 text-xs text-muted-foreground">{label}</span>
      <div role="radiogroup" aria-label={label} className="inline-flex rounded-md border border-border bg-card p-0.5">
        {options.map((o) => {
          const checked = o.value === value;
          return (
            <button
              key={o.value}
              type="button"
              role="radio"
              aria-checked={checked}
              onClick={() => onChange(o.value)}
              className={cn(
                "h-6 cursor-pointer rounded-sm px-3 text-xs outline-none transition-colors duration-150 focus-visible:ring-2 focus-visible:ring-ring motion-reduce:transition-none",
                checked ? "bg-muted font-medium text-foreground" : "text-muted-foreground hover:text-foreground",
              )}
            >
              {o.label}
            </button>
          );
        })}
      </div>
    </div>
  );
}

const SWATCHES = [
  { value: "green", color: "var(--swatch-green)" },
  { value: "blue", color: "var(--swatch-blue)" },
  { value: "violet", color: "var(--swatch-violet)" },
  { value: "orange", color: "var(--swatch-orange)" },
  { value: "teal", color: "var(--swatch-teal)" },
] as const;

function AccentPicker({
  label,
  names,
  value,
  onChange,
}: {
  label: string;
  names: Record<string, string>;
  value: string;
  onChange: (value: string) => void;
}) {
  const refs = React.useRef<(HTMLButtonElement | null)[]>([]);
  const move = (from: number, delta: number) => {
    const next = (from + delta + SWATCHES.length) % SWATCHES.length;
    onChange(SWATCHES[next].value);
    refs.current[next]?.focus();
  };
  return (
    <div className="flex items-center gap-3">
      <span className="w-14 shrink-0 text-xs text-muted-foreground">{label}</span>
      <div role="radiogroup" aria-label={label} className="flex items-center gap-3 p-1">
        {SWATCHES.map((s, i) => {
          const checked = s.value === value;
          return (
            <button
              key={s.value}
              ref={(el) => {
                refs.current[i] = el;
              }}
              type="button"
              role="radio"
              aria-checked={checked}
              aria-label={names[s.value]}
              title={names[s.value]}
              tabIndex={checked ? 0 : -1}
              onClick={() => onChange(s.value)}
              onKeyDown={(e) => {
                if (e.key === "ArrowRight" || e.key === "ArrowDown") {
                  e.preventDefault();
                  move(i, 1);
                } else if (e.key === "ArrowLeft" || e.key === "ArrowUp") {
                  e.preventDefault();
                  move(i, -1);
                }
              }}
              style={{ backgroundColor: s.color }}
              className={cn(
                "size-6 cursor-pointer rounded-full outline-none transition-shadow duration-150 focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background motion-reduce:transition-none",
                checked && "ring-2 ring-ring ring-offset-2 ring-offset-background",
              )}
            />
          );
        })}
      </div>
    </div>
  );
}

/** Mode / accent / language controls; shared by the sidebar appearance popover and the placeholder pages. */
export function AppearanceControls({ className }: { className?: string }) {
  const { t } = useI18n();
  const a = t.appearance;
  const { language, theme, accent, setLanguage, setTheme, setAccent } = usePrefs();

  return (
    <div className={cn("flex flex-col items-start gap-3", className)}>
      <Segmented
        label={a.mode}
        value={theme ?? "dark"}
        onChange={setTheme}
        options={[
          { value: "dark", label: a.dark },
          { value: "light", label: a.light },
          { value: "system", label: a.system },
        ]}
      />
      <AccentPicker label={a.accent} names={a.accents} value={accent ?? "green"} onChange={setAccent} />
      <Segmented
        label={a.language}
        value={language ?? "system"}
        onChange={setLanguage}
        options={[
          { value: "zh", label: "中文" },
          { value: "en", label: "English" },
          { value: "system", label: a.system },
        ]}
      />
    </div>
  );
}
