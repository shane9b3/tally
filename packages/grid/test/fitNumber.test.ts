import { describe, expect, it } from "vitest";
import { fitNumber, fixed, generalText, isGeneralNumberText, sci } from "../src/fitNumber";

/** Monospace measure: 7 px per character. */
const mono = (s: string): number => s.length * 7;
const fit = (num: number, chars: number, text?: string): string => fitNumber(num, chars * 7, mono, text);

describe("generalText", () => {
  it("prints like General", () => {
    expect(generalText(0)).toBe("0");
    expect(generalText(42)).toBe("42");
    expect(generalText(-1.5)).toBe("-1.5");
    expect(generalText(Math.PI)).toBe("3.141592654");
    expect(generalText(1234567.891)).toBe("1234567.891");
    expect(generalText(1e10)).toBe("1E+10");
    expect(generalText(123456789012)).toBe("1.23456789E+11");
    expect(generalText(0.00001234)).toBe("1.234E-05");
    expect(generalText(Number.NaN)).toBe("###");
  });

  it("helpers", () => {
    expect(fixed(1.5, 3)).toBe("1.5");
    expect(fixed(-0.0001, 2)).toBe("0");
    expect(sci(123456, 2)).toBe("1.23E+05");
    expect(sci(5e-7, 0)).toBe("5E-07");
    expect(sci(1.5e100, 1)).toBe("1.5E+100");
    expect(isGeneralNumberText("-12.5")).toBe(true);
    expect(isGeneralNumberText("1.2E+10")).toBe(true);
    expect(isGeneralNumberText("$1,234.00")).toBe(false);
  });
});

describe("fitNumber", () => {
  it("keeps text that fits", () => {
    expect(fit(3.14159, 10, "3.14159")).toBe("3.14159");
  });

  it("drops decimals first", () => {
    expect(fit(Math.PI, 6)).toBe("3.1416");
    expect(fit(Math.PI, 3)).toBe("3.1");
    expect(fit(Math.PI, 1)).toBe("3");
    expect(fit(1234.5678, 6, "1234.5678")).toBe("1234.6");
    expect(fit(-2.71828, 4, "-2.71828")).toBe("-2.7");
  });

  it("rounds when dropping decimals", () => {
    expect(fit(9.96, 3, "9.96")).toBe("10");
    expect(fit(1.25, 3, "1.25")).toBe("1.3");
  });

  it("switches to scientific when the integer part does not fit", () => {
    expect(fit(123456789, 8, "123456789")).toBe("1.23E+08");
    expect(fit(123456789, 5, "123456789")).toBe("1E+08");
    expect(fit(-123456789, 6, "-123456789")).toBe("-1E+08");
  });

  it("switches to scientific for tiny numbers instead of showing 0", () => {
    expect(fit(0.000123456, 8, "0.000123456")).toBe("0.000123");
    expect(fit(0.000123456, 4, "0.000123456")).not.toBe("0");
    expect(fit(0.000123456, 5, "0.000123456")).toBe("1E-04");
  });

  it("shortens scientific mantissas", () => {
    expect(fit(1.23456789e11, 8, "1.23456789E+11")).toBe("1.23E+11");
  });

  it("falls back to ###", () => {
    expect(fit(123456789, 3, "123456789")).toBe("###");
    expect(fit(Number.POSITIVE_INFINITY, 2, "inf")).toBe("###");
  });

  it("uses the measure function (proportional fonts)", () => {
    const narrowOnes = (s: string): number => [...s].reduce((w, ch) => w + (ch === "1" ? 3 : 8), 0);
    // "1.11111111" is 9 × 3 + 8 = 35 px; "2.222" is 4 × 8 + 8 = 40 px.
    expect(fitNumber(1.11111111, 40, narrowOnes, "1.11111111")).toBe("1.11111111");
    expect(fitNumber(2.22222222, 40, narrowOnes, "2.22222222")).toBe("2.222");
  });
});
