import { describe, expect, it } from "vitest";
import { CURRENCY_FORMAT } from "../src/formats";
import { formatNumberLite, isDateCode, splitSections } from "../src/numfmtLite";

describe("formatNumberLite", () => {
  it("formats the toolbar codes", () => {
    expect(formatNumberLite(1234.5, "General")).toBe("1234.5");
    expect(formatNumberLite(1234.5, "0")).toBe("1235");
    expect(formatNumberLite(1234.5, "0.00")).toBe("1234.50");
    expect(formatNumberLite(1234.5, "#,##0.00")).toBe("1,234.50");
    expect(formatNumberLite(1234567, "#,##0")).toBe("1,234,567");
    expect(formatNumberLite(0.256, "0.00%")).toBe("25.60%");
    expect(formatNumberLite(0.5, "0%")).toBe("50%");
    expect(formatNumberLite(12345, "0.00E+00")).toBe("1.23E+04");
    expect(formatNumberLite(686.1, CURRENCY_FORMAT)).toBe("$686.10");
    expect(formatNumberLite(-5, CURRENCY_FORMAT)).toBe("-$5.00");
    expect(formatNumberLite(-5, "0.00")).toBe("-5.00");
    expect(formatNumberLite(3, '0" units"')).toBe("3 units");
  });

  it("formats dates and times from serials", () => {
    // 2026-10-19 is serial 46314.
    expect(formatNumberLite(46314, "MM/DD/YYYY")).toBe("10/19/2026");
    expect(formatNumberLite(46314, "YYYY-MM-DD")).toBe("2026-10-19");
    expect(formatNumberLite(46314, "DD MMM YYYY")).toBe("19 Oct 2026");
    expect(formatNumberLite(0.5 + 1 / 1440, "HH:MM")).toBe("12:01");
    expect(formatNumberLite(0.75, "HH:MM:SS")).toBe("18:00:00");
    expect(formatNumberLite(0.75, "H:MM AM/PM")).toBe("6:00 PM");
  });

  it("splits sections and detects dates", () => {
    expect(splitSections('0;"a;b";0')).toEqual(["0", '"a;b"', "0"]);
    expect(isDateCode("MM/DD/YYYY")).toBe(true);
    expect(isDateCode("#,##0.00")).toBe(false);
    expect(isDateCode(CURRENCY_FORMAT)).toBe(false);
  });
});
