import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi, type Mock } from "vitest";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn().mockResolvedValue({ language: "zh", theme: null, manual_projects: [] }),
  updateUiPrefs: vi.fn(),
  listMemories: vi.fn(),
  readMemoryFile: vi.fn(),
  writeMemoryFile: vi.fn(),
  readCodexDb: vi.fn(),
  addManualProject: vi.fn(),
  removeManualProject: vi.fn(),
  onMemoryChanged: vi.fn().mockResolvedValue(() => {}),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("@uiw/react-codemirror", () => ({
  default: ({ value, onChange }: { value: string; onChange?: (v: string) => void }) => (
    <textarea aria-label="editor" value={value} onChange={(e) => onChange?.(e.target.value)} />
  ),
}));

import { listMemories, readCodexDb, readMemoryFile } from "@/api/memory";
import { renderApp, sampleIndex } from "@/test/fixtures";
import { MemoryPage } from "./MemoryPage";

beforeEach(() => {
  localStorage.clear();
  (listMemories as Mock).mockResolvedValue(sampleIndex);
  (readCodexDb as Mock).mockResolvedValue({ status: "unavailable", reason: "x" });
  (readMemoryFile as Mock).mockImplementation(async (path: string) => ({
    text: `text of ${path}`,
    hash: "h",
    has_bom: false,
    crlf: false,
    tokens: 10,
    imports: [],
  }));
});

describe("MemoryPage", () => {
  it("opens the first global file and asks before discarding edits", async () => {
    renderApp(<MemoryPage />);
    const box = await screen.findByDisplayValue("text of /home/u/.claude/CLAUDE.md");
    await userEvent.type(box, "!");
    await userEvent.click(screen.getByRole("treeitem", { name: /记忆库/ }));
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog).toHaveTextContent("有未保存的修改");
    await userEvent.click(screen.getByRole("button", { name: "继续编辑" }));
    expect(screen.getByLabelText("editor")).toHaveValue("text of /home/u/.claude/CLAUDE.md!");
    await userEvent.click(screen.getByRole("treeitem", { name: /记忆库/ }));
    await userEvent.click(await screen.findByRole("button", { name: "放弃修改并切换" }));
    await waitFor(() => expect(screen.queryByLabelText("editor")).not.toBeInTheDocument());
  });

  it("tree width is clamped and persisted from the keyboard", async () => {
    renderApp(<MemoryPage />);
    const sep = await screen.findByRole("separator");
    expect(sep).toHaveAttribute("aria-valuenow", "280");
    sep.focus();
    await userEvent.keyboard("{ArrowRight}");
    expect(sep).toHaveAttribute("aria-valuenow", "296");
    expect(localStorage.getItem("memdock.treeWidth")).toBe("296");
    for (let i = 0; i < 20; i++) await userEvent.keyboard("{ArrowRight}");
    expect(sep).toHaveAttribute("aria-valuenow", "420");
  });
});
