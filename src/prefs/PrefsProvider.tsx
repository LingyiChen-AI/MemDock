import * as React from "react";
import { getSettings, updateUiPrefs } from "@/api/memory";
import type { Settings } from "@/api/types";

interface Prefs {
  /** Raw stored value: null = follow system. */
  language: string | null;
  /** Raw stored value: null = default (dark). */
  theme: string | null;
  setLanguage: (language: string) => void;
  setTheme: (theme: string) => void;
}

const PrefsContext = React.createContext<Prefs | null>(null);

/**
 * Holds the persisted UI preferences. Renders immediately with defaults and
 * adopts the stored values once getSettings resolves.
 */
export function PrefsProvider({ children }: { children: React.ReactNode }) {
  const [language, setLanguageState] = React.useState<string | null>(null);
  const [theme, setThemeState] = React.useState<string | null>(null);
  // Once the user changes something, a late getSettings result must not overwrite it.
  const touched = React.useRef(false);

  const adopt = React.useCallback((s: Settings) => {
    setLanguageState(s.language);
    setThemeState(s.theme);
  }, []);

  React.useEffect(() => {
    let alive = true;
    getSettings()
      .then((s) => {
        if (alive && !touched.current) adopt(s);
      })
      .catch(() => {
        // Keep defaults when the backend is unreachable.
      });
    return () => {
      alive = false;
    };
  }, [adopt]);

  const setLanguage = React.useCallback(
    (next: string) => {
      touched.current = true;
      setLanguageState(next === "system" ? null : next);
      updateUiPrefs(next, undefined).then(adopt, () => {});
    },
    [adopt],
  );

  const setTheme = React.useCallback(
    (next: string) => {
      touched.current = true;
      setThemeState(next);
      updateUiPrefs(undefined, next).then(adopt, () => {});
    },
    [adopt],
  );

  const value = React.useMemo(
    () => ({ language, theme, setLanguage, setTheme }),
    [language, theme, setLanguage, setTheme],
  );
  return <PrefsContext.Provider value={value}>{children}</PrefsContext.Provider>;
}

export function usePrefs(): Prefs {
  const ctx = React.useContext(PrefsContext);
  if (!ctx) throw new Error("usePrefs must be used within PrefsProvider");
  return ctx;
}
