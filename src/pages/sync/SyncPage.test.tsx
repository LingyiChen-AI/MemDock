import { act, cleanup, screen, waitFor, within } from "@testing-library/react";
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
  syncSkippedFiles: vi.fn(),
  syncFinish: vi.fn(),
  syncDiscard: vi.fn().mockResolvedValue(undefined),
  syncProjects: vi.fn(),
  setIncludedLocal: vi.fn(),
  syncHistory: vi.fn(),
  syncFileAt: vi.fn(),
  restoreFromHistory: vi.fn().mockResolvedValue(undefined),
  logout: vi.fn(),
  prefsSyncInfo: vi.fn().mockResolvedValue({ with_memory: true, remote: null, newer: false, seen: null }),
}));

import { installSkill } from "@/api/skills";
import { restoreFromHistory, syncDiscard, syncFinish, syncHistory, syncProjects, syncSkippedFiles, syncStart, syncStatus } from "@/api/sync";
import { renderApp } from "@/test/fixtures";
import { SyncPage } from "./SyncPage";

const status: SyncStatus = {
  logged_in: true,
  login: "octo",
  repo: { owner: "octo", name: "MemDockHub", url: "https://github.com/octo/MemDockHub", private: true },
  last_sync_at: null,
  pending: 2,
  needs: null,
  remote_has_memory: false,
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
  download: false,
  kept_local: [],
  absent: [],
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
  (syncSkippedFiles as Mock).mockResolvedValue([]);
});

function setup(s: SyncStatus | null = status) {
  const onOpenLogin = vi.fn();
  renderApp(<SyncPage status={s} onStatus={vi.fn()} onOpenLogin={onOpenLogin} />);
  return { onOpenLogin };
}

