import type { ITheme } from "@xterm/xterm";

/** The terminal is dark in both app themes (CLIs assume a dark background). */
export const TERMINAL_THEME: ITheme = {
  background: "#16181d",
  foreground: "#d7dae0",
  cursor: "#d7dae0",
  cursorAccent: "#16181d",
  selectionBackground: "#3a4150",
  black: "#1e2127",
  red: "#e06c75",
  green: "#98c379",
  yellow: "#d19a66",
  blue: "#61afef",
  magenta: "#c678dd",
  cyan: "#56b6c2",
  white: "#abb2bf",
  brightBlack: "#5c6370",
  brightRed: "#f2777a",
  brightGreen: "#b5e890",
  brightYellow: "#ebcb8b",
  brightBlue: "#7cc0ff",
  brightMagenta: "#de9bf0",
  brightCyan: "#7fd4df",
  brightWhite: "#ffffff",
};

/** Status bar colors matching the terminal. */
export const TERMINAL_CHROME = {
  background: "#16181d",
  border: "rgba(255, 255, 255, 0.1)",
  text: "#abb2bf",
  strong: "#d7dae0",
  error: "#e06c75",
  /** Border of the focused pane of a split terminal. */
  focus: "#3fb950",
  /** Pane context menu. */
  menu: "#1e2127",
  paneHeader: "rgba(255, 255, 255, 0.04)",
};

export const TERMINAL_FONT =
  '"Cascadia Mono", "Cascadia Code", Consolas, Menlo, "SF Mono", "JetBrains Mono Variable", "DejaVu Sans Mono", "Microsoft YaHei UI", "PingFang SC", "Noto Sans Mono CJK SC", monospace';
