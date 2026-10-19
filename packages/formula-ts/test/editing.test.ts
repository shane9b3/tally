import { describe, expect, it } from "vitest";
import { callAt, groupedReferences, identifierAt, pointInsertion } from "../src/index";

describe("pointInsertion", () => {
  it("inserts after =, operators, ( , and ;", () => {
    expect(pointInsertion("=", 1)).toEqual({ start: 1, end: 1 });
    expect(pointInsertion("=A1+", 4)).toEqual({ start: 4, end: 4 });
    expect(pointInsertion("=SUM(", 5)).toEqual({ start: 5, end: 5 });
    expect(pointInsertion("=SUM(A1,", 8)).toEqual({ start: 8, end: 8 });
    expect(pointInsertion("=SUM(A1;", 8)).toEqual({ start: 8, end: 8 });
    expect(pointInsertion("=A1 + ", 6)).toEqual({ start: 6, end: 6 });
  });

  it("replaces a reference that follows an operator", () => {
    expect(pointInsertion("=A1+B2", 6)).toEqual({ start: 4, end: 6 });
    expect(pointInsertion("=SUM(A1:B3", 10)).toEqual({ start: 5, end: 10 });
    expect(pointInsertion("=B2", 3)).toEqual({ start: 1, end: 3 });
  });

  it("is off after operands, inside tokens and outside formulas", () => {
    expect(pointInsertion("=A1+2", 5)).toBeNull();
    expect(pointInsertion("=SUM(A1)", 8)).toBeNull();
    expect(pointInsertion("=A1+B2", 5)).toBeNull();
    expect(pointInsertion('="a+', 4)).toBeNull();
    expect(pointInsertion("A1+", 3)).toBeNull();
    expect(pointInsertion("=SU", 3)).toBeNull();
  });
});

describe("identifierAt", () => {
  it("finds the identifier being typed", () => {
    expect(identifierAt("=SU", 3)).toEqual({ start: 1, end: 3, text: "SU" });
    expect(identifierAt("=A1+av", 6)).toEqual({ start: 4, end: 6, text: "av" });
    expect(identifierAt("=LOG1", 5)?.text).toBe("LOG1");
  });

  it("ignores numbers, strings, refs with $ and non-formulas", () => {
    expect(identifierAt("=12", 3)).toBeNull();
    expect(identifierAt('="SU', 4)).toBeNull();
    expect(identifierAt("=$A1", 4)).toBeNull();
    expect(identifierAt("SU", 2)).toBeNull();
    expect(identifierAt("=SUM(", 5)).toBeNull();
  });
});

describe("callAt", () => {
  it("tracks the innermost call and argument index", () => {
    expect(callAt("=SUM(", 5)).toEqual({ name: "SUM", argIndex: 0, nameSpan: { start: 1, end: 4 } });
    expect(callAt("=IF(A1>0,SUM(A1,B1),", 20)?.name).toBe("IF");
    expect(callAt("=IF(A1>0,SUM(A1,B1),", 20)?.argIndex).toBe(2);
    expect(callAt("=IF(A1>0,SUM(A1,", 17)).toMatchObject({ name: "SUM", argIndex: 1 });
    expect(callAt("=SUM(A1)", 8)).toBeNull();
    expect(callAt("=(1+2", 5)).toBeNull();
    expect(callAt("=SUM({1,2", 9)).toMatchObject({ name: "SUM", argIndex: 0 });
  });
});

describe("groupedReferences", () => {
  it("gives repeated references the same group", () => {
    const refs = groupedReferences("=A1+$A$1+B2:C3+b2:c3+Sheet2.A1");
    expect(refs.map((r) => r.group)).toEqual([0, 0, 1, 1, 2]);
    expect(refs[4]?.sheet).toBe("Sheet2");
  });
});
