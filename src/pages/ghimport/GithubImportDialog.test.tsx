import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi, type Mock } from "vitest";
import type { GhImportScan } from "@/api/types";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn().mockResolvedValue({ language: "zh", theme: null, accent: null, manual_projects: [] }),
  updateUiPrefs: vi.fn(),
}));
vi.mock("@/api/ghimport", () => ({
  ghImportScan: vi.fn(),
  ghImportPreview: vi.fn(),
  ghImportMemories: vi.fn(),
  ghImportSkill: vi.fn(),
}));

import { ghImportMemories, ghImportPreview, ghImportScan, ghImportSkill } from "@/api/ghimport";
import { mkProject, renderApp } from "@/test/fixtures";
import { GithubImportDialog, targetFor } from "./GithubImportDialog";

const scan: GhImportScan = {
  session_id: "sid",
  repo: "octo/mem",
  repo_url: "https://github.com/octo/mem",
  git_ref: "main",
  commit: "abcdef1234",
  scope: "",
  truncated: false,
  tree_truncated: false,
  authenticated: false,
  items: [
    { id: "m:CLAUDE.md", path: "CLAUDE.md", name: "CLAUDE.md", size: 40, too_large: false, type: "memory", memory_kind: "claude_md" },
    { id: "m:AGENTS.md", path: "AGENTS.md", name: "AGENTS.md", size: 20, too_large: false, type: "memory", memory_kind: "agents_md" },
    { id: "m:big/CLAUDE.md", path: "big/CLAUDE.md", name: "CLAUDE.md", size: 999999, too_large: true, type: "memory", memory_kind: "claude_md" },
    { id: "s:skills/pdf", path: "skills/pdf", name: "pdf", size: 100, too_large: false, type: "skill", files: 2 },
  ],
};

const projects = [mkProject({ id: "/work/jev", name: "jev" })];

function setup() {
  (ghImportScan as Mock).mockResolvedValue(scan);
  (ghImportPreview as Mock).mockImplementation(async (_s: string, id: string) => ({
    text: id === "s:skills/pdf" ? "---\nname: pdf\n---" : `# ${id}`,
    binary: false,
    files: id === "s:skills/pdf" ? [{ path: "SKILL.md", size: 10 }] : [],
  }));
  (ghImportMemories as Mock).mockResolvedValue({ added: 2, duplicates: 0, skipped: [] });
  (ghImportSkill as Mock).mockResolvedValue([{ name: "pdf" }]);
  const onOpenChange = vi.fn();
  renderApp(<GithubImportDialog open onOpenChange={onOpenChange} projects={projects} />);
  return { onOpenChange };
}

beforeEach(() => vi.clearAllMocks());

test("targetFor maps select values and auto mode", () => {
  const agents = scan.items[1];
  const claude = scan.items[0];
  expect(targetFor("auto", agents)).toEqual({ type: "global_agents" });
  expect(targetFor("auto", claude)).toEqual({ type: "global_claude" });
  expect(targetFor("pc:/work/jev", agents)).toEqual({ type: "project_claude", project_id: "/work/jev" });
  expect(targetFor("pa:/work/jev", claude)).toEqual({ type: "project_agents", project_id: "/work/jev" });
});

test("scan, preview, pick and import memories plus a skill", async () => {
  const user = userEvent.setup();
  setup();
  await user.type(screen.getByLabelText("GitHub 链接"), "octo/mem");
  await user.click(screen.getByRole("button", { name: "扫描" }));

  expect(await screen.findByText(/octo\/mem · main · abcdef1/)).toBeInTheDocument();
  expect(screen.getByText(/未登录 GitHub/)).toBeInTheDocument();
  // First importable item is previewed automatically.
  await waitFor(() => expect(screen.getByTestId("gh-preview")).toHaveTextContent("# m:CLAUDE.md"));
  // Few items: every importable one is pre-selected; the oversized one is disabled.
  expect(screen.getByRole("checkbox", { name: "big/CLAUDE.md" })).toBeDisabled();
  expect(screen.getByRole("checkbox", { name: "CLAUDE.md" })).toBeChecked();

  // Preview a skill.
  await user.click(screen.getByRole("button", { name: /^pdf/ }));
  await waitFor(() => expect(screen.getByTestId("gh-preview")).toHaveTextContent("name: pdf"));

  await user.click(screen.getByRole("button", { name: "导入 3 项" }));
  await waitFor(() => expect(screen.getByText("导入完成")).toBeInTheDocument());
  expect(ghImportMemories).toHaveBeenCalledWith("sid", [
    { item_id: "m:CLAUDE.md", target: { type: "global_claude" } },
    { item_id: "m:AGENTS.md", target: { type: "global_agents" } },
  ]);
  expect(ghImportSkill).toHaveBeenCalledWith("sid", "s:skills/pdf", [{ tool: "claude", scope: { type: "global" } }], false);
  expect(screen.getByText("已生成 2 条待审核建议")).toBeInTheDocument();
  expect(screen.getByText("已安装 skill：pdf")).toBeInTheDocument();
});

test("skill conflict asks before overwriting", async () => {
  const user = userEvent.setup();
  setup();
  (ghImportSkill as Mock)
    .mockRejectedValueOnce({ code: "exists", message: "already exists", dirs: ["/h/.claude/skills/pdf"] })
    .mockResolvedValueOnce([{ name: "pdf" }]);
  await user.type(screen.getByLabelText("GitHub 链接"), "octo/mem");
  await user.click(screen.getByRole("button", { name: "扫描" }));
  await screen.findByText(/octo\/mem · main/);
  await user.click(screen.getByRole("checkbox", { name: "CLAUDE.md" }));
  await user.click(screen.getByRole("checkbox", { name: "AGENTS.md" }));
  await user.click(screen.getByRole("button", { name: "导入 1 项" }));
  expect(await screen.findByText("覆盖已有的 skill？")).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "覆盖" }));
  await waitFor(() => expect(screen.getByText("已安装 skill：pdf")).toBeInTheDocument());
  expect((ghImportSkill as Mock).mock.calls[1][3]).toBe(true);
  expect(ghImportMemories).not.toHaveBeenCalled();
});

test("scan errors are localized", async () => {
  const user = userEvent.setup();
  setup();
  (ghImportScan as Mock).mockRejectedValueOnce({ code: "import_not_found", message: "not found" });
  await user.type(screen.getByLabelText("GitHub 链接"), "octo/none");
  await user.click(screen.getByRole("button", { name: "扫描" }));
  expect(await screen.findByText(/找不到这个仓库、分支或路径/)).toBeInTheDocument();
});
