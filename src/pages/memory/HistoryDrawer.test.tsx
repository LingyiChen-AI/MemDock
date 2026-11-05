import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi, type Mock } from "vitest";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn().mockResolvedValue({ language: "zh", theme: null, accent: null, manual_projects: [] }),
  updateUiPrefs: vi.fn(),
  listSnapshots: vi.fn(),
  readSnapshot: vi.fn(),
  writeMemoryFile: vi.fn(),
}));

import { listSnapshots, readSnapshot, writeMemoryFile } from "@/api/memory";
import { renderApp } from "@/test/fixtures";
import { HistoryDrawer } from "./HistoryDrawer";

const T1 = new Date(2026, 10, 5, 14, 2).getTime();
const T2 = new Date(2026, 10, 4, 9, 30).getTime();
const snaps = [
  { id: "s2", created_at: T1, size_bytes: 2048 },
  { id: "s1", created_at: T2, size_bytes: 1024 },
];

beforeEach(() => {
  vi.clearAllMocks();
  (listSnapshots as Mock).mockResolvedValue(snaps);
  (readSnapshot as Mock).mockImplementation(async (_p: string, id: string) => `old text ${id}\ncommon`);
  (writeMemoryFile as Mock).mockImplementation(async (_p: string, text: string) => ({
    text,
    hash: "h2",
    has_bom: false,
    crlf: false,
    tokens: 1,
    imports: [],
  }));
});

function setup(over: Partial<React.ComponentProps<typeof HistoryDrawer>> = {}) {
  const onRestored = vi.fn();
  renderApp(
    <HistoryDrawer
      open
      onOpenChange={() => {}}
      path="/a/CLAUDE.md"
      currentText={"current text\ncommon"}
      currentHash="h1"
      onRestored={onRestored}
      {...over}
    />,
  );
  return { onRestored };
}

describe("HistoryDrawer", () => {
  it("history lists snapshots newest first", async () => {
    setup();
    const items = await screen.findAllByRole("option");
    expect(items).toHaveLength(2);
    expect(items[0]).toHaveTextContent("11-05 14:02");
    expect(items[0]).toHaveTextContent("2.0 KB");
    expect(items[1]).toHaveTextContent("11-04 09:30");
    expect(items[1]).toHaveTextContent("1.0 KB");
  });

  it("shows a diff of the selected snapshot against the current text", async () => {
    setup();
    const items = await screen.findAllByRole("option");
    await userEvent.click(items[1]);
    await waitFor(() => expect(readSnapshot).toHaveBeenCalledWith("/a/CLAUDE.md", "s1"));
    await waitFor(() => expect(document.querySelector("mark[data-change=add]")).toHaveTextContent("old"));
  });

  it("restore writes snapshot content with current hash", async () => {
    const { onRestored } = setup();
    await screen.findAllByRole("option");
    await userEvent.click(await screen.findByRole("button", { name: "恢复此版本" }));
    await waitFor(() => expect(writeMemoryFile).toHaveBeenCalledWith("/a/CLAUDE.md", "old text s2\ncommon", "h1"));
    expect(onRestored).toHaveBeenCalledWith("old text s2\ncommon", expect.objectContaining({ hash: "h2" }));
  });

  it("asks for confirmation when the editor has unsaved changes", async () => {
    setup({ dirty: true });
    await screen.findAllByRole("option");
    await userEvent.click(await screen.findByRole("button", { name: "恢复此版本" }));
    expect(writeMemoryFile).not.toHaveBeenCalled();
    const dialog = await screen.findByRole("alertdialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "放弃修改并恢复" }));
    await waitFor(() => expect(writeMemoryFile).toHaveBeenCalledTimes(1));
  });

  it("shows an empty state without snapshots", async () => {
    (listSnapshots as Mock).mockResolvedValue([]);
    setup();
    expect(await screen.findByText("还没有历史版本")).toBeInTheDocument();
  });

  it("a snapshot read failure only affects the diff area", async () => {
    (readSnapshot as Mock).mockImplementation(async (_p: string, id: string) => {
      if (id === "s2") throw { code: "io", message: "boom" };
      return "other\ncommon";
    });
    setup();
    const items = await screen.findAllByRole("option");
    expect(await screen.findByRole("alert")).toHaveTextContent("boom");
    expect(screen.getAllByRole("option")).toHaveLength(2);
    await userEvent.click(items[1]);
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
    expect(document.querySelector("[data-diff-layout]")).not.toBeNull();
  });
});