async function startWith(p: SyncPlan) {
  (syncStart as Mock).mockResolvedValue({ kind: "needs_resolution", plan: p });
  setup();
  await userEvent.click(await screen.findByRole("button", { name: "上传到云端" }));
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
    await userEvent.click(screen.getByRole("button", { name: "上传到云端" }));
    expect(await screen.findByText("已同步：拉取 1、推送 2、自动合并 3")).toBeInTheDocument();
    expect(screen.getByText("本机文件不是 UTF-8 文本")).toBeInTheDocument();
  });

  it("shows syncing state and disables the button", async () => {
    let resolve: (v: unknown) => void = () => {};
    (syncStart as Mock).mockReturnValue(new Promise((r) => (resolve = r)));
    setup();
    await userEvent.click(await screen.findByRole("button", { name: "上传到云端" }));
    expect(screen.getByRole("button", { name: "上传中…" })).toBeDisabled();
    await act(async () => resolve({ kind: "blocked", secret_hits: [] }));
  });

  it("maps error codes to messages", async () => {
    (syncStart as Mock).mockRejectedValue({ code: "sync_busy", message: "busy" });
    setup();
    await userEvent.click(await screen.findByRole("button", { name: "上传到云端" }));
    expect(await screen.findByText("已有一次同步正在进行，请稍后再试。")).toBeInTheDocument();
  });

  it("blocked_lists_hits_without_secret_text", async () => {
    (syncStart as Mock).mockResolvedValue({
      kind: "blocked",
      secret_hits: [{ repo_path: "global/claude/CLAUDE.md", line: 12, rule: "github_token" }],
    });
    setup();
    await userEvent.click(await screen.findByRole("button", { name: "上传到云端" }));
    const box = await screen.findByRole("dialog", { name: "发现疑似密钥" });
    expect(within(box).getByText("CLAUDE.md")).toBeInTheDocument();
    expect(within(box).getByText("global/claude")).toBeInTheDocument();
    expect(within(box).getByTitle("global/claude/CLAUDE.md")).toBeInTheDocument();
    expect(within(box).getByText("第 12 行")).toBeInTheDocument();
    expect(within(box).getByText("github_token")).toBeInTheDocument();
    const allow = within(box).getByRole("button", { name: "坚持同步" });
    expect(allow).toHaveAccessibleDescription(/密钥可能因此泄露/);
    await userEvent.click(within(box).getByRole("button", { name: "跳过这些文件，继续同步" }));
    expect(syncStart).toHaveBeenLastCalledWith("skip", "up", expect.any(Object));
  });

  it("leaving the page mid-resolution discards the plan", async () => {
    const p = plan();
    await startWith(p);
    cleanup();
    expect(syncDiscard).toHaveBeenCalledWith(p.id);
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
    await userEvent.click(await screen.findByRole("button", { name: "上传到云端" }));
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

const pdfSkill = {
  name: "pdf",
  url: "https://github.com/x/skills",
  git_ref: "main",
  commit: "abc",
  skill_path: "pdf",
  tool: "claude" as const,
  scope: { type: "project" as const, key: "github.com/x/jev" },
  installed_on: ["m1"],
  removed_on: [],
};

const doneDown = (over: Record<string, unknown> = {}) => ({
  kind: "done",
  summary: { commit: "c1", committed: false, pulled: 4, pushed: 0, merged: 1 },
  skipped: [],
  missing_skills: [],
  removed_elsewhere: [],
  download: true,
  kept_local: [],
  absent: [],
  ...over,
});

describe("SyncPage directions", () => {
  it("shows both actions with hints", async () => {
    setup();
    const up = await screen.findByRole("button", { name: "上传到云端" });
    expect(up).toHaveAccessibleDescription("先合并云端的改动，再把本机的改动推上去");
    const down = screen.getByRole("button", { name: "同步到本地" });
    expect(down).toHaveAccessibleDescription("只把云端的记忆拉到本机，不上传任何内容");
  });

  it("download sends direction down and reports what stayed out", async () => {
    (syncStart as Mock).mockResolvedValue(
      doneDown({
        kept_local: ["global/claude/CLAUDE.md"],
        absent: [
          { repo_path: "projects/github.com/x/other/claude/CLAUDE.md", project_key: "github.com/x/other", reason: "no_project" },
          { repo_path: "projects/github.com/x/other/codex/AGENTS.md", project_key: "github.com/x/other", reason: "no_project" },
        ],
      }),
    );
    setup();
    await userEvent.click(await screen.findByRole("button", { name: "同步到本地" }));
    expect(syncStart).toHaveBeenLastCalledWith("block", "down", undefined);
    expect(await screen.findByText("已同步到本地：写入 4、合并 1")).toBeInTheDocument();
    expect(screen.getByText("本机还有 1 处改动没有上传，需要时点「上传到云端」。")).toBeInTheDocument();
    const absent = screen.getByRole("region", { name: "这些云端文件没有写入本机（2）" });
    expect(within(absent).getByText("github.com/x/other")).toBeInTheDocument();
    expect(within(absent).getByText("本机没有这个项目")).toBeInTheDocument();
    expect(within(absent).getByText("2 个文件")).toBeInTheDocument();
  });

  it("new computer is offered a one-click download", async () => {
    (syncStart as Mock).mockResolvedValue(doneDown());
    setup({ ...status, remote_has_memory: true });
    const banner = await screen.findByRole("region", { name: "检测到云端已有记忆" });
    await userEvent.click(within(banner).getByRole("button", { name: "一键同步到本机" }));
    expect(syncStart).toHaveBeenLastCalledWith("block", "down", undefined);
  });

  it("no new-computer banner after a sync", async () => {
    setup({ ...status, remote_has_memory: false, last_sync_at: 1 });
    await screen.findByRole("button", { name: "上传到云端" });
    expect(screen.queryByRole("region", { name: "检测到云端已有记忆" })).not.toBeInTheDocument();
  });

  it("download conflicts use local-only wording", async () => {
    (syncStart as Mock).mockResolvedValue({ kind: "needs_resolution", plan: plan({ download: true }) });
    setup();
    await userEvent.click(await screen.findByRole("button", { name: "同步到本地" }));
    expect(await screen.findByText("处理同步到本地的冲突")).toBeInTheDocument();
    expect(screen.getByText(/处理结果只写入本机，不会上传/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "写入本机" })).toBeDisabled();
  });

  it("lists persisted unsynced files and opens them", async () => {
    (syncSkippedFiles as Mock).mockResolvedValue([
      {
        repo_path: "global/claude/CLAUDE.md",
        reason: "secret",
        at: 1_700_000_000_000,
        local_path: "/home/u/.claude/CLAUDE.md",
        file_id: "/home/u/.claude/claude.md",
      },
    ]);
    const onOpen = vi.fn();
    renderApp(<SyncPage status={status} onStatus={vi.fn()} onOpenLogin={vi.fn()} onOpenMemoryFile={onOpen} />);
    const box = await screen.findByRole("region", { name: "未同步的文件（1）" });
    expect(within(box).getByText("CLAUDE.md")).toBeInTheDocument();
    expect(within(box).getByText("/home/u/.claude")).toBeInTheDocument();
    expect(within(box).getByText("疑似包含密钥")).toBeInTheDocument();
    expect(within(box).getByText(/^跳过于 /)).toBeInTheDocument();
    await userEvent.click(within(box).getByRole("button", { name: "在记忆页打开 CLAUDE.md" }));
    expect(onOpen).toHaveBeenCalledWith("/home/u/.claude/claude.md");
  });

  it("install all asks first, lists blocked skills, then installs one by one", async () => {
    const other = { ...pdfSkill, name: "xlsx", skill_path: "xlsx", scope: { type: "project" as const, key: "github.com/x/absent" } };
    const global = { ...pdfSkill, name: "docx", skill_path: "docx", scope: { type: "global" as const } };
    (syncStart as Mock).mockResolvedValue(doneDown({ missing_skills: [pdfSkill, other, global] }));
    setup();
    await userEvent.click(await screen.findByRole("button", { name: "同步到本地" }));
    await userEvent.click(await screen.findByRole("button", { name: "全部安装" }));
    const dlg = await screen.findByRole("dialog", { name: "安装云端的 Skill" });
    expect(installSkill).not.toHaveBeenCalled();
    expect(within(dlg).getByText("以下 1 个无法自动安装（本机没有对应项目）：")).toBeInTheDocument();
    expect(within(dlg).getByText("xlsx")).toBeInTheDocument();
    await userEvent.click(within(dlg).getByRole("button", { name: "全部安装" }));
    await waitFor(() => expect(installSkill).toHaveBeenCalledTimes(2));
    expect((installSkill as Mock).mock.calls.map((c) => c[0].skill_path)).toEqual(["pdf", "docx"]);
    expect(await screen.findByText("Skill 安装完成：成功 2，失败 0")).toBeInTheDocument();
  });
});
