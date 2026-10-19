import { describe, expect, it } from "vitest";
import { pretty } from "../src/pretty";

describe("pretty", () => {
  it("spaces multiplication as ×", () => {
    expect(pretty("=A1*B1")).toBe("=A1 × B1");
    expect(pretty("=D6 * E6")).toBe("=D6 × E6");
    expect(pretty("=A1*B1*2")).toBe("=A1 × B1 × 2");
  });
  it("shows <> as ≠", () => {
    expect(pretty('=SUMIF(G2:G15,"Paid",F2:F15)+(A1<>B1)')).toBe('=SUMIF(G2:G15,"Paid",F2:F15)+(A1 ≠ B1)');
  });
  it("leaves string literals and sheet names alone", () => {
    expect(pretty('=COUNTIF(A:A,"<>x*")')).toBe('=COUNTIF(A:A,"<>x*")');
    expect(pretty('="a""*""b"*2')).toBe('="a""*""b" × 2');
    expect(pretty("='Q*1'.A1*2")).toBe("='Q*1'.A1 × 2");
  });
  it("only touches formulas", () => {
    expect(pretty("2*3")).toBe("2*3");
    expect(pretty("")).toBe("");
    expect(pretty("=")).toBe("=");
  });
});
