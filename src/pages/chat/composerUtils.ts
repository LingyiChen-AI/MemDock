/** Helpers of the chat composer: path insertion, image detection, drop hit-testing. */

/** Largest image (pasted, picked or dropped) the composer attaches. */
export const MAX_IMAGE_BYTES = 5 * 1024 * 1024;

export const IMAGE_EXTS = ["png", "jpg", "jpeg", "gif", "webp"];

export function isImagePath(path: string): boolean {
  const m = /\.([^./\\]+)$/.exec(path);
  return !!m && IMAGE_EXTS.includes(m[1].toLowerCase());
}

function slashes(p: string): string {
  return p.replace(/\\/g, "/");
}

/** Windows paths (drive letter or UNC) compare case-insensitively. */
function isWindowsPath(p: string): boolean {
  return /^[a-zA-Z]:\//.test(p) || p.startsWith("//");
}

/**
 * The text inserted for a dropped / picked file: relative to `cwd` (forward slashes) when the
 * file is inside it, otherwise the absolute path as given; quoted when it contains whitespace.
 */
export function formatInsertPath(path: string, cwd: string): string {
  let out = path;
  const p = slashes(path);
  const base = slashes(cwd).replace(/\/+$/, "");
  if (base) {
    const ci = isWindowsPath(p) || isWindowsPath(base);
    const pp = ci ? p.toLowerCase() : p;
    const bb = ci ? base.toLowerCase() : base;
    if (pp === bb) out = ".";
    else if (pp.startsWith(bb + "/")) out = p.slice(base.length + 1);
  }
  return /\s/.test(out) ? `"${out}"` : out;
}

/**
 * Inserts `insert` over the selection `[start, end)`, padding it with a space where it would
 * otherwise touch neighbouring text. Returns the new text and the caret position after it.
 */
export function insertAtCursor(text: string, start: number, end: number, insert: string): { text: string; caret: number } {
  const before = text.slice(0, start);
  const after = text.slice(end);
  const lead = before && !/\s$/.test(before) ? " " : "";
  const trail = after && !/^\s/.test(after) ? " " : "";
  const piece = lead + insert + trail;
  return { text: before + piece + after, caret: before.length + lead.length + insert.length };
}

/** Whether a drag position (physical pixels, as Tauri reports it) lies inside `rect` (CSS pixels). */
export function inRect(pos: { x: number; y: number }, rect: { left: number; top: number; right: number; bottom: number }, dpr: number): boolean {
  const x = pos.x / (dpr || 1);
  const y = pos.y / (dpr || 1);
  return x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom;
}

/** Reads a File (pasted image) as a data URI. */
export function fileToDataUri(file: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const r = new FileReader();
    r.onload = () => resolve(String(r.result));
    r.onerror = () => reject(r.error ?? new Error("read failed"));
    r.readAsDataURL(file);
  });
}
