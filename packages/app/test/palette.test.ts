import { describe, expect, it } from "vitest";
import { chipColorsFor, CHIPS, hexToRgba, rgbaToHex, SWATCHES } from "../src/palette";
import { nextThemePref, resolveTheme } from "../src/theme/theme";

describe("palette", () => {
  it("converts colours", () => {
    expect(hexToRgba("#dc2626")).toBe(0xdc2626ff);
    expect(hexToRgba("dc2626")).toBe(0xdc2626ff);
    expect(hexToRgba("#xyz")).toBeNull();
    expect(rgbaToHex(0x0a0b0cff)).toBe("#0a0b0c");
    expect(SWATCHES).toHaveLength(24);
  });

  it("gives status words their usual chip and cycles the rest", () => {
    const m = chipColorsFor(["Paid", "Due", "Overdue", "North", "South"]);
    expect(m.get("Paid")?.name).toBe("green");
    expect(m.get("Due")?.name).toBe("amber");
    expect(m.get("Overdue")?.name).toBe("red");
    expect(m.get("North")?.name).toBe("blue");
    expect(m.get("South")?.name).toBe("purple");
    const many = chipColorsFor(["a", "b", "c", "d", "e", "f", "g"]);
    expect([...many.values()].map((c) => c.name)).toEqual([...CHIPS.map((c) => c.name), "green"]);
  });
});

describe("theme preference", () => {
  it("cycles system → light → dark and resolves system", () => {
    expect(nextThemePref("system")).toBe("light");
    expect(nextThemePref("light")).toBe("dark");
    expect(nextThemePref("dark")).toBe("system");
    expect(resolveTheme("system", "dark")).toBe("dark");
    expect(resolveTheme("light", "dark")).toBe("light");
  });
});

describe("recent files", async () => {
  const { displayName } = await import("../src/components/Sidebar");
  it("derives display names from paths", () => {
    expect(displayName("/Users/me/Documents/Budget 2026.ods")).toBe("Budget 2026");
    expect(displayName("C:\\\\Data\\\\orders.xlsx")).toBe("orders");
    expect(displayName("notes.txt")).toBe("notes.txt");
  });
});
