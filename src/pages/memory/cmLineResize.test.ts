import { vi } from "vitest";
import { EditorView } from "@codemirror/view";
import { EditorState } from "@codemirror/state";
import { lineResizeRemeasure, remeasureLines } from "./cmLineResize";

type Internals = { viewState: { mustMeasureContent: boolean | "refresh" } };

describe("lineResizeRemeasure", () => {
  const RealRO = globalThis.ResizeObserver;
  // CodeMirror itself also creates ResizeObservers; remember which callback watches which element.
  let observers: { cb: ResizeObserverCallback; els: Element[] }[] = [];
  const observed = () => observers.flatMap((o) => o.els);
  const callbackFor = (el: Element) => observers.find((o) => o.els.includes(el))!.cb;

  beforeEach(() => {
    observers = [];
    globalThis.ResizeObserver = class {
      private o: { cb: ResizeObserverCallback; els: Element[] };
      constructor(cb: ResizeObserverCallback) {
        this.o = { cb, els: [] };
        observers.push(this.o);
      }
      observe(el: Element) {
        this.o.els.push(el);
      }
      unobserve() {}
      disconnect() {
        this.o.els = [];
      }
    } as unknown as typeof ResizeObserver;
  });
  afterEach(() => {
    globalThis.ResizeObserver = RealRO;
  });

  // Tripwire for CodeMirror upgrades: the fix relies on this (internal) flag, the same one
  // CodeMirror sets itself when document.fonts.ready resolves.
  it("relies on a viewState.mustMeasureContent flag that this CodeMirror version still has", () => {
    const view = new EditorView({ state: EditorState.create({ doc: "a" }) });
    const vs = (view as unknown as Internals).viewState;
    expect("mustMeasureContent" in vs).toBe(true);
    vs.mustMeasureContent = false;
    remeasureLines(view);
    expect(vs.mustMeasureContent).toBe(true);
    view.destroy();
  });

  it("forces a content re-measure when a rendered line changes height on its own", () => {
    const view = new EditorView({ state: EditorState.create({ doc: "one\ntwo", extensions: [lineResizeRemeasure] }) });
    view.dispatch({ changes: { from: 0, insert: "x" } }); // redraw -> lines get observed
    const line = view.contentDOM.querySelector(".cm-line")!;
    expect(observed()).toContain(line);
    const callback = callbackFor(line);

    const vs = (view as unknown as Internals).viewState;
    const spy = vi.spyOn(view, "requestMeasure");
    const entry = (height: number) => [{ target: line, contentRect: { height } }] as unknown as ResizeObserverEntry[];

    vs.mustMeasureContent = false;
    callback(entry(24), {} as ResizeObserver); // first sighting: nothing to compare against
    expect(spy).not.toHaveBeenCalled();
    callback(entry(24), {} as ResizeObserver); // unchanged
    expect(spy).not.toHaveBeenCalled();

    callback(entry(48), {} as ResizeObserver); // font swap re-wrapped the line
    expect(spy).toHaveBeenCalledTimes(1);
    expect(vs.mustMeasureContent).toBe(true);

    view.destroy();
    expect(observed()).not.toContain(line);
  });
});
