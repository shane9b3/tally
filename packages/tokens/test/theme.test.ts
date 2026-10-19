import { describe, expect, it } from "vitest";
import { DEFAULT_THEME, readTheme, THEME_VARIABLES, type StyleSource } from "../src/index";
import css from "../src/tokens.css?raw";

/** Extracts the declarations of the first `:root` block. */
function lightVars(): Map<string, string> {
  const block = /:root,\s*\[data-theme="light"\]\s*\{([^}]*)\}/.exec(css)?.[1] ?? "";
  const vars = new Map<string, string>();
  for (const m of block.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) {
    if (m[1] && m[2]) vars.set(m[1], m[2].trim());
  }
  return vars;
}

/** A StyleSource that resolves one level of var() like the browser would. */
function sourceFrom(vars: Map<string, string>): StyleSource {
  const resolve = (v: string): string =>
    v.replace(/var\((--[\w-]+)\)/g, (_, name: string) => resolve(vars.get(name) ?? ""));
  return { getPropertyValue: (name) => resolve(vars.get(name) ?? "") };
}

const allVariables = (): string[] =>
  Object.values(THEME_VARIABLES).flatMap((group) => Object.values(group) as string[]);

describe("tokens.css", () => {
  it("defines every variable the Theme reads, in light and dark", () => {
    const dark = css.slice(css.indexOf('[data-theme="dark"] {'));
    for (const name of allVariables()) {
      expect(lightVars().has(name), name).toBe(true);
      expect(dark.includes(`${name}:`), `${name} (dark)`).toBe(true);
    }
  });

  it("light values agree with DEFAULT_THEME", () => {
    expect(readTheme(sourceFrom(lightVars()))).toEqual(DEFAULT_THEME);
  });
});

describe("readTheme", () => {
  it("parses pixel values and falls back to defaults", () => {
    const vars = new Map([
      ["--tally-font-size", " 14px"],
      ["--tally-accent", "#ff0000"],
      ["--tally-radius-card", "oops"],
    ]);
    const theme = readTheme(sourceFrom(vars));
    expect(theme.fonts.sizePx).toBe(14);
    expect(theme.colors.accent).toBe("#ff0000");
    expect(theme.radii.card).toBe(DEFAULT_THEME.radii.card);
    expect(theme.colors.bg).toBe(DEFAULT_THEME.colors.bg);
  });
});

describe("dark theme", () => {
  /** Declarations of a block starting at `marker` (up to its first closing brace). */
  function block(marker: string): Map<string, string> {
    const start = css.indexOf(marker);
    const body = css.slice(start + marker.length, css.indexOf("}", start));
    const vars = new Map<string, string>();
    for (const m of body.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) if (m[1] && m[2]) vars.set(m[1], m[2].trim());
    return vars;
  }

  it("has no placeholders and covers every light variable", () => {
    const dark = block('[data-theme="dark"] {');
    expect(css).not.toMatch(/TODO/);
    for (const name of lightVars().keys()) expect(dark.has(name), name).toBe(true);
  });

  it("is also the prefers-color-scheme default, with identical values", () => {
    const dark = block('[data-theme="dark"] {');
    const media = block(":root:not([data-theme]) {");
    expect([...media.entries()]).toEqual([...dark.entries()]);
  });

  it("dark canvas colours differ from light where it matters", () => {
    const dark = block('[data-theme="dark"] {');
    const resolve = (v: string): string => v.replace(/var\((--[\w-]+)\)/g, (_, n: string) => resolve(dark.get(n) ?? ""));
    const theme = readTheme({ getPropertyValue: (n) => resolve(dark.get(n) ?? "") });
    expect(theme.colors.bg).toBe("#16181c");
    expect(theme.colors.text).not.toBe(DEFAULT_THEME.colors.text);
    expect(theme.colors.gridLine).not.toBe(DEFAULT_THEME.colors.gridLine);
  });
});
