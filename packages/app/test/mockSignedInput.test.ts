import { describe, expect, it } from "vitest";
import { signedFormulaInput } from "../src/dev/mockSignedInput";

const ctx = { isName: (n: string) => n.toLowerCase() === "rate", isFunction: (n: string) => ["SUM", "TRUE"].includes(n) };

describe("signedFormulaInput (P18, mock parity with the engine)", () => {
  it("turns signed non-numbers into formulas", () => {
    expect(signedFormulaInput("+B1+B2", ctx)).toBe("=+B1+B2");
    expect(signedFormulaInput("+A1", ctx)).toBe("=+A1");
    expect(signedFormulaInput("-A1*2", ctx)).toBe("=-A1*2");
    expect(signedFormulaInput("+SUM(A1:A3)", ctx)).toBe("=+SUM(A1:A3)");
    expect(signedFormulaInput("-Rate", ctx)).toBe("=-Rate");
    expect(signedFormulaInput("+12/31/2024", ctx)).toBe("=+12/31/2024");
  });

  it("leaves numbers, lone signs, unknown names and broken input alone", () => {
    for (const s of ["-5", "+5", "-5%", "-12:30", "+1:02:03", "+1e3", "-2020", "- 5", "--5", "+-5", "-", "+", "+-", "-abc", "+foo(1)", "+1)", "+B1+", "+SUM(A1", "B1+B2", "'+B1"]) {
      expect(signedFormulaInput(s, ctx), s).toBeNull();
    }
  });
});
