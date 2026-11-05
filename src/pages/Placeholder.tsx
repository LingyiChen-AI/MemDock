import type { LucideIcon } from "lucide-react";
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
      <span className="text-xs text-muted-foreground">{label}</span>
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

/** Empty state for pages that land in later plans; also hosts the language/theme switches until the settings page exists. */
export function Placeholder({ icon: Icon }: { icon: LucideIcon }) {
  const { t } = useI18n();
  const { language, theme, setLanguage, setTheme } = usePrefs();

  return (
    <div className="flex h-full flex-col items-center justify-center gap-6">
      <div className="flex flex-col items-center gap-3">
        <Icon className="size-8 text-muted-foreground" aria-hidden="true" />
        <p className="text-sm text-muted-foreground">{t.placeholder.comingSoon}</p>
      </div>
      <div className="flex flex-col items-start gap-3">
        <Segmented
          label={t.placeholder.language}
          value={language ?? "system"}
          onChange={setLanguage}
          options={[
            { value: "system", label: t.placeholder.system },
            { value: "zh", label: "中文" },
            { value: "en", label: "English" },
          ]}
        />
        <Segmented
          label={t.placeholder.theme}
          value={theme ?? "dark"}
          onChange={setTheme}
          options={[
            { value: "system", label: t.placeholder.system },
            { value: "dark", label: t.placeholder.dark },
            { value: "light", label: t.placeholder.light },
          ]}
        />
      </div>
    </div>
  );
}
