/**
 * App-wide hover tooltips in the app's own style (dark rounded bubble, like the Radix tooltips):
 * - any element with a native `title` shows it in the bubble instead of the browser's grey box
 *   (the title is moved to `data-tip` on first hover so the native one never appears);
 * - text cut off with an ellipsis (`truncate` / `text-overflow: ellipsis` / `line-clamp`) and without a
 *   tooltip of its own shows its full text.
 * One delegated listener and one reusable bubble element; nothing to add per component.
 */
const TIP = "data-tip";
const AUTO = "data-tip-auto";
const DELAY = 450;
const GAP = 8;

let bubble: HTMLDivElement | null = null;
let timer: number | undefined;
let current: HTMLElement | null = null;

function ensureBubble(): HTMLDivElement {
  if (bubble && document.body.contains(bubble)) return bubble;
  bubble = document.createElement("div");
  bubble.setAttribute("role", "tooltip");
  bubble.className =
    "pointer-events-none fixed z-[1000] max-w-sm rounded-md bg-foreground px-3 py-1.5 text-xs leading-5 whitespace-pre-wrap text-background shadow-md opacity-0 transition-opacity duration-100 [overflow-wrap:anywhere]";
  bubble.style.left = "-9999px";
  document.body.appendChild(bubble);
  return bubble;
}

function hide() {
  window.clearTimeout(timer);
  current = null;
  if (bubble) {
    bubble.style.opacity = "0";
    bubble.style.left = "-9999px";
  }
}

function place(el: HTMLElement, text: string) {
  const b = ensureBubble();
  b.textContent = text;
  b.style.opacity = "0";
  b.style.left = "0px";
  b.style.top = "0px";
  const r = el.getBoundingClientRect();
  const bw = b.offsetWidth;
  const bh = b.offsetHeight;
  const vw = window.innerWidth;
  const vh = window.innerHeight;
  let top = r.bottom + GAP;
  if (top + bh > vh - 4) top = Math.max(4, r.top - GAP - bh); // flip above near the bottom edge
  let left = r.left + r.width / 2 - bw / 2;
  left = Math.min(Math.max(4, left), vw - bw - 4);
  b.style.left = `${Math.round(left)}px`;
  b.style.top = `${Math.round(top)}px`;
  b.style.opacity = "1";
}

function clipped(el: HTMLElement): boolean {
  const s = getComputedStyle(el);
  const hidden = [s.overflow, s.overflowX, s.overflowY].some((v) => v === "hidden" || v === "clip");
  if (!hidden) return false;
  return s.textOverflow === "ellipsis" || (s.webkitLineClamp !== "none" && s.webkitLineClamp !== "") || el.classList.contains("truncate");
}

function overflowing(el: HTMLElement): boolean {
  return el.scrollWidth > el.clientWidth + 1 || el.scrollHeight > el.clientHeight + 1;
}

/** The element to show a tip for, and its text. */
function tipFor(start: HTMLElement | null): { el: HTMLElement; text: string } | null {
  // An explicit title anywhere up the tree wins.
  for (let el = start; el && el !== document.body; el = el.parentElement) {
    // Radix tooltip triggers handle themselves.
    if (el.hasAttribute("data-state") && el.hasAttribute("data-slot") && el.getAttribute("data-slot") === "tooltip-trigger") return null;
    const title = el.getAttribute("title");
    if (title) {
      el.setAttribute(TIP, title);
      el.removeAttribute("title");
    }
    const tip = el.getAttribute(TIP);
    if (tip && !el.hasAttribute(AUTO)) return { el, text: tip };
  }
  // Otherwise: the nearest cut-off text (a few levels up).
  let el = start;
  for (let depth = 0; el && depth < 4; depth++, el = el.parentElement) {
    if (!clipped(el)) continue;
    const text = (el.textContent ?? "").replace(/\s+/g, " ").trim();
    if (text && overflowing(el)) return { el, text };
    return null;
  }
  return null;
}

function onOver(e: Event) {
  const found = tipFor(e.target instanceof HTMLElement ? e.target : null);
  if (!found) {
    if (current && !(e.target instanceof Node && current.contains(e.target))) hide();
    return;
  }
  if (found.el === current) return;
  hide();
  current = found.el;
  timer = window.setTimeout(() => {
    if (current === found.el && document.body.contains(found.el)) place(found.el, found.text);
  }, DELAY);
}

function onOut(e: Event) {
  const to = (e as PointerEvent).relatedTarget;
  if (current && !(to instanceof Node && current.contains(to))) hide();
}

export function installOverflowTitles(root: Document = document): () => void {
  root.addEventListener("pointerover", onOver, { passive: true });
  root.addEventListener("pointerout", onOut, { passive: true });
  root.addEventListener("pointerdown", hide, { passive: true });
  window.addEventListener("scroll", hide, { passive: true, capture: true });
  window.addEventListener("blur", hide);
  return () => {
    root.removeEventListener("pointerover", onOver);
    root.removeEventListener("pointerout", onOut);
    root.removeEventListener("pointerdown", hide);
    window.removeEventListener("scroll", hide, { capture: true });
    window.removeEventListener("blur", hide);
    hide();
  };
}
