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
    expect(screen.getByRole("button", { name: /scripts/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /logo.png/ })).toBeInTheDocument();
  });

  it("files editor saves with expected hash", async () => {
    (readSkillFile as Mock).mockResolvedValue(dto());
    (writeSkillFile as Mock).mockResolvedValue(dto({ text: "# changed", hash: "h2" }));
    renderApp(<SkillFiles skillId="s1" readonly={false} />);
    const ed = await screen.findByDisplayValue("# hello");
    await userEvent.clear(ed);
    await userEvent.type(ed, "# changed");
    await userEvent.click(screen.getByRole("button", { name: "保存 (Ctrl+S)" }));
    expect(writeSkillFile).toHaveBeenCalledWith("s1", "SKILL.md", "# changed", "h1");
    await waitFor(() => expect(screen.getByRole("button", { name: "保存 (Ctrl+S)" })).toBeDisabled());
  });

  it("disk_changed opens the conflict dialog", async () => {
    (readSkillFile as Mock).mockResolvedValue(dto());
    (writeSkillFile as Mock).mockRejectedValue({ code: "disk_changed", message: "x", current_hash: "h9" });
    renderApp(<SkillFiles skillId="s1" readonly={false} />);
    const ed = await screen.findByDisplayValue("# hello");
    await userEvent.type(ed, "!");
    await userEvent.click(screen.getByRole("button", { name: "保存 (Ctrl+S)" }));
    expect(await screen.findByRole("dialog", { name: "重新读取文件" })).toBeInTheDocument();
  });

  it("binary file shows a notice and no editor", async () => {
    (readSkillFile as Mock).mockImplementation(async (_id: string, p: string) =>
      p === "logo.png" ? dto({ binary: true, text: "" }) : dto(),
    );
    renderApp(<SkillFiles skillId="s1" readonly={false} />);
    await screen.findByDisplayValue("# hello");
    await userEvent.click(screen.getByRole("button", { name: /logo.png/ }));
    expect(await screen.findByText("二进制文件，不能预览或编辑")).toBeInTheDocument();
    expect(screen.queryByLabelText("editor")).toBeNull();
  });

  it("too large file shows a notice", async () => {
    (readSkillFile as Mock).mockImplementation(async (_id: string, p: string) =>
      p === "logo.png" ? dto({ too_large: true, text: "" }) : dto(),
    );
    renderApp(<SkillFiles skillId="s1" readonly={false} />);
    await screen.findByDisplayValue("# hello");
    await userEvent.click(screen.getByRole("button", { name: /logo.png/ }));
    expect(await screen.findByText("文件太大，不能在这里打开")).toBeInTheDocument();
  });

  it("readonly skill opens files read-only without a save button", async () => {
    (readSkillFile as Mock).mockResolvedValue(dto());
    renderApp(<SkillFiles skillId="s1" readonly />);
    const ed = await screen.findByDisplayValue("# hello");
    expect(ed).toHaveAttribute("readonly");
    expect(screen.queryByRole("button", { name: "保存 (Ctrl+S)" })).toBeNull();
  });

  it("keeps edits in an inactive tab and reports them to the Skill page", async () => {
    (readSkillFile as Mock).mockResolvedValue(dto());
    const onDirtyChange = vi.fn();
    renderApp(<SkillFiles skillId="s1" readonly={false} onDirtyChange={onDirtyChange} />);
    await userEvent.type(await screen.findByDisplayValue("# hello"), "x");
    await userEvent.click(screen.getByRole("button", { name: "scripts" }));
    await userEvent.click(await screen.findByRole("button", { name: "run.py" }));
    expect(await screen.findByDisplayValue("# hello")).toBeInTheDocument();
    expect(onDirtyChange).toHaveBeenLastCalledWith(true);
    await userEvent.click(screen.getByRole("button", { name: "SKILL.md", pressed: false }));
    expect(await screen.findByDisplayValue("# hellox")).toBeInTheDocument();
  });

  it("external change reloads a clean editor", async () => {
    (readSkillFile as Mock).mockResolvedValue(dto());
    renderApp(<SkillFiles skillId="s1" readonly={false} />);
    await screen.findByDisplayValue("# hello");
    (readSkillFile as Mock).mockResolvedValue(dto({ text: "# remote", hash: "h5" }));
    await act(async () => emit({ paths: ["C:\\x\\SKILL.md"] }));
    expect(await screen.findByDisplayValue("# remote")).toBeInTheDocument();
  });

  it("refreshes an inactive clean tab when it is selected again", async () => {
    (readSkillFile as Mock).mockResolvedValue(dto());
    renderApp(<SkillFiles skillId="s1" readonly={false} />);
    await screen.findByDisplayValue("# hello");
    await userEvent.click(screen.getByRole("button", { name: "scripts" }));
    await userEvent.click(await screen.findByRole("button", { name: "run.py" }));
    await screen.findByDisplayValue("# hello");
    (readSkillFile as Mock).mockResolvedValue(dto({ text: "# externally updated", hash: "h2" }));
    await userEvent.click(screen.getByRole("button", { name: "SKILL.md", pressed: false }));
    expect(await screen.findByDisplayValue("# externally updated")).toBeInTheDocument();
  });

  it("opens a script from the shared file finder and saves through the Skill API", async () => {
    (readSkillFile as Mock).mockResolvedValue(dto({ text: "print(1)" }));
    (writeSkillFile as Mock).mockResolvedValue(dto({ text: "print(12)", hash: "h2" }));
    renderApp(<SkillFiles skillId="s1" readonly={false} />);
    await screen.findByDisplayValue("print(1)");
    await userEvent.click(screen.getByRole("button", { name: "找文件" }));
    await userEvent.type(screen.getByRole("textbox", { name: "找文件" }), "run.py");
    await userEvent.click(await screen.findByRole("button", { name: "scripts/run.py" }));
    const editor = await screen.findByDisplayValue("print(1)");
    await userEvent.clear(editor);
    await userEvent.type(editor, "print(12)");
    await userEvent.keyboard("{Control>}s{/Control}");
    await waitFor(() => expect(writeSkillFile).toHaveBeenCalledWith("s1", "scripts/run.py", "print(12)", "h1"));
  });
});
