import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi, type Mock } from "vitest";
import type { SyncConflict, SyncPlan, SyncStatus } from "@/api/types";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn().mockResolvedValue({ language: "zh", theme: null, accent: null, manual_projects: [] }),
  updateUiPrefs: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn().mockResolvedValue(undefined) }));
vi.mock("@/api/skills", () => ({
  installSkill: vi.fn().mockResolvedValue([]),
  deleteSkill: vi.fn(),
  listSkills: vi.fn().mockResolvedValue([]),
}));
vi.mock("@/api/sync", () => ({
  syncStatus: vi.fn(),
  syncStart: vi.fn(),
  syncFinish: vi.fn(),
  syncDiscard: vi.fn().mockResolvedValue(undefined),
  syncProjects: vi.fn(),
  setIncludedLocal: vi.fn(),
  syncHistory: vi.fn(),
  syncFileAt: vi.fn(),
  restoreFromHistory: vi.fn().mockResolvedValue(undefined),
  logout: vi.fn(),
}));

import { installSkill } from "@/api/skills";
import { restoreFromHistory, syncFinish, syncHistory, syncProjects, syncStart, syncStatus } from "@/api/sync";
import { renderApp } from "@/test/fixtures";
import { SyncPage } from "./SyncPage";

const status: SyncStatus = {
  logged_in: true,
  login: "octo",
  repo: { owner: "octo", name: "MemDockHub", url: "https://github.com/octo/MemDockHub", private: true },
  last_sync_at: null,
  pending: 2,
  needs: null,
};

const textConflict: SyncConflict = {
  repo_path: "global/claude/CLAUDE.md",
  local_path: "/home/u/.claude/CLAUDE.md",
  local: "a\nmine\n",
  remote: "a\ntheirs\n",
  base: "a\n",
  merged: "a\n<<<<<<< local\nmine\n||||||| base\n=======\ntheirs\n>>>>>>> remote\n",
  conflicts: 1,
};
const deletedConflict: SyncConflict = {
  repo_path: "global/codex/AGENTS.md",
  local_path: "/home/u/.codex/AGENTS.md",
  local: "kept here\n",
  remote: null,
  base: "old\n",
  merged: "",
  conflicts: 1,
};

const plan = (over: Partial<SyncPlan> = {}): SyncPlan => ({
  id: "p1",
  counts: { pull: 0, push: 0, merged: 0, conflicts: 2, remote_deleted: 0, local_all_deleted: 0, skipped: 0 },
  conflicts: [textConflict, deletedConflict],
  remote_deleted: [],
  local_all_deleted: [],
  skipped: [],
  secret_hits: [],
  missing_skills: [],
  removed_elsewhere: [],
  ...over,
});

const summary = { commit: "c1", committed: true, pulled: 1, pushed: 2, merged: 3 };

beforeEach(() => {
  vi.clearAllMocks();
  (syncStatus as Mock).mockResolvedValue(status);
  (syncProjects as Mock).mockResolvedValue([
    { project_id: "/work/jev", name: "jev", key: "github.com/x/jev", dir_key: "jev", included: true },
  ]);
  (syncHistory as Mock).mockResolvedValue([]);
});

function setup(s: SyncStatus | null = status) {
  const onOpenLogin = vi.fn();
  renderApp(<SyncPage status={s} onStatus={vi.fn()} onOpenLogin={onOpenLogin} />);
  return { onOpenLogin };
}

async function startWith(p: SyncPlan) {
  (syncStart as Mock).mockResolvedValue({ kind: "needs_resolution", plan: p });
  setup();
  await userEvent.click(await screen.findByRole("button", { name: "同步记忆" }));
  await screen.findByText("处理同步冲突");
}

