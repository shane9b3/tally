import { describe, expect, it } from "vitest";
import { cellAddress, colToLetters, lettersToCol } from "../src/colLetters";
import { MAX_COLS } from "../src/types";

describe("column letters", () => {
  it("converts known columns", () => {
    expect(colToLetters(0)).toBe("A");
    expect(colToLetters(25)).toBe("Z");
    expect(colToLetters(26)).toBe("AA");
    expect(colToLetters(51)).toBe("AZ");
    expect(colToLetters(52)).toBe("BA");
    expect(colToLetters(701)).toBe("ZZ");
    expect(colToLetters(702)).toBe("AAA");
    expect(colToLetters(MAX_COLS - 1)).toBe("XFD");
  });

  it("round-trips every column", () => {
    for (let c = 0; c < MAX_COLS; c++) expect(lettersToCol(colToLetters(c))).toBe(c);
  });

  it("parses case-insensitively and rejects junk", () => {
    expect(lettersToCol("xfd")).toBe(16383);
    expect(lettersToCol("")).toBe(-1);
    expect(lettersToCol("A1")).toBe(-1);
  });

  it("formats addresses", () => {
    expect(cellAddress(0, 0)).toBe("A1");
    expect(cellAddress(1_048_575, 16_383)).toBe("XFD1048576");
  });
});
