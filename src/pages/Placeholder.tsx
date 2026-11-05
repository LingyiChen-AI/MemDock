import type { LucideIcon } from "lucide-react";
import { useI18n } from "@/i18n/I18nProvider";
import { AppearanceControls } from "@/components/appearance/AppearanceControls";

/** Empty state for pages that land in later plans; also hosts the appearance controls until the settings page exists. */
export function Placeholder({ icon: Icon }: { icon: LucideIcon }) {
  const { t } = useI18n();

  return (
    <div className="flex h-full flex-col items-center justify-center gap-6">
      <div className="flex flex-col items-center gap-3">
        <Icon className="size-8 text-muted-foreground" aria-hidden="true" />
        <p className="text-sm text-muted-foreground">{t.placeholder.comingSoon}</p>
      </div>
      <AppearanceControls />
    </div>
  );
}
