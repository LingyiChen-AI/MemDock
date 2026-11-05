import * as React from "react";
import { PrefsProvider } from "@/prefs/PrefsProvider";
import { ThemeProvider } from "@/theme/ThemeProvider";
import { I18nProvider } from "@/i18n/I18nProvider";
import { TooltipProvider } from "@/components/ui/tooltip";

export function AppProviders({ children }: { children: React.ReactNode }) {
  return (
    <PrefsProvider>
      <ThemeProvider>
        <I18nProvider>
          <TooltipProvider>{children}</TooltipProvider>
        </I18nProvider>
      </ThemeProvider>
    </PrefsProvider>
  );
}
