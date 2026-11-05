import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn().mockResolvedValue({ language: "zh", theme: null, accent: null, manual_projects: [] }),
  updateUiPrefs: vi.fn(),
}));

import { renderApp } from "@/test/fixtures";
import type { SkillEntry, UpdateStatus } from "@/api/types";
import { SkillList } from "./SkillList";
import { mkSkill, mkUpdate } from "./testUtils";

const alpha = mkSkill({ name: "alpha", description: "Draws diagrams" });
const zeta = mkSkill({ name: "zeta", description: "Writes tests", tool: "codex" });
const proj = mkSkill({
  name: "beta",
  description: "Project helper",
  scope: { type: "project", project_id: "/work/jev-rpa" },
});
const plug = mkSkill({ name: "plugged", kind: "plugin", plugin: "superpowers", readonly: true, description: "From plugin" });
const sys = mkSkill({ name: "imagegen", kind: "codex_system", tool: "codex", readonly: true, description: "System skill" });
const gh = mkSkill({
  name: "gh-skill",
  description: "From GitHub",
  source: { url: "https://github.com/o/r", git_ref: null, commit: "abc1234567", skill_path: "", origin: "registry" },
});

function setup(entries: SkillEntry[], over: Partial<Parameters<typeof SkillList>[0]> = {}) {
  const props = {
    entries,
    projectName: (id: string) => (id === "/work/jev-rpa" ? "jev-rpa" : id),
    updates: new Map<string, UpdateStatus>(),
    selectedKey: undefined,
    onSelect: vi.fn(),
    gitOk: true,
    checking: false,
    checkedUpdates: null as number | null,
    onCheck: vi.fn(),
    onAdd: vi.fn(),
    ...over,
  };
  renderApp(<SkillList {...props} />);
  return props;
}

const names = () => within(screen.getByRole("listbox")).getAllByRole("option").map((o) => within(o).getByTestId("skill-name").textContent);

describe("SkillList", () => {
  it("list groups writable before readonly", () => {
    setup([plug, zeta, sys, alpha]);
    expect(names()).toEqual(["alpha", "zeta", "imagegen", "plugged"]);
  });

  it("read-only rows are dimmed to 60%", () => {
    setup([alpha, plug]);
    const opt = within(screen.getByRole("listbox")).getAllByRole("option").find((o) => within(o).queryByText("plugged"))!;
    expect(opt.className).toContain("opacity-60");
  });

  it("search filters by name and description", async () => {
    setup([alpha, zeta, proj]);
    const box = screen.getByRole("searchbox", { name: "搜索 skill" });
    await userEvent.type(box, "ALP");
    expect(names()).toEqual(["alpha"]);
    await userEvent.clear(box);
    await userEvent.type(box, "writes TESTS");
    expect(names()).toEqual(["zeta"]);
    await userEvent.clear(box);
    await userEvent.type(box, "nothing-here");
    expect(screen.getByText("没有匹配的 skill")).toBeInTheDocument();
  });

  it("tool and scope filters", async () => {
    setup([alpha, zeta, proj, plug]);
    await userEvent.selectOptions(screen.getByLabelText("工具"), "codex");
    expect(names()).toEqual(["zeta"]);
    await userEvent.selectOptions(screen.getByLabelText("工具"), "all");
    await userEvent.selectOptions(screen.getByLabelText("范围"), "project");
    expect(names()).toEqual(["beta"]);
    await userEvent.selectOptions(screen.getByLabelText("范围"), "global");
    expect(names()).toEqual(["alpha", "zeta", "plugged"]);
    await userEvent.selectOptions(screen.getByLabelText("范围"), "readonly");
    expect(names()).toEqual(["plugged"]);
  });

  it("shows tool, scope and source tags", () => {
    setup([gh, proj, plug, sys]);
    const row = (n: string) => within(screen.getByRole("listbox")).getAllByRole("option").find((o) => within(o).queryByText(n))!;
    expect(within(row("gh-skill")).getByText("GitHub")).toBeInTheDocument();
    expect(within(row("gh-skill")).getByText("Claude Code")).toBeInTheDocument();
    expect(within(row("gh-skill")).getByText("全局")).toBeInTheDocument();
    expect(within(row("beta")).getByText("jev-rpa")).toBeInTheDocument();
    expect(within(row("beta")).getByText("本地")).toBeInTheDocument();
    expect(within(row("plugged")).getByText("superpowers")).toBeInTheDocument();
    expect(within(row("imagegen")).getByText("系统")).toBeInTheDocument();
  });

  it("update badge shown from check results", () => {
    const updates = new Map([
      [gh.id, mkUpdate({ skill_id: gh.id, has_update: true, local_modified: true })],
      [alpha.id, mkUpdate({ skill_id: alpha.id, local_state_unknown: true })],
    ]);
    setup([gh, alpha, zeta], { updates, checkedUpdates: 1 });
    const row = (n: string) => within(screen.getByRole("listbox")).getAllByRole("option").find((o) => within(o).queryByText(n))!;
    expect(within(row("gh-skill")).getByText("有更新")).toBeInTheDocument();
    expect(within(row("gh-skill")).getByTitle("本地有改动")).toBeInTheDocument();
    expect(within(row("zeta")).queryByText("有更新")).toBeNull();
    expect(within(row("alpha")).getByTitle("无法判断本地是否修改，更新时会先确认")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "检查更新（1）" })).toBeInTheDocument();
  });

  it("check button is disabled without git and calls back otherwise", async () => {
    const p = setup([alpha], { gitOk: false });
    const btn = screen.getByRole("button", { name: /检查更新/ });
    expect(btn).toBeDisabled();
    expect(btn.closest("span")).toHaveAttribute("title", "需要安装 git");
    expect(p.onCheck).not.toHaveBeenCalled();
  });

  it("check button and add button call their handlers", async () => {
    const p = setup([alpha]);
    await userEvent.click(screen.getByRole("button", { name: "检查更新" }));
    expect(p.onCheck).toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "添加 skill" }));
    expect(p.onAdd).toHaveBeenCalled();
  });

  it("clicking a row selects it", async () => {
    const p = setup([alpha, zeta]);
    await userEvent.click(screen.getByText("zeta"));
    expect(p.onSelect).toHaveBeenCalledWith(zeta);
  });

  it("two entries with the same id in different scopes both render", () => {
    const a = mkSkill({ name: "dup", id: "x" });
    const b = mkSkill({ name: "dup", id: "x", scope: { type: "project", project_id: "/work/jev-rpa" } });
    setup([a, b]);
    expect(names()).toEqual(["dup", "dup"]);
  });
});
