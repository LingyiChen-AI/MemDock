import { installManifestPlugin, installSkill } from "@/api/skills";
import type { ManifestSkill, SkillScope } from "@/api/types";

export const isLocalOnly = (m: ManifestSkill) => Boolean(m.local_only) || (!m.plugin && !m.url);

/** Restore a standalone skill or an entire marketplace plugin into the mapped scope. */
export async function installFromManifest(m: ManifestSkill, scope: SkillScope): Promise<void> {
  if (m.plugin) {
    await installManifestPlugin(m.tool, m.plugin.key, scope, Boolean(m.plugin.local_scope));
    return;
  }
  await installSkill(
    { type: "git", url: m.url, clone_url: m.url, git_ref: m.git_ref, skill_path: m.skill_path, commit: m.commit },
    [{ tool: m.tool, scope }],
    false,
  );
}
