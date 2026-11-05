import * as React from "react";
import type { EditorView } from "@codemirror/view";
import { lineForPreviewTop, previewTopForLine, withSentinel, type Block } from "./scrollSync";

const LOCK_MS = 150;
/** Without user input for this long the leader is released, so programmatic scrolls (search reveal) can lead. */
const LEADER_IDLE_MS = 300;

type Pane = "editor" | "preview";

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
 * The pane the user is interacting with is the leader (released after a short idle); a short lock
 * after each programmatic scroll stops the follower's own scroll event from bouncing back.
 * Returns `leadEditor`, to mark an upcoming programmatic editor scroll (search reveal) as editor-led.
 */
export function useScrollSync(opts: {
  enabled: boolean;
  view: EditorView | null;
  previewRef: React.RefObject<HTMLElement | null>;
}) {
  const { enabled, view, previewRef } = opts;
  const leaderRef = React.useRef<Pane | null>(null);
  const idleTimer = React.useRef<number | undefined>(undefined);

  const setLeader = React.useCallback((who: Pane) => {
    leaderRef.current = who;
    window.clearTimeout(idleTimer.current);
    idleTimer.current = window.setTimeout(() => {
      leaderRef.current = null;
    }, LEADER_IDLE_MS);
  }, []);
  const leadEditor = React.useCallback(() => setLeader("editor"), [setLeader]);
  React.useEffect(() => () => window.clearTimeout(idleTimer.current), []);

  React.useEffect(() => {
    const preview = previewRef.current;
    const editor = view?.scrollDOM;
    if (!enabled || !view || !preview || !editor) return;

    const lock = { editor: 0, preview: 0 };
    const pad = () => view.documentPadding?.top ?? 0;

    /** Fractional source line at editor scroll offset `scrollTop`. */
    const editorPos = (scrollTop: number) => {
      const h = Math.max(0, scrollTop - pad());
      const lb = view.lineBlockAtHeight(h);
      const line = view.state.doc.lineAt(lb.from).number;
      const fraction = lb.height > 0 ? Math.min(0.999, Math.max(0, (h - lb.top) / lb.height)) : 0;
      return line + fraction;
    };
    /** Preview blocks plus an end marker: editor at max scroll <-> preview at max scroll. */
    const blocksWithEnd = () => {
      const blocks = readBlocks(preview);
      if (blocks.length === 0) return null;
      const maxEditor = Math.max(0, editor.scrollHeight - editor.clientHeight);
      const endPos = Math.max(editorPos(maxEditor), 1);
      const maxPreview = Math.max(0, preview.scrollHeight - preview.clientHeight);
      return { blocks: withSentinel(blocks, endPos, maxPreview), endPos, maxEditor };
    };

    const onEditorScroll = () => {
      const l = leaderRef.current;
      if (performance.now() < lock.editor || (l && l !== "editor")) return;
      const b = blocksWithEnd();
      if (!b) return;
      const pos = editor.scrollTop >= b.maxEditor - 1 ? b.endPos : editorPos(editor.scrollTop);
      const line = Math.floor(pos);
      lock.preview = performance.now() + LOCK_MS;
      preview.scrollTop = previewTopForLine(b.blocks, line, pos - line);
    };

    const onPreviewScroll = () => {
      const l = leaderRef.current;
      if (performance.now() < lock.preview || (l && l !== "preview")) return;
      const b = blocksWithEnd();
      if (!b) return;
      lock.editor = performance.now() + LOCK_MS;
      const target = Math.min(lineForPreviewTop(b.blocks, preview.scrollTop), b.endPos);
      if (target >= b.endPos) {
        editor.scrollTop = b.maxEditor;
        return;
      }
      const doc = view.state.doc;
      const n = Math.min(doc.lines, Math.max(1, Math.floor(target)));
      const lb = view.lineBlockAt(doc.line(n).from);
      editor.scrollTop = lb.top + pad() + (target - n) * lb.height;
    };

    const lead = (who: Pane) => () => {
      setLeader(who);
      lock[who] = 0;
    };
    const leadEditorEv = lead("editor");
    const leadPreviewEv = lead("preview");
    const paneEvents = ["pointerenter", "pointerdown", "wheel", "keydown", "touchstart"] as const;
    paneEvents.forEach((ev) => {
      editor.addEventListener(ev, leadEditorEv, { passive: true });
      preview.addEventListener(ev, leadPreviewEv, { passive: true });
    });
    editor.addEventListener("scroll", onEditorScroll, { passive: true });
    preview.addEventListener("scroll", onPreviewScroll, { passive: true });
    return () => {
      paneEvents.forEach((ev) => {
        editor.removeEventListener(ev, leadEditorEv);
        preview.removeEventListener(ev, leadPreviewEv);
      });
      editor.removeEventListener("scroll", onEditorScroll);
      preview.removeEventListener("scroll", onPreviewScroll);
    };
  }, [enabled, view, previewRef, setLeader]);

  return { leadEditor };
}
