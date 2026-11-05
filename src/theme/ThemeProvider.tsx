import * as React from "react";
import { usePrefs } from "@/prefs/PrefsProvider";

const QUERY = "(prefers-color-scheme: dark)";

function systemPrefersDark(): boolean {
  return typeof window.matchMedia === "function" ? window.matchMedia(QUERY).matches : true;
}

/** Toggles `.dark` on <html>. null -> dark; "system" follows prefers-color-scheme live. */
export function ThemeProvider({ children }: { children: React.ReactNode }) {
  const { theme } = usePrefs();

  React.useEffect(() => {
    const root = document.documentElement;
    const apply = (dark: boolean) => root.classList.toggle("dark", dark);

    if (theme === "system" && typeof window.matchMedia === "function") {
      const mql = window.matchMedia(QUERY);
      apply(mql.matches);
      const onChange = (e: { matches: boolean }) => apply(e.matches);
      mql.addEventListener("change", onChange);
      return () => mql.removeEventListener("change", onChange);
    }
    apply(theme === "system" ? systemPrefersDark() : theme !== "light");
  }, [theme]);

  return <>{children}</>;
}
