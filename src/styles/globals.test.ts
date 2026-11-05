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
