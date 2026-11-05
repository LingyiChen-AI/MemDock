import { screen, waitFor, within } from "@testing-library/react";
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
  updateSkill: vi.fn(),
  deleteSkill: vi.fn(),
  syncSkill: vi.fn(),
  installSkill: vi.fn(),
  inspectGitSource: vi.fn(),
  listSkillBackups: vi.fn().mockResolvedValue([]),
  rollbackSkill: vi.fn(),
  readSkillBackupFile: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

import { checkSkillUpdates, deleteSkill, listSkillBackups, listSkills, readSkillFile, updateSkill } from "@/api/skills";
import { renderApp } from "@/test/fixtures";
import { SkillsPage } from "./SkillsPage";
import { mkSkill, mkUpdate } from "./testUtils";

const src = { url: "https://github.com/o/r", git_ref: null, commit: "aaaaaaa1111", skill_path: "", origin: "registry" as const };
const alpha = mkSkill({ name: "alpha", source: src });
const link = mkSkill({
  name: "linked",
  is_symlink: true,
  symlink_target: "C:\\repo\\linked",
});

beforeEach(() => {
  vi.clearAllMocks();
  (listSkills as Mock).mockResolvedValue([alpha, link]);
  (readSkillFile as Mock).mockResolvedValue({ text: "", hash: "h", has_bom: false, crlf: false, tokens: 0, imports: [], binary: false, too_large: false });
});

async function open() {
  renderApp(<SkillsPage />);
  await screen.findByRole("heading", { name: "alpha" });
}

describe("skill actions", () => {
  it("update with local changes asks confirmation", async () => {
    (updateSkill as Mock)
      .mockRejectedValueOnce({ code: "local_changes", message: "local changes" })
      .mockResolvedValueOnce({ ...alpha, source: { ...src, commit: "abc1234567890" } });
    await open();
    await userEvent.click(screen.getByRole("button", { name: "更新" }));
    expect(await screen.findByText("本地有修改，更新会覆盖（已自动备份），继续？")).toBeInTheDocument();
    expect(updateSkill).toHaveBeenCalledWith(alpha.id, false);
    await userEvent.click(screen.getByRole("button", { name: "继续更新" }));
    await waitFor(() => expect(updateSkill).toHaveBeenLastCalledWith(alpha.id, true));
    expect(await screen.findByText("已更新到 abc1234")).toBeInTheDocument();
  });

  it("uses the unknown-state wording when the baseline is missing", async () => {
    (checkSkillUpdates as Mock).mockResolvedValue([mkUpdate({ skill_id: alpha.id, has_update: true, local_state_unknown: true })]);
    (updateSkill as Mock).mockRejectedValueOnce({ code: "local_changes", message: "local changes" });
    await open();
    await userEvent.click(screen.getByRole("button", { name: /检查更新/ }));
    await screen.findByText("有更新");
    await userEvent.click(screen.getByRole("button", { name: "更新" }));
    expect(
      await screen.findByText("无法判断本地是否修改过，更新会覆盖当前内容（已自动备份），继续？"),
    ).toBeInTheDocument();
    // cancelling never retries
    await userEvent.click(screen.getByRole("button", { name: "取消" }));
    expect(updateSkill).toHaveBeenCalledTimes(1);
  });

  it("takes the confirmation wording from the error, without a prior update check", async () => {
    (updateSkill as Mock).mockRejectedValueOnce({ code: "local_changes", message: "local changes", unknown: true });
    await open();
    await userEvent.click(screen.getByRole("button", { name: "更新" }));
    expect(await screen.findByText(/无法判断本地是否修改过/)).toBeInTheDocument();
  });

  it("clamps long toast messages and offers to expand them", async () => {
    (updateSkill as Mock).mockRejectedValueOnce({ code: "git_failed", message: "fatal: ".concat("x".repeat(300)) });
    await open();
    await userEvent.click(screen.getByRole("button", { name: "更新" }));
    const more = await screen.findByRole("button", { name: "展开" });
    await userEvent.click(more);
    expect(screen.getByRole("button", { name: "收起" })).toBeInTheDocument();
  });

  it("shows a warning when files were updated but the registry was not saved", async () => {
    (updateSkill as Mock).mockRejectedValueOnce({ code: "io", message: "files updated, registry not saved: denied" });
    await open();
    const callsBefore = (listSkills as Mock).mock.calls.length;
    await userEvent.click(screen.getByRole("button", { name: "更新" }));
    expect(await screen.findByText(/文件已更新，但 MemDock 的记录没有保存成功/)).toBeInTheDocument();
    await waitFor(() => expect((listSkills as Mock).mock.calls.length).toBeGreaterThan(callsBefore));
  });

  it("delete symlink shows link-only notice", async () => {
    (deleteSkill as Mock).mockResolvedValue({ id: "1700000000000", created_at: 1, size_bytes: 1 });
    await open();
    await userEvent.click(screen.getByRole("option", { name: /linked/ }));
    await screen.findByRole("heading", { name: "linked" });
    await userEvent.click(screen.getByRole("button", { name: "删除" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(within(dialog).getByText(/只删除链接，不会删除 → C:\\repo\\linked/)).toBeInTheDocument();
    expect(within(dialog).getByText(/可在列表底部「已删除的 skill」里恢复/)).toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole("button", { name: "删除" }));
    await waitFor(() => expect(deleteSkill).toHaveBeenCalledWith(link.id));
    expect(await screen.findByText(/已删除 linked，可在列表底部「已删除的 skill」里恢复/)).toBeInTheDocument();
  });

  it("tells that a dangling link needed no backup", async () => {
    (deleteSkill as Mock).mockResolvedValue({ id: "", created_at: 0, size_bytes: 0 });
    await open();
    await userEvent.click(screen.getByRole("button", { name: "删除" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(within(dialog).getByText("会先备份，可在列表底部「已删除的 skill」里恢复。")).toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole("button", { name: "删除" }));
    expect(await screen.findByText("已删除失效的链接（无需备份）")).toBeInTheDocument();
  });

  it("opens the backups drawer for the opened skill", async () => {
    await open();
    await userEvent.click(screen.getByRole("button", { name: "备份与回滚" }));
    await waitFor(() => expect(listSkillBackups).toHaveBeenCalledWith(alpha.id));
    expect(await screen.findByText("还没有备份")).toBeInTheDocument();
  });

  it("opens the add dialog from the list", async () => {
    await open();
    await userEvent.click(screen.getByRole("button", { name: "添加 skill" }));
    expect(await screen.findByRole("dialog", { name: "添加 skill" })).toBeInTheDocument();
  });
});
