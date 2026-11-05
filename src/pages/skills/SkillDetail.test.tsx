import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn().mockResolvedValue({ language: "zh", theme: null, accent: null, manual_projects: [] }),
  updateUiPrefs: vi.fn(),
}));
vi.mock("@/api/skills", () => ({
  registerSkillSource: vi.fn(),
  listSkillFiles: vi.fn().mockResolvedValue({ files: [], truncated: false }),
  readSkillFile: vi.fn(),
  writeSkillFile: vi.fn(),
  onSkillsChanged: vi.fn().mockResolvedValue(() => {}),
}));

import { registerSkillSource } from "@/api/skills";
import { renderApp } from "@/test/fixtures";
import { SkillDetail } from "./SkillDetail";
import { mkSkill, mkUpdate } from "./testUtils";

const local = mkSkill({ name: "alpha" });
const ro = mkSkill({ name: "plugged", kind: "plugin", plugin: "superpowers", readonly: true });
const withSource = mkSkill({
  name: "gh-skill",
  source: { url: "https://github.com/o/r", git_ref: null, commit: "abc1234567890", skill_path: "skills/x", origin: "registry" },
  is_symlink: true,
  symlink_target: "C:\\repo\\gh-skill",
});

function setup(entry = local, over: Record<string, unknown> = {}) {
  const props = {
    entry,
    installs: [entry],
    projectName: (id: string) => id,
    update: undefined,
    onUpdate: vi.fn(),
    onSync: vi.fn(),
    onDelete: vi.fn(),
    onBackups: vi.fn(),
    onChanged: vi.fn(),
    ...over,
  };
  renderApp(<SkillDetail {...props} />);
  return props;
}

beforeEach(() => vi.clearAllMocks());

describe("SkillDetail", () => {
  it("writable detail shows all actions and wires the handlers", async () => {
    const p = setup();
    for (const n of ["更新", "编辑文件", "同步到…", "删除"]) {
      expect(screen.getByRole("button", { name: n })).toBeInTheDocument();
    }
    await userEvent.click(screen.getByRole("button", { name: "更新" }));
    expect(p.onUpdate).toHaveBeenCalledWith(local);
    await userEvent.click(screen.getByRole("button", { name: "同步到…" }));
    expect(p.onSync).toHaveBeenCalledWith(local);
    await userEvent.click(screen.getByRole("button", { name: "删除" }));
    expect(p.onDelete).toHaveBeenCalledWith(local);
  });

  it("readonly detail hides edit and delete", () => {
    setup(ro);
    expect(screen.queryByRole("button", { name: "编辑文件" })).toBeNull();
    expect(screen.queryByRole("button", { name: "删除" })).toBeNull();
    expect(screen.queryByRole("button", { name: "更新" })).toBeNull();
    expect(screen.queryByRole("button", { name: "同步到…" })).toBeNull();
    expect(screen.getByRole("button", { name: "复制到可写位置" })).toBeInTheDocument();
  });

  it("shows source url, short commit, install locations and symlink target", () => {
    const other = mkSkill({ name: "gh-skill", id: "other", tool: "codex", dir: "C:\\Users\\u\\.codex\\skills\\gh-skill" });
    setup(withSource, { installs: [withSource, other] });
    expect(screen.getByText("https://github.com/o/r")).toBeInTheDocument();
    expect(screen.getByText("abc1234")).toBeInTheDocument();
    expect(screen.getByText(withSource.dir)).toBeInTheDocument();
    expect(screen.getByText(other.dir)).toBeInTheDocument();
    expect(screen.getByText("→ C:\\repo\\gh-skill")).toBeInTheDocument();
    expect(screen.getByText("3 个文件 · 2.0 KB")).toBeInTheDocument();
  });

  it("shows latest commit and update info", () => {
    setup(withSource, { update: mkUpdate({ skill_id: withSource.id, has_update: true, latest: "fffffff9999" }) });
    expect(screen.getByText("最新 fffffff")).toBeInTheDocument();
  });

  it("unknown source shows register button", () => {
    setup();
    expect(screen.getByText("来源未知")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "补登来源" })).toBeInTheDocument();
  });

  it("known source does not show register button", () => {
    setup(withSource);
    expect(screen.queryByRole("button", { name: "补登来源" })).toBeNull();
  });

  it("register source calls API with url and path", async () => {
    (registerSkillSource as import("vitest").Mock).mockResolvedValue(withSource);
    const p = setup();
    await userEvent.click(screen.getByRole("button", { name: "补登来源" }));
    await userEvent.type(screen.getByLabelText("GitHub 地址"), "https://github.com/o/r");
    await userEvent.type(screen.getByLabelText("仓库内路径"), "skills/alpha");
    await userEvent.click(screen.getByRole("button", { name: "登记" }));
    expect(registerSkillSource).toHaveBeenCalledWith(local.id, "https://github.com/o/r", "skills/alpha");
    await vi.waitFor(() => expect(p.onChanged).toHaveBeenCalled());
  });

  it("register source failure shows the message and keeps the dialog", async () => {
    (registerSkillSource as import("vitest").Mock).mockRejectedValue({ code: "invalid", message: "bad url" });
    setup();
    await userEvent.click(screen.getByRole("button", { name: "补登来源" }));
    await userEvent.type(screen.getByLabelText("GitHub 地址"), "nope");
    await userEvent.click(screen.getByRole("button", { name: "登记" }));
    expect(await screen.findByText("登记失败：bad url")).toBeInTheDocument();
  });
});
