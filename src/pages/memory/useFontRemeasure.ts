import * as React from "react";
import type { EditorView } from "@codemirror/view";

/**
 * CodeMirror measures wrapped line heights once; when a webfont swaps in later the heights change
 * and the line-number gutter drifts. Re-measure when fonts finish loading.
 */
export function useFontRemeasure(view: EditorView | null) {
  React.useEffect(() => {
    if (!view) return;
    const fonts = typeof document === "undefined" ? undefined : document.fonts;
    if (!fonts) return;
    let alive = true;
    const measure = () => {
      if (alive) view.requestMeasure();
    };
    void fonts.ready.then(measure);
    fonts.addEventListener("loadingdone", measure);
    return () => {
      alive = false;
      fonts.removeEventListener("loadingdone", measure);
    };
  }, [view]);
}
