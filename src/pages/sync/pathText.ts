/** `a/b/c.md` -> `{ name: "c.md", dir: "a/b" }` (`/` or `\` separators). */
export function splitPath(p: string): { name: string; dir: string } {
  const s = p.replace(/\\/g, "/").replace(/\/+$/, "");
  const i = s.lastIndexOf("/");
  return i < 0 ? { name: s, dir: "" } : { name: s.slice(i + 1), dir: s.slice(0, i) };
}

/** Shortens `s` to at most `max` characters by cutting the middle (`head…tail`). */
export function middleEllipsis(s: string, max: number): string {
  if (s.length <= max || max < 5) return s;
  const keep = max - 1;
  const head = Math.ceil(keep / 2);
  const tail = keep - head;
  return `${s.slice(0, head)}…${s.slice(s.length - tail)}`;
}
