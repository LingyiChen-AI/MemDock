import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi, type Mock } from "vitest";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn().mockResolvedValue({ language: "zh", theme: null, accent: null, manual_projects: [] }),
  updateUiPrefs: vi.fn(),
}));
vi.mock("@/api/skills", () => ({
  inspectGitSource: vi.fn(),
  installSkill: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

import { inspectGitSource, installSkill } from "@/api/skills";
import type { InspectResult } from "@/api/types";
import { AppProviders } from "@/providers";
import { mkProject, renderApp } from "@/test/fixtures";
import { AddSkillDialog } from "./AddSkillDialog";
import { mkSkill } from "./testUtils";
import { optionLabels, selectOption } from "@/test/selectOption";

const inspected: InspectResult = {
  url: "https://github.com/o/repo",
  clone_url: "https://github.com/o/repo.git",
  git_ref: null,
  commit: "abc1234567890",
  preselected: null,
  skills: [
    { skill_path: "skills/alpha", name: "alpha", description: "Alpha does a thing" },
    { skill_path: "skills/beta", name: "beta", description: "Beta does another" },
    { skill_path: "tools/gamma", name: "gamma", description: "Gamma" },
  ],
};

const projects = [
  mkProject({ id: "c:/work/jev", name: "jev", path: "C:\\work\\jev" }),
  mkProject({ id: "c:/work/gone", name: "gone", path: "C:\\work\\gone", exists: false }),
];

function setup(over: Record<string, unknown> = {}) {
  const props = {
    open: true,
    onOpenChange: vi.fn(),
    projects,
    gitOk: true,
    onInstalled: vi.fn(),
    ...over,
  };
  renderApp(<AddSkillDialog {...props} />);
  return props;
}

beforeEach(() => vi.clearAllMocks());

describe("AddSkillDialog", () => {
  it("github tab inspects and lists multiple skills", async () => {
    (inspectGitSource as Mock).mockResolvedValue(inspected);
    (installSkill as Mock).mockResolvedValue([mkSkill({ name: "alpha" })]);
    setup();
    await userEvent.type(screen.getByLabelText("仓库地址"), "o/repo");
    await userEvent.click(screen.getByRole("button", { name: "解析" }));
    expect(inspectGitSource).toHaveBeenCalledWith("o/repo");
    expect(await screen.findByText("alpha")).toBeInTheDocument();
    expect(screen.getByText("Beta does another")).toBeInTheDocument();
    expect(screen.getByText("tools/gamma")).toBeInTheDocument();
    // several skills: nothing selected yet, install is disabled
    const install = screen.getByRole("button", { name: "安装" });
    expect(install).toBeDisabled();
    await userEvent.click(screen.getByRole("checkbox", { name: /alpha/ }));
    await userEvent.click(screen.getByRole("checkbox", { name: /gamma/ }));
    expect(install).toBeEnabled();
    await userEvent.click(install);
    await waitFor(() => expect(installSkill).toHaveBeenCalledTimes(2));
    expect((installSkill as Mock).mock.calls[0]).toEqual([
      { type: "git", url: inspected.url, clone_url: inspected.clone_url, git_ref: null, skill_path: "skills/alpha", commit: inspected.commit },
      [{ tool: "claude", scope: { type: "global" } }],
      false,
    ]);
    expect((installSkill as Mock).mock.calls[1][0].skill_path).toBe("tools/gamma");
  });

  it("selects a single skill by default and the pinned one for tree urls", async () => {
    (inspectGitSource as Mock).mockResolvedValue({ ...inspected, skills: [inspected.skills[0]] });
    setup();
    await userEvent.type(screen.getByLabelText("仓库地址"), "o/repo");
    await userEvent.click(screen.getByRole("button", { name: "解析" }));
    expect(await screen.findByRole("checkbox", { name: /alpha/ })).toBeChecked();
    expect(screen.getByRole("button", { name: "安装" })).toBeEnabled();
  });

  it("shows the git error summary and keeps the dialog open", async () => {
    (inspectGitSource as Mock).mockRejectedValue({ code: "git_failed", message: "fatal: repository not found" });
    const props = setup();
    await userEvent.type(screen.getByLabelText("仓库地址"), "o/missing");
    await userEvent.click(screen.getByRole("button", { name: "解析" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("fatal: repository not found");
    expect(props.onOpenChange).not.toHaveBeenCalled();
  });

  it("git unavailable disables github tab", async () => {
    setup({ gitOk: false });
    expect(screen.getByRole("tab", { name: "从 GitHub" })).toBeDisabled();
    expect(screen.getByRole("tab", { name: "本地文件夹 / zip" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByText(/没有检测到 git/)).toBeInTheDocument();
  });

  it("template name validation", async () => {
    (installSkill as Mock).mockResolvedValue([mkSkill({ name: "my-skill" })]);
    const props = setup();
    await userEvent.click(screen.getByRole("tab", { name: "空白模板" }));
    const name = screen.getByLabelText("名称");
    const install = screen.getByRole("button", { name: "安装" });
    expect(install).toBeDisabled();
    await userEvent.type(name, "My Skill");
    expect(screen.getByText(/名称格式不对/)).toBeInTheDocument();
    expect(install).toBeDisabled();
    await userEvent.clear(name);
    await userEvent.type(name, "my-skill");
    expect(screen.queryByText(/名称格式不对/)).not.toBeInTheDocument();
    expect(install).toBeEnabled();
    await userEvent.type(screen.getByLabelText("描述"), "Use when testing");
    await userEvent.click(install);
    await waitFor(() => expect(installSkill).toHaveBeenCalled());
    expect((installSkill as Mock).mock.calls[0][0]).toEqual({ type: "template", name: "my-skill", description: "Use when testing" });
    await waitFor(() => expect(props.onInstalled).toHaveBeenCalled());
  });

  it("install exists asks overwrite then retries", async () => {
    (installSkill as Mock)
      .mockRejectedValueOnce({ code: "exists", message: "exists", dir: "C:\\Users\\u\\.claude\\skills\\my-skill" })
      .mockResolvedValueOnce([mkSkill({ name: "my-skill" })]);
    const props = setup();
    await userEvent.click(screen.getByRole("tab", { name: "空白模板" }));
    await userEvent.type(screen.getByLabelText("名称"), "my-skill");
    await userEvent.click(screen.getByRole("button", { name: "安装" }));
    expect(await screen.findByText(/已存在，覆盖（会先备份）？/)).toBeInTheDocument();
    expect((installSkill as Mock).mock.calls[0][2]).toBe(false);
    await userEvent.click(screen.getByRole("button", { name: "覆盖" }));
    await waitFor(() => expect(installSkill).toHaveBeenCalledTimes(2));
    expect((installSkill as Mock).mock.calls[1][2]).toBe(true);
    await waitFor(() => expect(props.onInstalled).toHaveBeenCalled());
  });

  it("lists every conflicting directory and overwrites only after confirmation", async () => {
    (installSkill as Mock)
      .mockRejectedValueOnce({ code: "exists", message: "exists", dir: "C:\\a\\demo", dirs: ["C:\\a\\demo", "C:\\b\\demo"] })
      .mockResolvedValueOnce([mkSkill({ name: "demo" })]);
    setup();
    await userEvent.click(screen.getByRole("tab", { name: "空白模板" }));
    await userEvent.type(screen.getByLabelText("名称"), "demo");
    await userEvent.click(screen.getByRole("checkbox", { name: "Codex" }));
    await userEvent.click(screen.getByRole("button", { name: "安装" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog).toHaveTextContent("C:\\a\\demo");
    expect(dialog).toHaveTextContent("C:\\b\\demo");
    expect(installSkill).toHaveBeenCalledTimes(1);
    await userEvent.click(screen.getByRole("button", { name: "覆盖" }));
    await waitFor(() => expect(installSkill).toHaveBeenCalledTimes(2));
    expect((installSkill as Mock).mock.calls[1][2]).toBe(true);
  });

  it("does not retry when the overwrite is declined", async () => {
    (installSkill as Mock).mockRejectedValueOnce({ code: "exists", message: "exists", dir: "C:\\a\\demo", dirs: ["C:\\a\\demo"] });
    setup();
    await userEvent.click(screen.getByRole("tab", { name: "空白模板" }));
    await userEvent.type(screen.getByLabelText("名称"), "demo");
    await userEvent.click(screen.getByRole("button", { name: "安装" }));
    await screen.findByRole("alertdialog");
    await userEvent.click(screen.getByRole("button", { name: "取消" }));
    expect(installSkill).toHaveBeenCalledTimes(1);
  });

  it("ignores an inspect response that arrives after the dialog was reopened", async () => {
    let resolve!: (r: InspectResult) => void;
    (inspectGitSource as Mock).mockReturnValue(new Promise<InspectResult>((r) => (resolve = r)));
    const props = { open: true, onOpenChange: vi.fn(), projects, gitOk: true, onInstalled: vi.fn() };
    vi.spyOn(navigator, "language", "get").mockReturnValue("zh-CN");
    const view = render(<AppProviders><AddSkillDialog {...props} /></AppProviders>);
    await userEvent.type(screen.getByLabelText("仓库地址"), "o/repo");
    await userEvent.click(screen.getByRole("button", { name: "解析" }));
    view.rerender(<AppProviders><AddSkillDialog {...props} open={false} /></AppProviders>);
    view.rerender(<AppProviders><AddSkillDialog {...props} open /></AppProviders>);
    resolve(inspected);
    await new Promise((r) => setTimeout(r, 20));
    expect(screen.queryByText("Beta does another")).not.toBeInTheDocument();
  });

  it("only offers projects whose directory exists, and passes the project scope", async () => {
    (installSkill as Mock).mockResolvedValue([mkSkill({ name: "my-skill" })]);
    setup();
    await userEvent.click(screen.getByRole("tab", { name: "空白模板" }));
    await userEvent.type(screen.getByLabelText("名称"), "my-skill");
    await userEvent.click(screen.getByRole("radio", { name: "选择项目" }));
    const select = screen.getByLabelText("项目");
    expect(select).toHaveTextContent("选择项目");
    expect(await optionLabels(select)).toEqual(["jev"]);
    expect(screen.getByRole("button", { name: "安装" })).toBeDisabled();
    await selectOption(select, "c:/work/jev");
    await userEvent.click(screen.getByRole("checkbox", { name: "Codex" }));
    await userEvent.click(screen.getByRole("button", { name: "安装" }));
    await waitFor(() => expect(installSkill).toHaveBeenCalled());
    expect((installSkill as Mock).mock.calls[0][1]).toEqual([
      { tool: "claude", scope: { type: "project", project_id: "c:/work/jev" } },
      { tool: "codex", scope: { type: "project", project_id: "c:/work/jev" } },
    ]);
  });

  it("requires at least one tool", async () => {
    setup();
    await userEvent.click(screen.getByRole("tab", { name: "空白模板" }));
    await userEvent.type(screen.getByLabelText("名称"), "my-skill");
    await userEvent.click(screen.getByRole("checkbox", { name: "Claude Code" }));
    expect(screen.getByRole("button", { name: "安装" })).toBeDisabled();
    expect(screen.getByText("至少选一个工具")).toBeInTheDocument();
  });

  it("keeps the dialog open and shows the error inline when installing fails", async () => {
    (installSkill as Mock).mockRejectedValue({ code: "invalid", message: "project directory not found" });
    const props = setup();
    await userEvent.click(screen.getByRole("tab", { name: "空白模板" }));
    await userEvent.type(screen.getByLabelText("名称"), "my-skill");
    await userEvent.click(screen.getByRole("button", { name: "安装" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("项目目录不存在");
    expect(props.onOpenChange).not.toHaveBeenCalledWith(false);
  });
});
