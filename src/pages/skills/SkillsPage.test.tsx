import { act, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi, type Mock } from "vitest";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn().mockResolvedValue({ language: "zh", theme: null, accent: null, manual_projects: [] }),
  updateUiPrefs: vi.fn(),
  listMemories: vi.fn().mockResolvedValue({ global: [], projects: [] }),
}));
vi.mock("@/api/skills", () => ({
  listSkills: vi.fn(),
  listDeletedSkillBackups: vi.fn().mockResolvedValue([]),
  restoreDeletedSkill: vi.fn(),
  revealBackupFolder: vi.fn(),
  gitAvailable: vi.fn().mockResolvedValue(true),
  checkSkillUpdates: vi.fn(),
  onSkillsChanged: vi.fn().mockResolvedValue(() => {}),
  listSkillFiles: vi.fn().mockResolvedValue({ files: [], truncated: false }),
  readSkillFile: vi.fn(),
  writeSkillFile: vi.fn(),
  registerSkillSource: vi.fn(),
  inspectGitSource: vi.fn(),
  installSkill: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

vi.mock("@uiw/react-codemirror", () => ({
  default: ({ value, onChange }: { value: string; onChange?: (v: string) => void }) => (
    <textarea aria-label="editor" value={value} onChange={(e) => onChange?.(e.target.value)} />
  ),
  EditorView: { theme: () => [], lineWrapping: [] },
}));

import { checkSkillUpdates, listSkills } from "@/api/skills";
import { renderApp } from "@/test/fixtures";
import { SkillsPage } from "./SkillsPage";
import { mkSkill, mkUpdate } from "./testUtils";

const a = mkSkill({ name: "alpha" });
const b = mkSkill({ name: "beta" });

beforeEach(() => vi.clearAllMocks());

describe("SkillsPage", () => {
  it("shows the empty state with an add button", async () => {
    (listSkills as Mock).mockResolvedValue([]);
    renderApp(<SkillsPage />);
    expect(await screen.findByText("还没有 skill")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "添加 skill" }));
    expect(await screen.findByRole("dialog", { name: "添加 skill" })).toBeInTheDocument();
  });

  it("does not check updates until the button is clicked, then merges results into rows", async () => {
    (listSkills as Mock).mockResolvedValue([a, b]);
    (checkSkillUpdates as Mock).mockResolvedValue([mkUpdate({ skill_id: b.id, has_update: true })]);
    renderApp(<SkillsPage />);
    await screen.findByRole("listbox");
    expect(checkSkillUpdates).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "检查更新" }));
    const row = within(screen.getByRole("listbox")).getAllByRole("option").find((o) => within(o).queryByText("beta"))!;
    expect(await within(row).findByText("有更新")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "检查更新（1）" })).toBeInTheDocument();
  });

  it("opens the first skill and switches on click", async () => {
    (listSkills as Mock).mockResolvedValue([a, b]);
    renderApp(<SkillsPage />);
    expect(await screen.findByRole("heading", { name: "alpha" })).toBeInTheDocument();
    await userEvent.click(within(screen.getByRole("listbox")).getByText("beta"));
    expect(await screen.findByRole("heading", { name: "beta" })).toBeInTheDocument();
  });

  it("groups installs by name and source, not by name alone", async () => {
    const src = { url: "https://github.com/o/notes", git_ref: null, commit: "abc1234", skill_path: "", origin: "registry" as const };
    const local = mkSkill({ name: "notes", dir: "/local/notes", id: "l" });
    const gh1 = mkSkill({ name: "notes", dir: "/gh/notes", id: "g1", source: src });
    const gh2 = mkSkill({ name: "notes", dir: "/gh2/notes", id: "g2", tool: "codex", source: { ...src, url: "https://github.com/o/notes.git" } });
    (listSkills as Mock).mockResolvedValue([local, gh1, gh2]);
    renderApp(<SkillsPage />);
    await screen.findByRole("heading", { name: "notes" });
    // The first row opened is a GitHub one: both GitHub installs, but not the local copy.
    expect(screen.getByText("/gh/notes")).toBeInTheDocument();
    expect(screen.getByText("/gh2/notes")).toBeInTheDocument();
    expect(screen.queryByText("/local/notes")).toBeNull();
    const rows = within(screen.getByRole("listbox")).getAllByRole("option");
    await userEvent.click(rows.find((r) => within(r).queryByText("本地"))!);
    expect(await screen.findByText("/local/notes")).toBeInTheDocument();
    expect(screen.queryByText("/gh/notes")).toBeNull();
  });

  it("update button is disabled when git is missing", async () => {
    const { gitAvailable } = await import("@/api/skills");
    (gitAvailable as Mock).mockResolvedValueOnce(false);
    const src = { url: "https://github.com/o/r", git_ref: null, commit: "abc1234", skill_path: "", origin: "registry" as const };
    (listSkills as Mock).mockResolvedValue([mkSkill({ name: "alpha", source: src })]);
    renderApp(<SkillsPage />);
    await screen.findByRole("heading", { name: "alpha" });
    await vi.waitFor(() => expect(screen.getByRole("button", { name: "更新" })).toBeDisabled());
  });

  it("keeps the detail with a warning when the skill vanishes while dirty", async () => {
    const { onSkillsChanged, readSkillFile, listSkillFiles } = await import("@/api/skills");
    const cbs: ((p: { paths: string[] }) => void)[] = [];
    const emit = () => cbs.forEach((cb) => cb({ paths: [] }));
    (onSkillsChanged as Mock).mockImplementation(async (cb) => {
      cbs.push(cb);
      return () => {};
    });
    (listSkillFiles as Mock).mockResolvedValue({ files: [{ rel_path: "SKILL.md", size_bytes: 1 }], truncated: false });
    (readSkillFile as Mock).mockResolvedValue({ text: "hi", hash: "h", has_bom: false, crlf: false, tokens: 1, imports: [], binary: false, too_large: false });
    (listSkills as Mock).mockResolvedValue([a, b]);
    const onDirty = vi.fn();
    renderApp(<SkillsPage onDirtyChange={onDirty} />);
    await screen.findByRole("heading", { name: "alpha" });
    await userEvent.click(screen.getByRole("radio", { name: "文件" }));
    const ed = await screen.findByLabelText("editor");
    await userEvent.type(ed, "x");
    await vi.waitFor(() => expect(onDirty).toHaveBeenLastCalledWith(true));
    (listSkills as Mock).mockResolvedValue([b]);
    await act(async () => emit());
    expect(await screen.findByText(/已被外部删除/)).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "alpha" })).toBeInTheDocument();
  });

  it("lists deleted skills at the bottom, opens one and restores it", async () => {
    const { listDeletedSkillBackups, restoreDeletedSkill, revealBackupFolder } = await import("@/api/skills");
    const gone = {
      bucket: "gone-1a2b3c4d",
      name: "gone",
      tool: "claude" as const,
      scope: { type: "global" as const },
      dir: a.dir.replace("alpha", "gone"),
      backups: [{ id: "1700000900000", created_at: 1_700_000_900_000, size_bytes: 2048 }],
    };
    (listDeletedSkillBackups as Mock).mockResolvedValue([gone]);
    (revealBackupFolder as Mock).mockResolvedValue("C:\\data\\skill-backups\\gone-1a2b3c4d");
    (restoreDeletedSkill as Mock).mockResolvedValue(gone.dir);
    const back = mkSkill({ name: "gone", dir: gone.dir, id: gone.dir.replace(/\\/g, "/").toLowerCase() });
    (listSkills as Mock).mockResolvedValue([a, b]);
    renderApp(<SkillsPage />);
    await screen.findByRole("heading", { name: "alpha" });
    await userEvent.click(await screen.findByRole("button", { name: "已删除的 skill（1）" }));
    await userEvent.click(within(screen.getByRole("list", { name: "已删除的 skill 列表" })).getByText("gone"));
    expect(await screen.findByText("这个 skill 已被删除，备份还在，可以恢复到原位置。")).toBeInTheDocument();
    expect(await screen.findByText("C:\\data\\skill-backups\\gone-1a2b3c4d")).toBeInTheDocument();
    // Restoring brings it back: the list is reloaded and the restored skill opens.
    (listDeletedSkillBackups as Mock).mockResolvedValue([]);
    (listSkills as Mock).mockResolvedValue([a, b, back]);
    await userEvent.click(screen.getByRole("button", { name: "恢复到原位置" }));
    const dialog = await screen.findByRole("alertdialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "恢复" }));
    await vi.waitFor(() => expect(restoreDeletedSkill).toHaveBeenCalledWith("gone-1a2b3c4d", "1700000900000"));
    expect(await screen.findByText("已恢复 gone 到原位置")).toBeInTheDocument();
    expect(await screen.findByRole("heading", { name: "gone" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /已删除的 skill（/ })).toBeNull();
  });

  it("still shows the deleted group when the last skill was deleted", async () => {
    const { listDeletedSkillBackups } = await import("@/api/skills");
    (listDeletedSkillBackups as Mock).mockResolvedValue([
      { bucket: "gone-1a2b3c4d", name: "gone", tool: "claude", scope: { type: "global" }, dir: "C:\\x\\gone", backups: [] },
    ]);
    (listSkills as Mock).mockResolvedValue([]);
    renderApp(<SkillsPage />);
    expect(await screen.findByRole("button", { name: "已删除的 skill（1）" })).toBeInTheDocument();
    expect(screen.queryByText("还没有 skill")).toBeNull();
  });
});
