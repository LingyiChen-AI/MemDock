// Splits diff3 output ("<<<<<<< local" / "||||||| base" / "=======" / ">>>>>>> remote")
// into plain text and conflict blocks, and joins chosen sides back into one text.

export type Block =
  | { kind: "text"; text: string }
  | { kind: "conflict"; local: string; base: string; remote: string };

export type Choice = "local" | "remote" | "both" | { custom: string };

/** Lines including their line break. */
function splitLines(text: string): string[] {
  const out: string[] = [];
  let start = 0;
  for (let i = 0; i < text.length; i++) {
    if (text[i] === "\n") {
      out.push(text.slice(start, i + 1));
      start = i + 1;
    }
  }
  if (start < text.length) out.push(text.slice(start));
  return out;
}

const bare = (line: string) => line.replace(/\r?\n$/, "");

export function parse(text: string): Block[] {
  const blocks: Block[] = [];
  let plain = "";
  let section: "local" | "base" | "remote" | null = null;
  let cur = { local: "", base: "", remote: "" };
  for (const line of splitLines(text)) {
    const l = bare(line);
    if (section === null) {
      if (l.startsWith("<<<<<<<")) {
        if (plain) blocks.push({ kind: "text", text: plain });
        plain = "";
        section = "local";
        cur = { local: "", base: "", remote: "" };
      } else plain += line;
      continue;
    }
    if (section === "local" && l.startsWith("|||||||")) section = "base";
    else if ((section === "local" || section === "base") && l === "=======") section = "remote";
    else if (section === "remote" && l.startsWith(">>>>>>>")) {
      blocks.push({ kind: "conflict", ...cur });
      section = null;
    } else cur[section] += line;
  }
  if (section !== null) {
    // Unterminated block: keep what was read as plain text so nothing is lost.
    plain += cur.local + cur.base + cur.remote;
  }
  if (plain) blocks.push({ kind: "text", text: plain });
  return blocks;
}

/** Number of conflict blocks. */
export function conflictCount(blocks: Block[]): number {
  return blocks.filter((b) => b.kind === "conflict").length;
}

/** Choices are keyed by the conflict's ordinal (0-based); `null` while any block is unchosen. */
export function render(blocks: Block[], choices: Record<number, Choice | undefined>): string | null {
  let out = "";
  let n = 0;
  for (const b of blocks) {
    if (b.kind === "text") {
      out += b.text;
      continue;
    }
    const c = choices[n++];
    if (c === undefined) return null;
    if (c === "local") out += b.local;
    else if (c === "remote") out += b.remote;
    else if (c === "both") out += b.local + (b.local && !b.local.endsWith("\n") ? "\n" : "") + b.remote;
    else out += c.custom;
  }
  return out;
}

/** Same rule as the backend: a line starting with a conflict marker. */
export function hasMarkers(text: string): boolean {
  return text.split("\n").some((l) => l.startsWith("<<<<<<<") || l.startsWith(">>>>>>>") || l.startsWith("|||||||"));
}
