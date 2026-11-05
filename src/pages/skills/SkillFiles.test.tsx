import { useEffect } from "react";
import { act, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi, type Mock } from "vitest";
import type { SkillFileDto, SkillsChangedPayload } from "@/api/types";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn().mockResolvedValue({ language: "zh", theme: null, accent: null, manual_projects: [] }),
  updateUiPrefs: vi.fn(),
}));
vi.mock("@/api/skills", () => ({
  listSkillFiles: vi.fn(),
  readSkillFile: vi.fn(),
  writeSkillFile: vi.fn(),
  onSkillsChanged: vi.fn(),
}));
vi.mock("@uiw/react-codemirror", () => ({
  default: ({
    value,
    onChange,
    editable,
    readOnly,
  }: {
    value: string;
    onChange?: (v: string) => void;
    editable?: boolean;
    readOnly?: boolean;
  }) => {
    useEffect(() => {}, []);
    return (
      <textarea
        aria-label="editor"
        value={value}
        readOnly={readOnly || editable === false}
        onChange={(e) => onChange?.(e.target.value)}
      />
    );
  },
  EditorView: { theme: () => [], lineWrapping: [] },
}));

import { listSkillFiles, onSkillsChanged, readSkillFile, writeSkillFile } from "@/api/skills";
import { renderApp } from "@/test/fixtures";
import { SkillFiles } from "./SkillFiles";

const dto = (over: Partial<SkillFileDto> = {}): SkillFileDto => ({
  text: "# hello",
  hash: "h1",
  has_bom: false,
  crlf: false,
  tokens: 10,
  imports: [],
  binary: false,
  too_large: false,
  ...over,
});

const files = {
  files: [
    { rel_path: "scripts/run.py", size_bytes: 20 },
    { rel_path: "SKILL.md", size_bytes: 50 },
    { rel_path: "logo.png", size_bytes: 900 },
  ],
  truncated: false,
};

let emit: (p: SkillsChangedPayload) => void;

beforeEach(() => {
  vi.clearAllMocks();
  (listSkillFiles as Mock).mockResolvedValue(files);
  (onSkillsChanged as Mock).mockImplementation(async (cb) => {
    emit = cb;
    return () => {};
  });
});

describe("SkillFiles", () => {
  it("opens SKILL.md by default and lists the tree", async () => {
    (readSkillFile as Mock).mockResolvedValue(dto());
    renderApp(<SkillFiles skillId="s1" readonly={false} />);
    expect(await screen.findByDisplayValue("# hello")).toBeInTheDocument();
    expect(readSkillFile).toHaveBeenCalledWith("s1", "SKILL.md");
    expect(screen.getByRole("treeitem", { name: /scripts/ })).toBeInTheDocument();
    expect(screen.getByRole("treeitem", { name: /logo.png/ })).toBeInTheDocument();
  });

  it("files editor saves with expected hash", async () => {
    (readSkillFile as Mock).mockResolvedValue(dto());
    (writeSkillFile as Mock).mockResolvedValue(dto({ text: "# changed", hash: "h2" }));
    renderApp(<SkillFiles skillId="s1" readonly={false} />);
    const ed = await screen.findByDisplayValue("# hello");
    await userEvent.clear(ed);
    await userEvent.type(ed, "# changed");
    await userEvent.click(screen.getByRole("button", { name: "保存" }));
    expect(writeSkillFile).toHaveBeenCalledWith("s1", "SKILL.md", "# changed", "h1");
    await waitFor(() => expect(screen.getByRole("button", { name: "保存" })).toBeDisabled());
  });

  it("disk_changed opens the conflict dialog", async () => {
    (readSkillFile as Mock).mockResolvedValue(dto());
    (writeSkillFile as Mock).mockRejectedValue({ code: "disk_changed", message: "x", current_hash: "h9" });
    renderApp(<SkillFiles skillId="s1" readonly={false} />);
    const ed = await screen.findByDisplayValue("# hello");
    await userEvent.type(ed, "!");
    await userEvent.click(screen.getByRole("button", { name: "保存" }));
    expect(await screen.findByText("文件在磁盘上已被修改，重新加载会丢弃编辑器里未保存的内容。")).toBeInTheDocument();
  });

  it("binary file shows a notice and no editor", async () => {
    (readSkillFile as Mock).mockImplementation(async (_id: string, p: string) =>
      p === "logo.png" ? dto({ binary: true, text: "" }) : dto(),
    );
    renderApp(<SkillFiles skillId="s1" readonly={false} />);
    await screen.findByDisplayValue("# hello");
    await userEvent.click(screen.getByRole("treeitem", { name: /logo.png/ }));
    expect(await screen.findByText("二进制文件，不能预览或编辑")).toBeInTheDocument();
    expect(screen.queryByLabelText("editor")).toBeNull();
  });

  it("too large file shows a notice", async () => {
    (readSkillFile as Mock).mockImplementation(async (_id: string, p: string) =>
      p === "logo.png" ? dto({ too_large: true, text: "" }) : dto(),
    );
    renderApp(<SkillFiles skillId="s1" readonly={false} />);
    await screen.findByDisplayValue("# hello");
    await userEvent.click(screen.getByRole("treeitem", { name: /logo.png/ }));
    expect(await screen.findByText("文件太大，不能在这里打开")).toBeInTheDocument();
  });

  it("readonly skill opens files read-only without a save button", async () => {
    (readSkillFile as Mock).mockResolvedValue(dto());
    renderApp(<SkillFiles skillId="s1" readonly />);
    const ed = await screen.findByDisplayValue("# hello");
    expect(ed).toHaveAttribute("readonly");
    expect(screen.queryByRole("button", { name: "保存" })).toBeNull();
  });

  it("switching files with unsaved edits asks first", async () => {
    (readSkillFile as Mock).mockResolvedValue(dto());
    renderApp(<SkillFiles skillId="s1" readonly={false} />);
    const ed = await screen.findByDisplayValue("# hello");
    await userEvent.type(ed, "x");
    await userEvent.click(screen.getByRole("treeitem", { name: /logo.png/ }));
    expect(await screen.findByText("放弃未保存的修改？")).toBeInTheDocument();
    expect(readSkillFile).toHaveBeenCalledTimes(1);
  });

  it("external change reloads a clean editor", async () => {
    (readSkillFile as Mock).mockResolvedValue(dto());
    renderApp(<SkillFiles skillId="s1" readonly={false} />);
    await screen.findByDisplayValue("# hello");
    (readSkillFile as Mock).mockResolvedValue(dto({ text: "# remote", hash: "h5" }));
    await act(async () => emit({ paths: ["C:\\x\\SKILL.md"] }));
    expect(await screen.findByDisplayValue("# remote")).toBeInTheDocument();
  });
});
