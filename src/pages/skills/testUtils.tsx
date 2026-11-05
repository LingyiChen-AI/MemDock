import type { SkillEntry, UpdateStatus } from "@/api/types";

/** Test/preview helper: a complete SkillEntry with sensible defaults. */
export function mkSkill(over: Partial<SkillEntry> & { name: string }): SkillEntry {
  return {
    id: `c:/users/u/.claude/skills/${over.name}`,
    description: `${over.name} description`,
    tool: "claude",
    scope: { type: "global" },
    kind: "user",
    dir: `C:\\Users\\u\\.claude\\skills\\${over.name}`,
    is_symlink: false,
    symlink_target: null,
    readonly: false,
    plugin: null,
    source: null,
    file_count: 3,
    size_bytes: 2048,
    modified_at: 1_700_000_000_000,
    ...over,
  };
}

export function mkUpdate(over: Partial<UpdateStatus> & { skill_id: string }): UpdateStatus {
  return {
    current: "aaaaaaa1111",
    latest: "bbbbbbb2222",
    has_update: false,
    local_modified: false,
    local_state_unknown: false,
    error: null,
    ...over,
  };
}
