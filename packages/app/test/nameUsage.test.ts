import type { NameInfo, NameUsage, Op } from "@tally/bridge";
import { describe, expect, it } from "vitest";
import { WebMockClient } from "../src/dev/webMock";
import { criteriaChoices, matchChoice, NO_USAGE, printChoices, printRangeText, repeatText } from "../src/nameUsage";

const name = (n: string, refersTo: string, usage: Partial<NameUsage>): NameInfo => ({ name: n, scope: null, refersTo, sheetName: null, usage: { ...NO_USAGE, ...usage } });

describe("name usage lists (AOO ScPrintAreasDlg / ScSpecialFilterDlg)", () => {
  const names = [
    name("Crit", "$Data.$H$1:$I$3", { filter: true }),
    name("Area", "$Data.$A$1:$G$15", { printRange: true, repeatRow: true }),
    name("Titles", "$'My sheet'.$A$1:$B$2", { repeatRow: true, repeatColumn: true }),
    name("Rate", "0.07", { printRange: true, filter: true }),
    name("Plain", "$Data.$C$3", {}),
  ];

  it("criteria lists filter names with their reference, expressions included", () => {
    expect(criteriaChoices(names)).toEqual([
      { name: "Crit", text: "$Data.$H$1:$I$3" },
      { name: "Rate", text: "0.07" },
    ]);
  });

  it("print lists only ranges, formatted like lcl_GetRepeatRangeString", () => {
    expect(printChoices(names, "printRange")).toEqual([{ name: "Area", text: "$A$1:$G$15" }]);
    expect(printChoices(names, "repeatRow")).toEqual([
      { name: "Area", text: "$1:$15" },
      { name: "Titles", text: "$1:$2" },
    ]);
    expect(printChoices(names, "repeatColumn")).toEqual([{ name: "Titles", text: "$A:$B" }]);
  });

  it("formats single rows, columns and cells", () => {
    expect(repeatText({ r0: 0, c0: 2, r1: 0, c1: 2 }, "row")).toBe("$1");
    expect(repeatText({ r0: 0, c0: 2, r1: 0, c1: 2 }, "col")).toBe("$C");
    expect(printRangeText({ r0: 4, c0: 1, r1: 4, c1: 1 })).toBe("$B$5");
  });

  it("matches typed text ignoring $ and case", () => {
    const rows = printChoices(names, "repeatRow");
    expect(matchChoice(rows, "1:2")?.name).toBe("Titles");
    expect(matchChoice(rows, "$1:$15")?.name).toBe("Area");
    expect(matchChoice(rows, "3:4")).toBeNull();
    expect(matchChoice(rows, "  ")).toBeNull();
    expect(matchChoice(printChoices(names, "printRange"), "a1:g15")?.name).toBe("Area");
  });

  it("tolerates names without usage (older engines)", () => {
    const legacy = { name: "Old", scope: null, refersTo: "$Data.$A$1", sheetName: null } as unknown as NameInfo;
    expect(criteriaChoices([legacy])).toEqual([]);
    expect(printChoices([legacy], "printRange")).toEqual([]);
  });
});

describe("setNameUsage in the web mock", () => {
  it("sets flags, keeps them on redefinition, drops them with the name", async () => {
    const c = new WebMockClient();
    const info = await c.newWorkbook("invoices");
    const doc = info.doc;
    const tx = (ops: Op[]) => c.apply(doc, { ops, label: null, selectionBefore: null, selectionAfter: null });
    await tx([
      { type: "defineName", name: "Crit", scope: null, refersTo: "$Invoices.$H$1:$H$2" },
      { type: "setNameUsage", name: "crit", scope: null, usage: { ...NO_USAGE, filter: true } },
    ]);
    expect((await c.names(doc))[0]?.usage).toEqual({ ...NO_USAGE, filter: true });
    await tx([{ type: "defineName", name: "Crit", scope: null, refersTo: "$Invoices.$H$1:$H$3" }]);
    expect((await c.names(doc))[0]?.usage.filter).toBe(true);
    const bad = await tx([{ type: "setNameUsage", name: "Nope", scope: null, usage: NO_USAGE }]);
    expect(bad.errors[0]?.code).toBe("not_found");
  });
});
