import * as React from "react";
import { getSettings, updateUiPrefs } from "@/api/memory";
import type { Settings } from "@/api/types";

interface Prefs {
  /** Raw stored value: null = follow system. */
  language: string | null;
  /** Raw stored value: null = default (dark). */
  theme: string | null;
  /** Raw stored value: null = default (green). */
  accent: string | null;
  setLanguage: (language: string) => void;
  setTheme: (theme: string) => void;
  setAccent: (accent: string) => void;
}

const PrefsContext = React.createContext<Prefs | null>(null);

/**
 * Holds the persisted UI preferences. Renders immediately with defaults and
 * adopts the stored values once getSettings resolves.
 */
export function PrefsProvider({ children }: { children: React.ReactNode }) {
  const [language, setLanguageState] = React.useState<string | null>(null);
  const [theme, setThemeState] = React.useState<string | null>(null);
  const [accent, setAccentState] = React.useState<string | null>(null);
  // Once the user changes something, a late getSettings result must not overwrite it.
  const touched = React.useRef(false);

  const adopt = React.useCallback((s: Settings) => {
    setLanguageState(s.language);
    setThemeState(s.theme);
    setAccentState(s.accent ?? null);
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

  // Per-field sequence counters: only the latest change of a field may adopt or roll back.
  const langSeq = React.useRef(0);
  const themeSeq = React.useRef(0);
  const accentSeq = React.useRef(0);
  const langRef = React.useRef<string | null>(null);
  const themeRef = React.useRef<string | null>(null);
  langRef.current = language;
  themeRef.current = theme;
  const accentRef = React.useRef<string | null>(null);
  accentRef.current = accent;

  const setLanguage = React.useCallback((next: string) => {
    touched.current = true;
    const prev = langRef.current;
    const seq = ++langSeq.current;
    setLanguageState(next === "system" ? null : next);
    updateUiPrefs(next, undefined).then(
      (s) => {
        if (seq === langSeq.current) setLanguageState(s.language);
      },
      (err) => {
        console.error("failed to save language preference", err);
        if (seq === langSeq.current) setLanguageState(prev);
      },
    );
  }, []);

  const setTheme = React.useCallback((next: string) => {
    touched.current = true;
    const prev = themeRef.current;
    const seq = ++themeSeq.current;
    setThemeState(next);
    updateUiPrefs(undefined, next).then(
      (s) => {
        if (seq === themeSeq.current) setThemeState(s.theme);
      },
      (err) => {
        console.error("failed to save theme preference", err);
        if (seq === themeSeq.current) setThemeState(prev);
      },
    );
  }, []);

  const setAccent = React.useCallback((next: string) => {
    touched.current = true;
    const prev = accentRef.current;
    const seq = ++accentSeq.current;
    setAccentState(next);
    updateUiPrefs(undefined, undefined, next).then(
      (s) => {
        if (seq === accentSeq.current) setAccentState(s.accent ?? null);
      },
      (err) => {
        console.error("failed to save accent preference", err);
        if (seq === accentSeq.current) setAccentState(prev);
      },
    );
  }, []);

  const value = React.useMemo(
    () => ({ language, theme, accent, setLanguage, setTheme, setAccent }),
    [language, theme, accent, setLanguage, setTheme, setAccent],
  );
  return <PrefsContext.Provider value={value}>{children}</PrefsContext.Provider>;
}

export function usePrefs(): Prefs {
  const ctx = React.useContext(PrefsContext);
  if (!ctx) throw new Error("usePrefs must be used within PrefsProvider");
  return ctx;
}
