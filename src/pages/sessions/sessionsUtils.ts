import type { SessionSummary, Tool } from "@/api/types";

export type ToolFilter = "all" | Tool;

export interface ProjectGroup {
  projectId: string;
  name: string;
  cwd: string;
  sessions: SessionSummary[];
}

export function otherTool(tool: Tool): Tool {
  return tool === "claude" ? "codex" : "claude";
}

/** Case-insensitive match on title, first prompt, project, cwd and id; every word must match. */
export function filterSessions(list: SessionSummary[], tool: ToolFilter, query: string): SessionSummary[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  return list.filter((s) => {
    if (tool !== "all" && s.tool !== tool) return false;
    if (words.length === 0) return true;
    const hay = [s.title ?? "", s.first_prompt, s.project_name, s.cwd, s.id].join("\n").toLowerCase();
    return words.every((w) => hay.includes(w));
  });
}

/** Groups by project, keeping the input order inside a group; groups follow their newest session. */
export function groupByProject(list: SessionSummary[]): ProjectGroup[] {
  const groups = new Map<string, ProjectGroup>();
  for (const s of list) {
    let g = groups.get(s.project_id);
    if (!g) {
      g = { projectId: s.project_id, name: s.project_name, cwd: s.cwd, sessions: [] };
      groups.set(s.project_id, g);
    }
    g.sessions.push(s);
  }
  const newest = (g: ProjectGroup) => Math.max(...g.sessions.map((s) => s.updated_at ?? 0));
  return [...groups.values()].sort((a, b) => newest(b) - newest(a));
}

/** Same format as `convert::resume_command` on the Rust side. */
export function resumeCommand(tool: Tool, cwd: string, id: string): string {
  // Windows PowerShell 5 has no `&&`; `;` works in PowerShell 5 and 7.
  const sep = /Windows/i.test(navigator.userAgent) ? ";" : " &&";
  const cd = cwd ? `cd "${cwd}"${sep} ` : "";
  return tool === "claude" ? `${cd}claude --resume ${id}` : `${cd}codex resume ${id}`;
}

function pad(n: number): string {
  return String(n).padStart(2, "0");
}

/** `HH:mm` today, `MM-DD HH:mm` this year, else `YYYY-MM-DD`. */
export function formatTime(ms: number | null, now: Date = new Date()): string {
  if (ms == null) return "";
  const d = new Date(ms);
  const hm = `${pad(d.getHours())}:${pad(d.getMinutes())}`;
  if (d.toDateString() === now.toDateString()) return hm;
  if (d.getFullYear() === now.getFullYear()) return `${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${hm}`;
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** Same as `formatTime`, for RFC 3339 strings. */
export function formatIso(ts: string, now: Date = new Date()): string {
  const ms = Date.parse(ts);
  return Number.isNaN(ms) ? "" : formatTime(ms, now);
}
