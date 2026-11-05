import { readFileSync } from "node:fs";
import { readdirSync, statSync } from "node:fs";
import { join, resolve } from "node:path";

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

describe("accent swatches and raw hex guard", () => {
  const vars = (body: string) => Object.fromEntries([...body.matchAll(/--([\w-]+):\s*(#[0-9a-f]{6})/gi)].map((m) => [m[1], m[2]]));
  const accents = ["green", "blue", "violet", "orange", "teal"];

  it.each(accents)("--swatch-%s equals that accent's primary in light and dark", (a) => {
    const light = vars(rule(a === "green" ? ":root" : `html\\[data-accent="${a}"\\]`));
    const dark = vars(rule(a === "green" ? "\\.dark" : `html\\.dark\\[data-accent="${a}"\\]`));
    expect(vars(rule(":root"))[`swatch-${a}`]).toBe(light["primary"]);
    expect(vars(rule("\\.dark"))[`swatch-${a}`]).toBe(dark["primary"]);
  });

  it("no raw hex colors in non-test tsx components", () => {
    const files: string[] = [];
    const walk = (dir: string) => {
      for (const name of readdirSync(dir)) {
        const p = join(dir, name);
        if (statSync(p).isDirectory()) walk(p);
        else if (p.endsWith(".tsx") && !p.endsWith(".test.tsx") && !p.includes(join("src", "dev"))) files.push(p);
      }
    };
    walk(resolve(process.cwd(), "src"));
    const offenders = files.filter((f) => /#[0-9a-f]{3,8}\b/i.test(readFileSync(f, "utf8")));
    expect(offenders).toEqual([]);
  });
});

describe("scrollbars", () => {
  const block = (sel: string) => {
    const i = css.indexOf(sel + " {");
    if (i < 0) throw new Error("rule not found: " + sel);
    return css.slice(i, css.indexOf("}", i));
  };

  it("are thin, rounded and themed with semantic variables only", () => {
    expect(block("::-webkit-scrollbar")).toMatch(/width:\s*8px/);
    expect(block("::-webkit-scrollbar")).toMatch(/height:\s*8px/);
    const thumb = block("::-webkit-scrollbar-thumb");
    expect(thumb).toMatch(/color-mix\(in srgb, var\(--muted-foreground\) 35%, transparent\)/);
    expect(thumb).toMatch(/background-clip:\s*padding-box/);
    expect(thumb).toMatch(/border-radius/);
    expect(block("::-webkit-scrollbar-thumb:hover")).toMatch(/var\(--muted-foreground\) 55%/);
    expect(block("::-webkit-scrollbar-button")).toMatch(/display:\s*none/);
    for (const sel of ["::-webkit-scrollbar", "::-webkit-scrollbar-thumb", "::-webkit-scrollbar-thumb:hover"])
      expect(block(sel)).not.toMatch(/#[0-9a-f]{3,8}\b/i);
  });

  it("only uses scrollbar-width/color where ::-webkit-scrollbar is unsupported", () => {
    // In Chromium (WebView2) these standard properties would disable the ::-webkit-scrollbar styling.
    const idx = css.indexOf("scrollbar-width: thin");
    expect(idx).toBeGreaterThan(-1);
    expect(css.lastIndexOf("@supports not selector(::-webkit-scrollbar)", idx)).toBeGreaterThan(-1);
  });
});
