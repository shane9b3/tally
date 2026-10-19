/**
 * The TS lexer must pass the same JSON fixtures as the Rust lexer
 * (crates/tally-formula/tests/fixtures/tokens). Fixture spans are UTF-8
 * byte offsets; they are converted to UTF-16 indices here.
 */
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { tokenize, type Dialect } from "../src/index";

interface Fixture {
  src: string;
  dialect: Dialect;
  tokens: { kind: string; text: string; start: number; end: number }[];
}

const DIR = join(dirname(fileURLToPath(import.meta.url)), "../../../crates/tally-formula/tests/fixtures/tokens");
const files = readdirSync(DIR)
  .filter((f) => f.endsWith(".json"))
  .sort();

/** Maps every UTF-8 byte offset at a char boundary (and the end) to its UTF-16 index. */
function byteToUtf16(src: string): Map<number, number> {
  const map = new Map<number, number>();
  let bytes = 0;
  let i = 0;
  for (const ch of src) {
    map.set(bytes, i);
    bytes += new TextEncoder().encode(ch).length;
    i += ch.length;
  }
  map.set(bytes, i);
  return map;
}

describe("shared lexer fixtures", () => {
  it("finds all fixtures", () => {
    expect(files.length).toBeGreaterThanOrEqual(307);
  });

  it.each(files)("%s", (file) => {
    const fx = JSON.parse(readFileSync(join(DIR, file), "utf8")) as Fixture;
    const map = byteToUtf16(fx.src);
    const at = (b: number): number => {
      const v = map.get(b);
      if (v === undefined) throw new Error(`byte offset ${b} is not a char boundary`);
      return v;
    };
    const expected = fx.tokens.map((t) => ({ kind: t.kind, text: t.text, start: at(t.start), end: at(t.end) }));
    expect(tokenize(fx.src, fx.dialect)).toEqual(expected);
  });
});