describe("SyncPage", () => {
  it("not_logged_in_shows_login", async () => {
    const { onOpenLogin } = setup({ ...status, logged_in: false, login: null, repo: null });
    await userEvent.click(await screen.findByRole("button", { name: "登录 GitHub" }));
    expect(onOpenLogin).toHaveBeenCalled();
  });

  it("sync_done_toast", async () => {
    (syncStart as Mock).mockResolvedValue({
      kind: "done",
      summary,
      skipped: [{ repo_path: "global/claude/x.md", reason: "not_utf8_local" }],
      missing_skills: [],
      removed_elsewhere: [],
    });
    setup();
    expect(await screen.findByText("octo/MemDockHub")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "同步记忆" }));
    expect(await screen.findByText("已同步：拉取 1、推送 2、自动合并 3")).toBeInTheDocument();
    expect(screen.getByText("本机文件不是 UTF-8 文本")).toBeInTheDocument();
  });

  it("shows syncing state and disables the button", async () => {
    let resolve: (v: unknown) => void = () => {};
    (syncStart as Mock).mockReturnValue(new Promise((r) => (resolve = r)));
    setup();
    await userEvent.click(await screen.findByRole("button", { name: "同步记忆" }));
    expect(screen.getByRole("button", { name: "同步中…" })).toBeDisabled();
    await act(async () => resolve({ kind: "blocked", secret_hits: [] }));
  });

  it("maps error codes to messages", async () => {
    (syncStart as Mock).mockRejectedValue({ code: "sync_busy", message: "busy" });
    setup();
    await userEvent.click(await screen.findByRole("button", { name: "同步记忆" }));
    expect(await screen.findByText("已有一次同步正在进行，请稍后再试。")).toBeInTheDocument();
  });

  it("blocked_lists_hits_without_secret_text", async () => {
    (syncStart as Mock).mockResolvedValue({
      kind: "blocked",
      secret_hits: [{ repo_path: "global/claude/CLAUDE.md", line: 12, rule: "github_token" }],
    });
    setup();
    await userEvent.click(await screen.findByRole("button", { name: "同步记忆" }));
    const box = await screen.findByRole("dialog", { name: "发现疑似密钥" });
    expect(within(box).getByText("global/claude/CLAUDE.md")).toBeInTheDocument();
    expect(within(box).getByText("第 12 行")).toBeInTheDocument();
    expect(within(box).getByText("github_token")).toBeInTheDocument();
    await userEvent.click(within(box).getByRole("button", { name: "跳过这些文件，继续同步" }));
    expect(syncStart).toHaveBeenLastCalledWith("skip");
  });

  it("finish_disabled_until_resolved", async () => {
    await startWith(plan());
    const finish = screen.getByRole("button", { name: "完成同步" });
    expect(finish).toBeDisabled();
    await userEvent.click(screen.getByRole("button", { name: "用远端" }));
    expect(finish).toBeDisabled();
  });

  it("resolve_all_then_finish_sends_texts", async () => {
    (syncFinish as Mock).mockResolvedValue(summary);
    await startWith(plan());
    await userEvent.click(screen.getByRole("button", { name: "用远端" }));
    await userEvent.click(screen.getByRole("button", { name: /global\/codex\/AGENTS\.md/ }));
    await userEvent.click(screen.getByRole("button", { name: "保留" }));
    await userEvent.click(screen.getByRole("button", { name: "完成同步" }));
    expect(syncFinish).toHaveBeenCalledWith("p1", {
      "global/claude/CLAUDE.md": { kind: "text", text: "a\ntheirs\n" },
      "global/codex/AGENTS.md": { kind: "text", text: "kept here\n" },
    });
    expect(await screen.findByText("同步完成：拉取 1、推送 2、合并 3")).toBeInTheDocument();
  });

  it("manual_edit_with_markers_rejected", async () => {
    await startWith(plan({ conflicts: [textConflict] }));
    await userEvent.click(screen.getByRole("button", { name: "手动编辑合并结果" }));
    await userEvent.click(screen.getByRole("button", { name: "用这个结果" }));
    expect(await screen.findByText(/合并结果里还有冲突标记/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "完成同步" })).toBeDisabled();
  });

  it("remote_deleted_keep_or_delete", async () => {
    (syncFinish as Mock).mockResolvedValue(summary);
    await startWith(
      plan({
        conflicts: [],
        remote_deleted: [{ repo_path: "global/claude/old.md", local_path: "/home/u/.claude/old.md" }],
        local_all_deleted: [{ project_key: "github.com/x/gone", files: [{ repo_path: "projects/gone/CLAUDE.md", local_path: null }] }],
      }),
    );
    await userEvent.click(screen.getByRole("button", { name: "删除本机文件" }));
    await userEvent.click(screen.getByRole("button", { name: /github\.com\/x\/gone/ }));
    await userEvent.click(screen.getByRole("button", { name: "删除远端这些文件" }));
    await userEvent.click(screen.getByRole("button", { name: "完成同步" }));
    expect(syncFinish).toHaveBeenCalledWith("p1", {
      "global/claude/old.md": { kind: "delete_local" },
      "projects/gone/CLAUDE.md": { kind: "delete_remote" },
    });
  });

  it("install_missing_skill_calls_install", async () => {
    (syncStart as Mock).mockResolvedValue({
      kind: "done",
      summary,
      skipped: [],
      missing_skills: [
        {
          name: "pdf",
          url: "https://github.com/x/skills",
          git_ref: "main",
          commit: "abc",
          skill_path: "pdf",
          tool: "claude",
          scope: { type: "project", key: "github.com/x/jev" },
          installed_on: ["m1"],
          removed_on: [],
        },
      ],
      removed_elsewhere: [],
    });
    setup();
    await userEvent.click(await screen.findByRole("button", { name: "同步记忆" }));
    await userEvent.click(await screen.findByRole("button", { name: "安装 pdf" }));
    expect(installSkill).toHaveBeenCalledWith(
      { type: "git", url: "https://github.com/x/skills", clone_url: "https://github.com/x/skills", git_ref: "main", skill_path: "pdf", commit: "abc" },
      [{ tool: "claude", scope: { type: "project", project_id: "/work/jev" } }],
      false,
    );
  });

  it("history_restore_confirms", async () => {
    (syncHistory as Mock).mockResolvedValue([
      { id: "c9", message: "sync from laptop", time: 1_700_000_000_000, files: ["global/claude/CLAUDE.md"] },
    ]);
    setup();
    await userEvent.click(await screen.findByRole("button", { name: /sync from laptop/ }));
    await userEvent.click(screen.getByRole("button", { name: "恢复到本机" }));
    expect(restoreFromHistory).not.toHaveBeenCalled();
    await userEvent.click(await screen.findByRole("button", { name: "恢复" }));
    await waitFor(() => expect(restoreFromHistory).toHaveBeenCalledWith("c9", "global/claude/CLAUDE.md"));
  });
});
