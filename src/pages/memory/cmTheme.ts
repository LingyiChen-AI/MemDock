import { EditorView, Decoration, MatchDecorator, ViewPlugin, type DecorationSet, type ViewUpdate } from "@codemirror/view";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { tags as t } from "@lezer/highlight";
import type { Extension } from "@codemirror/state";

const CONTENT = '"Inter Variable", "LXGW WenKai Screen", "PingFang SC", "Microsoft YaHei UI", sans-serif';
const MONO = '"JetBrains Mono Variable", "Cascadia Code", Menlo, Consolas, monospace';

/** Every colour is a semantic CSS variable, so light/dark follows the app theme without rebuilding. */
const chrome = EditorView.theme({
  "&": {
    height: "100%",
    color: "var(--foreground)",
    backgroundColor: "var(--card)",
    fontSize: "13.5px",
  },
  "&.cm-focused": { outline: "none" },
  ".cm-scroller": { fontFamily: CONTENT, lineHeight: "24px", overflow: "auto" },
  ".cm-content": { caretColor: "var(--primary)", padding: "16px 0" },
  ".cm-line": { padding: "0 16px 0 8px" },
  ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--primary)", borderLeftWidth: "2px" },
  ".cm-gutters": {
    backgroundColor: "var(--card)",
    color: "var(--muted-foreground)",
    border: "none",
    fontFamily: MONO,
    fontSize: "12.5px",
  },
  ".cm-lineNumbers .cm-gutterElement": { opacity: "0.7", padding: "0 8px 0 16px", minWidth: "44px" },
  ".cm-activeLine": { backgroundColor: "color-mix(in srgb, var(--muted) 40%, transparent)" },
  ".cm-activeLineGutter": { backgroundColor: "transparent", color: "var(--foreground)", opacity: "1" },
  "&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection":
    { backgroundColor: "color-mix(in srgb, var(--primary) 28%, transparent)" },
  ".cm-import": { color: "var(--tool-codex)" },
});

const highlight = syntaxHighlighting(
  HighlightStyle.define([
    { tag: t.heading1, color: "var(--accent-text)", fontWeight: "600", fontSize: "1.15em" },
    { tag: [t.heading, t.heading2, t.heading3, t.heading4], color: "var(--accent-text)", fontWeight: "600" },
    { tag: t.strong, fontWeight: "600" },
    { tag: t.emphasis, fontStyle: "italic" },
    { tag: [t.link, t.url], color: "var(--accent-text)" },
    // Inline code and fenced code bodies keep the monospace font.
    { tag: t.monospace, color: "var(--tool-codex)", fontFamily: MONO, fontSize: "12.5px" },
    // `#`, `*`, `**`, backticks, list bullets and quote markers stay quiet.
    { tag: [t.processingInstruction, t.contentSeparator, t.list], color: "var(--muted-foreground)" },
    { tag: t.quote, color: "var(--muted-foreground)" },
  ]),
);

// `@path/to/file.md` imports, as in the design mockup.
const importMatcher = new MatchDecorator({
  regexp: /(?<=^|\s)@[^\s)\],;]+/g,
  decoration: Decoration.mark({ class: "cm-import" }),
});
const importMarks = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    constructor(view: EditorView) {
      this.decorations = importMatcher.createDeco(view);
    }
    update(u: ViewUpdate) {
      this.decorations = importMatcher.updateDeco(u, this.decorations);
    }
  },
  { decorations: (v) => v.decorations },
);

export const memdockCmTheme: Extension[] = [chrome, highlight, importMarks, EditorView.lineWrapping];
