import { render } from "@testing-library/react";
import { vi } from "vitest";
import type { EditorView } from "@codemirror/view";
import { useFontRemeasure } from "./useFontRemeasure";

function Probe({ view }: { view: EditorView }) {
  useFontRemeasure(view);
  return null;
}

describe("useFontRemeasure", () => {
  const original = Object.getOwnPropertyDescriptor(document, "fonts");
  afterEach(() => {
    if (original) Object.defineProperty(document, "fonts", original);
    else delete (document as unknown as { fonts?: unknown }).fonts;
  });

  it("re-measures on fonts.ready and on loadingdone, and detaches on unmount", async () => {
    const fonts = new EventTarget() as EventTarget & { ready: Promise<unknown> };
    fonts.ready = Promise.resolve();
    const removeSpy = vi.spyOn(fonts, "removeEventListener");
    Object.defineProperty(document, "fonts", { value: fonts, configurable: true });
    const view = { requestMeasure: vi.fn() } as unknown as EditorView;

    const { unmount } = render(<Probe view={view} />);
    await fonts.ready;
    await Promise.resolve();
    expect(view.requestMeasure).toHaveBeenCalledTimes(1);

    fonts.dispatchEvent(new Event("loadingdone"));
    expect(view.requestMeasure).toHaveBeenCalledTimes(2);

    unmount();
    expect(removeSpy).toHaveBeenCalledWith("loadingdone", expect.any(Function));
    fonts.dispatchEvent(new Event("loadingdone"));
    expect(view.requestMeasure).toHaveBeenCalledTimes(2);
  });
});
