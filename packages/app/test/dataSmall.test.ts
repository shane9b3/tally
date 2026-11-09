import { describe, expect, it } from "vitest";
import type { DatabaseRangeDto } from "@tally/bridge";
import { databaseRangeAt } from "../src/data/builders";
import { sortListEntries } from "../src/v03";

const db = (name: string, sheet: number, r0: number, c0: number, r1: number, c1: number): DatabaseRangeDto => ({
  name,
  sheet,
  range: { r0, c0, r1, c1 },
  hasHeader: true,
  filter: null,
  hasSort: false,
  hasSubtotals: false,
});

describe("Data ▸ Refresh Range target", () => {
  it("takes the database range at the cursor on the active sheet, the smallest when nested", () => {
    const list = [db("Big", 1, 0, 0, 20, 5), db("Small", 1, 2, 1, 4, 2), db("Other", 2, 0, 0, 9, 9)];
    expect(databaseRangeAt(list, 1, { r: 3, c: 1 })?.name).toBe("Small");
    expect(databaseRangeAt(list, 1, { r: 10, c: 0 })?.name).toBe("Big");
    expect(databaseRangeAt(list, 1, { r: 30, c: 0 })).toBeNull();
    expect(databaseRangeAt(list, 3, { r: 0, c: 0 })).toBeNull();
  });
});

describe("Validity: Sort entries ascending (browser demo engine)", () => {
  it("puts numbers first, then text in code-point order, each once", () => {
    expect(sortListEntries(["pear", "10", "Apple", "2", "apple", "pear", "Zebra", "2.0"])).toEqual(["2", "10", "Apple", "Zebra", "apple", "pear"]);
  });
});
