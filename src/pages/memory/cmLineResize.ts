import { ViewPlugin, type EditorView, type ViewUpdate } from "@codemirror/view";

type MeasureInternals = { viewState?: { mustMeasureContent?: boolean | "refresh" } };

/**
 * Make CodeMirror re-read the heights of the rendered lines on its next measure.
 *
 * `view.requestMeasure()` alone is not enough: CodeMirror only re-reads line heights when its own
 * update marked the content dirty or when the content box height changed. A reflow it did not
 * cause (a webfont subset swapping in) can move rows between lines while the total stays the same
 * — one line unwraps, another wraps — and then the height map, and with it the line-number gutter,
 * stays stale. CodeMirror itself sets this same flag when `document.fonts.ready` resolves.
 */
export function remeasureLines(view: EditorView) {
  const vs = (view as unknown as MeasureInternals).viewState;
  if (vs && "mustMeasureContent" in vs && vs.mustMeasureContent !== "refresh") vs.mustMeasureContent = true;
  view.requestMeasure();
}

/**
 * Watches every rendered line box; when one changes height without CodeMirror redrawing it
 * (font swap, late CSS), forces a content re-measure so the gutter follows the text.
 */
export const lineResizeRemeasure = ViewPlugin.fromClass(
  class {
    private readonly ro: ResizeObserver | null;
    private readonly heights = new WeakMap<Element, number>();

    constructor(private readonly view: EditorView) {
      this.ro =
        typeof ResizeObserver === "undefined"
          ? null
          : new ResizeObserver((entries) => {
              let changed = false;
              for (const e of entries) {
                const h = e.contentRect.height;
                const prev = this.heights.get(e.target);
                this.heights.set(e.target, h);
                if (prev !== undefined && Math.abs(prev - h) > 0.5) changed = true;
              }
              if (changed) remeasureLines(this.view);
            });
      // Plugins are constructed before the content DOM is drawn: observe once the first layout ran.
      view.requestMeasure({ read: () => this.observeLines() });
    }

    update(u: ViewUpdate) {
      if (u.docChanged || u.viewportChanged || u.geometryChanged || u.heightChanged) this.observeLines();
    }

    /** Called by CodeMirror after it redrew the content DOM: line elements may have been replaced. */
    docViewUpdate() {
      this.observeLines();
    }

    private observeLines() {
      if (!this.ro) return;
      // Re-observing from scratch drops lines CodeMirror detached, so they can be collected.
      this.ro.disconnect();
      for (const el of Array.from(this.view.contentDOM.children)) this.ro.observe(el);
    }

    destroy() {
      this.ro?.disconnect();
    }
  },
);
