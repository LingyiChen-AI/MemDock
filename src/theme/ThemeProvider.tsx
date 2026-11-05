import * as React from "react";
import { usePrefs } from "@/prefs/PrefsProvider";

export const ACCENTS = ["green", "blue", "violet", "orange", "teal"] as const;

const QUERY = "(prefers-color-scheme: dark)";

function systemPrefersDark(): boolean {
  return typeof window.matchMedia === "function" ? window.matchMedia(QUERY).matches : true;
}

/** Toggles `.dark` and `data-accent` on <html>. null -> dark; "system" follows prefers-color-scheme live. */
export function ThemeProvider({ children }: { children: React.ReactNode }) {
  const { theme, accent } = usePrefs();

  React.useEffect(() => {
    document.documentElement.dataset.accent = (ACCENTS as readonly string[]).includes(accent ?? "")
      ? (accent as string)
      : "green";
  }, [accent]);

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
