import { installSkill } from "@/api/skills";
import type { ManifestSkill, SkillEntry, SkillScope } from "@/api/types";

/** Installs one manifest entry from its git source (pinned to its commit) into `scope`. */
export function installFromManifest(m: ManifestSkill, scope: SkillScope): Promise<SkillEntry[]> {
  return installSkill(
    { type: "git", url: m.url, clone_url: m.url, git_ref: m.git_ref, skill_path: m.skill_path, commit: m.commit },
    [{ tool: m.tool, scope }],
    false,
  );
}
