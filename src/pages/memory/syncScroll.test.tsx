import { useEffect } from "react";
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi, type Mock } from "vitest";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn().mockResolvedValue({ language: "zh", theme: null, accent: null, manual_projects: [] }),
  updateUiPrefs: vi.fn(),
  readMemoryFile: vi.fn(),
  writeMemoryFile: vi.fn(),
  readCodexDb: vi.fn(),
  onMemoryChanged: vi.fn(),
  listSnapshots: vi.fn(),
  readSnapshot: vi.fn(),
}));
vi.mock("@uiw/react-codemirror", () => ({
  default: ({ value, onChange, onCreateEditor }: { value: string; onChange?: (v: string) => void; onCreateEditor?: (v: unknown) => void }) => {
    useEffect(() => onCreateEditor?.({ dom: { isConnected: true } }), [onCreateEditor]);
    return <textarea aria-label="editor" value={value} onChange={(e) => onChange?.(e.target.value)} />;
  },
  EditorView: { theme: () => [], lineWrapping: [] },
}));

import { onMemoryChanged, readMemoryFile } from "@/api/memory";
import { renderApp, globalClaude, sampleIndex } from "@/test/fixtures";
import { MemoryEditor, Preview } from "./MemoryEditor";

beforeEach(() => {
  localStorage.clear();
  (onMemoryChanged as Mock).mockResolvedValue(() => {});
  (readMemoryFile as Mock).mockResolvedValue({
    text: "# T\n\nbody",
    hash: "h",
    has_bom: false,
    crlf: false,
    tokens: 1,
    imports: [],
  });
});

describe("Preview data-line", () => {
  it("stamps block elements with their 1-based source line", () => {
    const md = "# Title\n\npara one\n\n- a\n- b\n\n> quote\n\n```\ncode\n```\n\n---\n\n| x | y |\n|---|---|\n| 1 | 2 |\n";
    const { container } = renderApp(<Preview text={md} />);
    const line = (sel: string) => container.querySelector(sel)?.getAttribute("data-line");
    expect(line("h1")).toBe("1");
    expect(line("p")).toBe("3");
    expect(container.querySelectorAll("li")[1].getAttribute("data-line")).toBe("6");
    expect(line("blockquote")).toBe("8");
    expect(line("pre")).toBe("10");
    expect(line("hr")).toBe("14");
    expect(line("table")).toBe("16");
  });
});

describe("sync scroll toggle", () => {
  const setup = () => renderApp(<MemoryEditor file={globalClaude} index={sampleIndex} />);

  it("is on by default in split mode and persists when toggled", async () => {
    setup();
    const btn = await screen.findByRole("button", { name: "同步滚动" });
    expect(btn).toHaveAttribute("aria-pressed", "true");
    await userEvent.click(btn);
    expect(btn).toHaveAttribute("aria-pressed", "false");
    expect(localStorage.getItem("memdock.syncScroll")).toBe("0");
  });

  it("restores the stored off state", async () => {
    localStorage.setItem("memdock.syncScroll", "0");
    setup();
    expect(await screen.findByRole("button", { name: "同步滚动" })).toHaveAttribute("aria-pressed", "false");
  });

  it("only shows in split mode", async () => {
    setup();
    await screen.findByRole("button", { name: "同步滚动" });
    await userEvent.click(screen.getByRole("radio", { name: "编辑" }));
    expect(screen.queryByRole("button", { name: "同步滚动" })).not.toBeInTheDocument();
  });
});

describe("editor top bar at narrow widths", () => {
  it("lets the path shrink, truncates the file name, and compacts the right side", async () => {
    renderApp(<MemoryEditor file={globalClaude} index={sampleIndex} />);
    const name = await screen.findByText("CLAUDE.md");
    const pathBox = name.parentElement as HTMLElement;
    expect(pathBox.className).toMatch(/min-w-0|min-w-\[6rem\]/);
    expect(name.className).toMatch(/\btruncate\b/);
    expect(name.className).not.toMatch(/shrink-0/);
    expect(pathBox).toHaveAttribute("title", globalClaude.path);
    const bar = pathBox.parentElement as HTMLElement;
    expect(bar.className).toMatch(/@container/);
    expect(screen.getByText(/tokens/).className).toMatch(/@max-\[[\d.]+rem\]:hidden/);
    for (const label of ["历史版本", "保存"]) {
      const span = screen.getByText(label);
      expect(span.className).toMatch(/@max-\[[\d.]+rem\]:sr-only/);
    }
    expect(screen.getByRole("button", { name: "保存" })).toHaveAttribute("title", "保存");
  });
});
