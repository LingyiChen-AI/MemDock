import * as React from "react";
import type { EditorView } from "@codemirror/view";
import { lineForPreviewTop, previewTopForLine, type Block } from "./scrollSync";

const LOCK_MS = 150;

function readBlocks(scroller: HTMLElement): Block[] {
  const base = scroller.getBoundingClientRect().top - scroller.scrollTop;
  const out: Block[] = [];
  scroller.querySelectorAll<HTMLElement>("[data-line]").forEach((el) => {
    const line = Number(el.dataset.line);
    if (Number.isFinite(line)) out.push({ line, top: el.getBoundingClientRect().top - base });
  });
  // Source order is document order, but guard against odd nesting (list item inside a list item).
  out.sort((a, b) => a.line - b.line || a.top - b.top);
  return out;
}

/**
 * Keeps an editor and a preview scroller aligned by Markdown source line.
 * The pane the user is interacting with is the leader; a short lock after each programmatic
 * scroll stops the follower's own scroll event from bouncing back.
 */
export function useScrollSync(opts: {
  enabled: boolean;
  view: EditorView | null;
  previewRef: React.RefObject<HTMLElement | null>;
}) {
  const { enabled, view, previewRef } = opts;

  React.useEffect(() => {
    const preview = previewRef.current;
    const editor = view?.scrollDOM;
    if (!enabled || !view || !preview || !editor) return;

    let leader: "editor" | "preview" | null = null;
    const lock = { editor: 0, preview: 0 };
    const pad = () => view.documentPadding?.top ?? 0;

    const onEditorScroll = () => {
      if (performance.now() < lock.editor || (leader && leader !== "editor")) return;
      const blocks = readBlocks(preview);
      if (blocks.length === 0) return;
      const h = Math.max(0, editor.scrollTop - pad());
      const lb = view.lineBlockAtHeight(h);
      const line = view.state.doc.lineAt(lb.from).number;
      const fraction = lb.height > 0 ? Math.min(0.999, Math.max(0, (h - lb.top) / lb.height)) : 0;
      lock.preview = performance.now() + LOCK_MS;
      preview.scrollTop = previewTopForLine(blocks, line, fraction);
    };

    const onPreviewScroll = () => {
      if (performance.now() < lock.preview || (leader && leader !== "preview")) return;
      const blocks = readBlocks(preview);
      if (blocks.length === 0) return;
      const target = lineForPreviewTop(blocks, preview.scrollTop);
      const doc = view.state.doc;
      const n = Math.min(doc.lines, Math.max(1, Math.floor(target)));
      const lb = view.lineBlockAt(doc.line(n).from);
      lock.editor = performance.now() + LOCK_MS;
      editor.scrollTop = lb.top + pad() + (target - n) * lb.height;
    };

    const lead = (who: "editor" | "preview") => () => {
      leader = who;
      lock[who] = 0;
    };
    const leadEditor = lead("editor");
    const leadPreview = lead("preview");
    const paneEvents = ["pointerenter", "pointerdown", "wheel", "keydown", "touchstart"] as const;
    paneEvents.forEach((ev) => {
      editor.addEventListener(ev, leadEditor, { passive: true });
      preview.addEventListener(ev, leadPreview, { passive: true });
    });
    editor.addEventListener("scroll", onEditorScroll, { passive: true });
    preview.addEventListener("scroll", onPreviewScroll, { passive: true });
    return () => {
      paneEvents.forEach((ev) => {
        editor.removeEventListener(ev, leadEditor);
        preview.removeEventListener(ev, leadPreview);
      });
      editor.removeEventListener("scroll", onEditorScroll);
      preview.removeEventListener("scroll", onPreviewScroll);
    };
  }, [enabled, view, previewRef]);
}
