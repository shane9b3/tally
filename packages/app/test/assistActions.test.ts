import { describe, expect, it, vi } from "vitest";
import { columnIndex, parseA1Range } from "../src/a1";
import { currencyTarget } from "../src/commands";
import { CommandRegistry } from "../src/registry";

describe("assist action arguments", () => {
  it("parses A1 ranges and columns", () => {
    expect(columnIndex("A")).toBe(0);
    expect(columnIndex("g")).toBe(6);
    expect(columnIndex("AB")).toBe(27);
    expect(columnIndex("A1")).toBeNull();
    expect(parseA1Range("F2:F15")).toEqual({ r0: 1, c0: 5, r1: 14, c1: 5 });
    expect(parseA1Range("$B$7")).toEqual({ r0: 6, c0: 1, r1: 6, c1: 1 });
    expect(parseA1Range("C5:A1")).toEqual({ r0: 0, c0: 0, r1: 4, c1: 2 });
    expect(parseA1Range("F2:F15:G1")).toBeNull();
    expect(parseA1Range("nope")).toBeNull();
  });

  it("targets the column body for format.currency", () => {
    const sel = { r0: 3, c0: 0, r1: 4, c1: 0 };
    expect(currencyTarget({ column: "F", range: "F2:F15" }, sel)).toEqual({ r0: 1, c0: 5, r1: 14, c1: 5 });
    expect(currencyTarget({ column: "F" }, sel)).toEqual({ r0: 3, c0: 5, r1: 4, c1: 5 });
    expect(currencyTarget({}, sel)).toBeNull();
    expect(currencyTarget(undefined, sel)).toBeNull();
  });

  it("passes args through the registry", () => {
    const reg = new CommandRegistry();
    const run = vi.fn();
    reg.register({ id: "format.currency", title: "Currency", kind: "Format", run });
    expect(reg.run("format.currency", { range: "F2:F15" })).toBe(true);
    expect(run).toHaveBeenCalledWith({ range: "F2:F15" });
    expect(reg.run("missing")).toBe(false);
  });
});
