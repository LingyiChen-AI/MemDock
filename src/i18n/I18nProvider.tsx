import * as React from "react";
import { invoke } from "@tauri-apps/api/core";
import { usePrefs } from "@/prefs/PrefsProvider";
import { stableContext } from "@/lib/stableContext";
import { getMessages, htmlLang, resolveLanguage, type Language, type Messages } from "./index";

interface I18n {
  language: Language;
  t: Messages;
}

const I18nContext = stableContext<I18n | null>("i18n", null);

export function I18nProvider({ children }: { children: React.ReactNode }) {
  const { language: setting } = usePrefs();
  const language = resolveLanguage(setting, navigator.language);

  React.useEffect(() => {
    document.documentElement.lang = htmlLang(language);
  }, [language]);

  // Tray menu labels follow the UI language (only inside the Tauri app).
  React.useEffect(() => {
    if (!("__TAURI_INTERNALS__" in window)) return;
    const t = getMessages(language).tray;
    void invoke("set_tray_labels", { show: t.show, quit: t.quit }).catch(() => {});
  }, [language]);

  const value = React.useMemo(() => ({ language, t: getMessages(language) }), [language]);
  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

export function useI18n(): I18n {
  const ctx = React.useContext(I18nContext);
  if (!ctx) throw new Error("useI18n must be used within I18nProvider");
  return ctx;
}
