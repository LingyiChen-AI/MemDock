import { useEffect } from "react";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi, type Mock } from "vitest";
import type { FileContentDto, MemoryChangedPayload } from "@/api/types";

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
const fakeView = vi.hoisted(() => ({
  dom: { isConnected: true },
  state: { doc: { lines: 10, line: (n: number) => ({ from: n * 100 }) } },
  dispatch: vi.fn(),
  focus: vi.fn(),
}));
// CodeMirror needs real layout; swap it for a textarea in jsdom.
vi.mock("@uiw/react-codemirror", () => ({
  default: ({ value, onChange, onCreateEditor }: { value: string; onChange?: (v: string) => void; onCreateEditor?: (v: unknown) => void }) => {
    useEffect(() => onCreateEditor?.(fakeView), [onCreateEditor]);
    return <textarea aria-label="editor" value={value} onChange={(e) => onChange?.(e.target.value)} />;
  },
  EditorView: { theme: () => [], lineWrapping: [] },
}));

import { listSnapshots, onMemoryChanged, readCodexDb, readMemoryFile, readSnapshot, writeMemoryFile } from "@/api/memory";
import { renderApp, globalClaude, globalAgents, globalDb, sampleIndex } from "@/test/fixtures";
import { MemoryEditor, Preview } from "./MemoryEditor";

const dto = (over: Partial<FileContentDto> = {}): FileContentDto => ({
  text: "hello",
  hash: "h1",
  has_bom: false,
  crlf: false,
  tokens: 4100,
  imports: [],
  ...over,
});

let emit: (p: MemoryChangedPayload) => void;

beforeEach(() => {
  vi.clearAllMocks();
  (onMemoryChanged as Mock).mockImplementation(async (cb) => {
    emit = cb;
    return () => {};
  });
});

function setup(file = globalClaude) {
  return renderApp(<MemoryEditor file={file} index={sampleIndex} />);
}

