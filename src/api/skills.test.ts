import { describe, it, expect, vi, beforeEach } from "vitest";

const invoke = vi.fn();
const listen = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: (...a: unknown[]) => listen(...a) }));

import {
  installSkill,
  updateSkill,
  listSkills,
  readSkillFile,
  writeSkillFile,
  syncSkill,
  registerSkillSource,
  onSkillsChanged,
} from "./skills";

beforeEach(() => {
  invoke.mockReset();
  listen.mockReset();
});

describe("skills api", () => {
  it("installSkill passes camelCase args", async () => {
    invoke.mockResolvedValue([]);
    await installSkill(
      { type: "template", name: "demo", description: "d" },
      [{ tool: "claude", scope: { type: "global" } }],
      true,
    );
    expect(invoke).toHaveBeenCalledWith("install_skill", {
      from: { type: "template", name: "demo", description: "d" },
      targets: [{ tool: "claude", scope: { type: "global" } }],
      overwrite: true,
    });
  });

  it("updateSkill rethrows local_changes", async () => {
    const err = { code: "local_changes", message: "local changes" };
    invoke.mockRejectedValue(err);
    await expect(updateSkill("/a/skill", false)).rejects.toEqual(err);
    expect(invoke).toHaveBeenCalledWith("update_skill", {
      skillId: "/a/skill",
      acceptLocalChanges: false,
    });
  });

  it("other commands use camelCase argument names", async () => {
    invoke.mockResolvedValue({});
    await listSkills();
    expect(invoke).toHaveBeenLastCalledWith("list_skills");
    await readSkillFile("/s", "ref/a.md");
    expect(invoke).toHaveBeenLastCalledWith("read_skill_file", { skillId: "/s", relPath: "ref/a.md" });
    await writeSkillFile("/s", "SKILL.md", "t", "h");
    expect(invoke).toHaveBeenLastCalledWith("write_skill_file", {
      skillId: "/s",
      relPath: "SKILL.md",
      text: "t",
      expectedHash: "h",
    });
    await syncSkill("/s", { tool: "codex", scope: { type: "project", project_id: "/p" } }, false);
    expect(invoke).toHaveBeenLastCalledWith("sync_skill", {
      skillId: "/s",
      target: { tool: "codex", scope: { type: "project", project_id: "/p" } },
      overwrite: false,
    });
    await registerSkillSource("/s", "https://github.com/a/b", "skills/x");
    expect(invoke).toHaveBeenLastCalledWith("register_skill_source", {
      skillId: "/s",
      url: "https://github.com/a/b",
      skillPath: "skills/x",
    });
  });

  it("onSkillsChanged unwraps the payload", async () => {
    const unlisten = vi.fn();
    listen.mockImplementation(async (_n: string, handler: (e: { payload: unknown }) => void) => {
      handler({ payload: { paths: ["x"] } });
      return unlisten;
    });
    const cb = vi.fn();
    const un = await onSkillsChanged(cb);
    expect(listen).toHaveBeenCalledWith("skills-changed", expect.any(Function));
    expect(cb).toHaveBeenCalledWith({ paths: ["x"] });
    expect(un).toBe(unlisten);
  });
});
