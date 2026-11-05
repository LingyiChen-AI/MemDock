import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const css = readFileSync(resolve(process.cwd(), "src/styles/globals.css"), "utf8");

function rule(selector: string): string {
  const re = new RegExp("(?:^|\\n)\\s*" + selector + "\\s*\\{([^}]*)\\}");
  const m = css.match(re);
  if (!m) throw new Error("rule not found: " + selector);
  return m[1];
}

describe("globals.css type scale", () => {
  it("does not change the rem base on html", () => {
    const html = rule("html");
    expect(html).not.toMatch(/font-size/);
    expect(html).not.toMatch(/line-height/);
  });

  it("keeps 13px/20px body text", () => {
    expect(rule("body")).toMatch(/font-size:\s*13px/);
    expect(rule("body")).toMatch(/line-height:\s*20px/);
  });

  it("defines the design-system type scale (min 12px)", () => {
    expect(css).toMatch(/--text-xs:\s*12px/);
    expect(css).toMatch(/--text-xs--line-height:\s*16px/);
    expect(css).toMatch(/--text-sm:\s*13px/);
    expect(css).toMatch(/--text-sm--line-height:\s*20px/);
    expect(css).toMatch(/--text-base:\s*14px/);
    expect(css).toMatch(/--text-lg:\s*18px/);
    expect(css).toMatch(/--text-2xl:\s*22px/);
  });
});

describe("accent colors", () => {
  const lum = (hex: string) => {
    const c = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255).map((x) =>
      x <= 0.03928 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4,
    );
    return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
  };
  const ratio = (a: string, b: string) => {
    const [hi, lo] = [lum(a), lum(b)].sort((x, y) => y - x);
    return (hi + 0.05) / (lo + 0.05);
  };
  const vars = (body: string) => Object.fromEntries([...body.matchAll(/--([\w-]+):\s*(#[0-9a-f]{6})/gi)].map((m) => [m[1], m[2]]));

  const cases: [string, "light" | "dark", string][] = [];
  for (const a of ["green", "blue", "violet", "orange", "teal"]) {
    cases.push([a, "light", a === "green" ? ":root" : `html\\[data-accent="${a}"\\]`]);
    cases.push([a, "dark", a === "green" ? "\\.dark" : `html\\.dark\\[data-accent="${a}"\\]`]);
  }

  it.each(cases)("%s %s: primary-foreground on primary >= 4.5:1, accent-text readable", (_a, mode, sel) => {
    const v = vars(rule(sel));
    expect(ratio(v["primary"], v["primary-foreground"])).toBeGreaterThanOrEqual(4.5);
    const bg = mode === "dark" ? "#0f172a" : "#f8fafc";
    expect(ratio(v["accent-text"], bg)).toBeGreaterThanOrEqual(4.5);
    // ring: accent-text in dark, primary in light
    expect(v["ring"]).toBe(mode === "dark" ? v["accent-text"] : v["primary"]);
  });
});