describe("MemoryEditor", () => {
  it("shows path and token pill after loading", async () => {
    (readMemoryFile as Mock).mockResolvedValue(dto());
    setup();
    expect(await screen.findByDisplayValue("hello")).toBeInTheDocument();
    expect(screen.getByText("约 4.1k tokens")).toBeInTheDocument();
    expect(screen.getByText("CLAUDE.md")).toBeInTheDocument();
    expect(screen.getByTitle(globalClaude.path)).toBeInTheDocument();
  });

  it("editor saves with expected hash", async () => {
    (readMemoryFile as Mock).mockResolvedValue(dto());
    (writeMemoryFile as Mock).mockResolvedValue(dto({ text: "hello world", hash: "h2" }));
    setup();
    const box = await screen.findByLabelText("editor");
    await userEvent.type(box, " world");
    expect(screen.getByTitle("未保存")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() =>
      expect(writeMemoryFile).toHaveBeenCalledWith(globalClaude.path, "hello world", "h1"),
    );
    await waitFor(() => expect(screen.queryByTitle("未保存")).not.toBeInTheDocument());
    // second save uses the new hash
    await userEvent.type(box, "!");
    (writeMemoryFile as Mock).mockResolvedValue(dto({ text: "hello world!", hash: "h3" }));
    await userEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() =>
      expect(writeMemoryFile).toHaveBeenLastCalledWith(globalClaude.path, "hello world!", "h2"),
    );
  });

  it("disk_changed shows conflict dialog and keeps user text", async () => {
    (readMemoryFile as Mock).mockResolvedValue(dto());
    (writeMemoryFile as Mock).mockRejectedValue({
      code: "disk_changed",
      message: "changed",
      current_hash: "zzz",
    });
    setup();
    const box = await screen.findByLabelText("editor");
    await userEvent.type(box, "X");
    await userEvent.click(screen.getByRole("button", { name: "保存" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog).toHaveTextContent("文件已在外部修改");
    await userEvent.click(screen.getByRole("button", { name: "取消" }));
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    expect(screen.getByLabelText("editor")).toHaveValue("helloX");

    // reload discards
    (readMemoryFile as Mock).mockResolvedValue(dto({ text: "disk text", hash: "zzz" }));
    await userEvent.click(screen.getByRole("button", { name: "保存" }));
    await userEvent.click(await screen.findByRole("button", { name: "重新加载（丢弃我的修改）" }));
    await waitFor(() => expect(screen.getByLabelText("editor")).toHaveValue("disk text"));
  });

  it("external change reloads when clean", async () => {
    (readMemoryFile as Mock).mockResolvedValue(dto());
    setup();
    await screen.findByDisplayValue("hello");
    (readMemoryFile as Mock).mockResolvedValue(dto({ text: "from disk", hash: "h9" }));
    await act(async () => emit({ paths: [globalClaude.path] }));
    await waitFor(() => expect(screen.getByLabelText("editor")).toHaveValue("from disk"));
  });

  it("external change with unsaved edits shows banner and keeps text", async () => {
    (readMemoryFile as Mock).mockResolvedValue(dto());
    setup();
    const box = await screen.findByLabelText("editor");
    await userEvent.type(box, "!");
    (readMemoryFile as Mock).mockResolvedValue(dto({ text: "from disk", hash: "h9" }));
    await act(async () => emit({ paths: [globalClaude.path] }));
    expect(await screen.findByText("文件已在外部修改")).toBeInTheDocument();
    expect(screen.getByLabelText("editor")).toHaveValue("hello!");
  });

  it("unrelated change is ignored", async () => {
    (readMemoryFile as Mock).mockResolvedValue(dto());
    setup();
    await screen.findByDisplayValue("hello");
    (readMemoryFile as Mock).mockClear();
    await act(async () => emit({ paths: ["/other/file.md"] }));
    expect(readMemoryFile).not.toHaveBeenCalled();
  });

  it("missing global file opens empty and creates with empty hash", async () => {
    (writeMemoryFile as Mock).mockResolvedValue(dto({ text: "new", hash: "n1" }));
    setup(globalAgents);
    expect(await screen.findByText("未创建，保存后创建")).toBeInTheDocument();
    expect(readMemoryFile).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "保存" })).toBeDisabled();
    await userEvent.type(screen.getByLabelText("editor"), "new");
    await userEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() => expect(writeMemoryFile).toHaveBeenCalledWith(globalAgents.path, "new", ""));
  });

  it("imports: existing indexed ones open, missing ones are flagged", async () => {
    (readMemoryFile as Mock).mockResolvedValue(
      dto({
        imports: [
          { raw: "./a.md", resolved: "/home/u/./.claude/CLAUDE.md", exists: true, line: 1 },
          { raw: "b.md", resolved: "/nowhere/b.md", exists: false, line: 2 },
          { raw: "c.md", resolved: "/outside/c.md", exists: true, line: 3 },
        ],
      }),
    );
    const onOpenImport = vi.fn();
    renderApp(<MemoryEditor file={globalClaude} index={sampleIndex} onOpenImport={onOpenImport} />);
    expect(await screen.findByText("引用 3 个文件：")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "./a.md" }));
    expect(onOpenImport).toHaveBeenCalledWith(globalClaude);
    expect(screen.getByText("b.md")).toHaveAttribute("title", "文件不存在");
    expect(screen.queryByRole("button", { name: "c.md" })).not.toBeInTheDocument();
    expect(screen.getByText("c.md")).toBeInTheDocument();
  });

  it("codex db unavailable shows reason", async () => {
    (readCodexDb as Mock).mockResolvedValue({ status: "unavailable", reason: "db locked" });
    setup(globalDb);
    expect(await screen.findByText("暂不可用：db locked")).toBeInTheDocument();
  });

  it("codex db lists entries read-only", async () => {
    (readCodexDb as Mock).mockResolvedValue({
      status: "ok",
      entries: [
        { thread_id: "t-1", raw_memory: "remember me", rollout_summary: "sum", generated_at: 1_700_000_000 },
      ],
    });
    setup(globalDb);
    expect(await screen.findByText("remember me")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "保存" })).not.toBeInTheDocument();
  });

  it("reveal moves the cursor to the requested line", async () => {
    (readMemoryFile as Mock).mockResolvedValue(dto());
    renderApp(<MemoryEditor file={globalClaude} index={sampleIndex} reveal={{ line: 3, nonce: 1 }} />);
    await screen.findByLabelText("editor");
    await waitFor(() =>
      expect(fakeView.dispatch).toHaveBeenCalledWith(expect.objectContaining({ selection: { anchor: 300 } })),
    );
    expect(fakeView.focus).toHaveBeenCalled();
  });

  it("history button opens the drawer and restoring reloads the editor", async () => {
    (readMemoryFile as Mock).mockResolvedValue(dto());
    (listSnapshots as Mock).mockResolvedValue([{ id: "s1", created_at: 1_700_000_000_000, size_bytes: 10 }]);
    (readSnapshot as Mock).mockResolvedValue("older");
    (writeMemoryFile as Mock).mockResolvedValue(dto({ text: "older", hash: "h9" }));
    setup();
    await screen.findByLabelText("editor");
    await userEvent.click(screen.getByRole("button", { name: "历史版本" }));
    await userEvent.click(await screen.findByRole("button", { name: "恢复此版本" }));
    await waitFor(() => expect(writeMemoryFile).toHaveBeenCalledWith(globalClaude.path, "older", "h1"));
    await waitFor(() => expect(screen.getByLabelText("editor")).toHaveValue("older"));
    expect(screen.queryByTitle("未保存")).not.toBeInTheDocument();
  });
});

describe("Preview", () => {
  it("never renders an <img> for remote images (zero network egress)", () => {
    const { container } = render(
      <Preview text={"![logo](https://example.com/x.png) and ![](http://a.b/c.gif) ![d](data:image/png;base64,AAAA)"} />,
    );
    expect(container.querySelector("img")).toBeNull();
    expect(screen.getByText("logo")).toBeInTheDocument();
    expect(screen.getByText("http://a.b/c.gif")).toBeInTheDocument();
  });
});
