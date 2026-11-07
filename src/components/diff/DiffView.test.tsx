import { act, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "@/test/fixtures";
import { DiffView } from "./DiffView";

const OLD = "line 1\nline 2\nfoo bar baz\nline 4\nline 5\n";
const NEW = "line 1\nline 2\nfoo qux baz\nline 4\nline 5\n";

type RoCallback = (entries: { contentRect: { width: number } }[]) => void;
let roCallbacks: RoCallback[] = [];
const OriginalRO = globalThis.ResizeObserver;

beforeEach(() => {
  localStorage.clear();
  roCallbacks = [];
  globalThis.ResizeObserver = class {
    constructor(cb: RoCallback) {
      roCallbacks.push(cb);
    }
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
});
afterEach(() => {
  globalThis.ResizeObserver = OriginalRO;
});

const resize = (width: number) => act(() => roCallbacks.forEach((cb) => cb([{ contentRect: { width } }])));

describe("DiffView", () => {
  it("defaults to split and remembers unified choice", async () => {
    const { container, unmount } = renderApp(<DiffView oldText={OLD} newText={NEW} oldLabel="当前" newLabel="建议改为" />);
    const root = container.querySelector("[data-diff-layout]");
    expect(root).toHaveAttribute("data-diff-layout", "split");
    expect(screen.getByRole("radio", { name: "左右" })).toBeChecked();
    expect(screen.getByText("当前")).toBeInTheDocument();
    expect(screen.getByText("建议改为")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("radio", { name: "上下" }));
    expect(root).toHaveAttribute("data-diff-layout", "unified");
    expect(localStorage.getItem("memdock.diffLayout")).toBe("unified");

    unmount();
    const again = renderApp(<DiffView oldText={OLD} newText={NEW} />);
    expect(again.container.querySelector("[data-diff-layout]")).toHaveAttribute("data-diff-layout", "unified");
    expect(screen.getByRole("radio", { name: "上下" })).toBeChecked();
  });

  it("falls back to unified when narrow", () => {
    const { container } = renderApp(<DiffView oldText={OLD} newText={NEW} />);
    const root = container.querySelector("[data-diff-layout]");
    resize(759);
    expect(root).toHaveAttribute("data-diff-layout", "unified");
    // The stored preference is untouched and the toggle is not offered while narrow.
    expect(localStorage.getItem("memdock.diffLayout")).toBeNull();
    expect(screen.queryByRole("radio")).toBeNull();
    resize(760);
    expect(root).toHaveAttribute("data-diff-layout", "split");
    expect(screen.getByRole("radio", { name: "左右" })).toBeChecked();
  });

  it("highlights changed words only", () => {
    const { container } = renderApp(<DiffView oldText={OLD} newText={NEW} />);
    const dels = [...container.querySelectorAll("mark[data-change='del']")].map((m) => m.textContent);
    const adds = [...container.querySelectorAll("mark[data-change='add']")].map((m) => m.textContent);
    expect(dels).toEqual(["bar"]);
    expect(adds).toEqual(["qux"]);
  });

  it("shows one line of context and folds the rest", () => {
    const old = Array.from({ length: 20 }, (_, i) => `row ${i + 1}`).join("\n");
    const next = old.replace("row 10", "row ten");
    const { container } = renderApp(<DiffView oldText={old} newText={next} />);
    const text = container.textContent ?? "";
    expect(text).toContain("row 9");
    expect(text).toContain("row 11");
    expect(text).not.toContain("row 8");
    expect(text).not.toContain("row 12");
    expect(screen.getAllByText(/行未改动/).length).toBeGreaterThan(0);
  });

  it("pads the shorter side with hatched empty rows", () => {
    const { container } = renderApp(<DiffView oldText={"a\nb\n"} newText={"a\nx\ny\nb\n"} />);
    expect(container.querySelectorAll("[data-empty='true']").length).toBe(2);
  });

  it("says so when the texts are identical", () => {
    renderApp(<DiffView oldText="same" newText="same" />);
    expect(screen.getByText("内容相同")).toBeInTheDocument();
  });

  it("unified layout lists removed and added lines with markers", async () => {
    localStorage.setItem("memdock.diffLayout", "unified");
    const { container } = renderApp(<DiffView oldText={OLD} newText={NEW} />);
    const root = container.querySelector("[data-diff-layout]") as HTMLElement;
    expect(within(root).getAllByText("-").length).toBe(1);
    expect(within(root).getAllByText("+").length).toBe(1);
  });


  it("does not word-diff unrelated lines in a replace block", () => {
    const { container } = renderApp(<DiffView oldText={"alpha beta gamma"} newText={"one two three four"} />);
    expect(container.querySelectorAll("mark").length).toBe(0);
    expect(container.textContent).toContain("alpha beta gamma");
    expect(container.textContent).toContain("one two three four");
  });

  it("falls back to a plain replaced block for huge, fully different texts", () => {
    const a = Array.from({ length: 3000 }, (_, i) => `old-${i} alpha`).join("\n");
    const b = Array.from({ length: 3000 }, (_, i) => `new-${i} omega`).join("\n");
    const { container } = renderApp(<DiffView oldText={a} newText={b} context={0} />);
    expect(container.querySelectorAll("mark").length).toBe(0);
    expect(container.textContent).toContain("old-2999 alpha");
    expect(container.textContent).toContain("new-2999 omega");
  });
});
