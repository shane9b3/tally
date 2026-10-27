/**
 * AutoFilter "Top 10" (AOO's largest-10 condition, ties included), Find &
 * Replace "Search in Notes" and "Current selection only", and the CSV
 * export prompts (Text Export options remembered for the session, "Keep
 * current format" on Save). WebMockClient emulates the engine; the mock
 * shell rejects a picked CSV file with `csv_options` like the desktop
 * shell and records each CSV save in `client.savedCsv`.
 */
import { expect, test, type Page } from "@playwright/test";

const ROW_H = 24;
const HEADER_H = 24;

interface Rec {
  type: string;
  [k: string]: unknown;
}

async function ready(page: Page, url = "/?template=invoices") {
  await page.goto(url);
  await expect(page.getByTestId("grid")).toBeVisible();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);
}

async function cellXY(page: Page, r: number, c: number) {
  const box = await page.getByTestId("grid").boundingBox();
  if (!box) throw new Error("grid not visible");
  const { headerW, xs, w } = await page.evaluate((c) => {
    const ctl = window.__tally!.controller;
    let x = 0;
    for (let i = 0; i < c; i++) x += ctl.layoutSize("col", i);
    return { headerW: (ctl.grid as unknown as { headerW: number }).headerW, xs: x, w: ctl.layoutSize("col", c) };
  }, c);
  return { x: box.x + headerW + xs + w / 2, y: box.y + HEADER_H + r * ROW_H + ROW_H / 2, w, left: box.x + headerW + xs };
}

async function clickCell(page: Page, r: number, c: number, modifier?: "Shift") {
  const p = await cellXY(page, r, c);
  if (modifier) await page.keyboard.down(modifier);
  await page.mouse.click(p.x, p.y);
  if (modifier) await page.keyboard.up(modifier);
}

const recorded = (page: Page): Promise<Rec[]> =>
  page.evaluate(() => JSON.parse(JSON.stringify((window.__tally!.client as unknown as { recorded: unknown[] }).recorded)) as Rec[]);

const run = (page: Page, id: string) => page.evaluate((id) => window.__tally!.controller.registry.run(id), id);

async function hiddenRows(page: Page): Promise<number[]> {
  return page.evaluate(async () => {
    const t = window.__tally!;
    const l = await t.client.sheetLayout(t.controller.info!.doc, t.controller.sheet);
    return l.rowRuns.filter((r) => r.hidden).flatMap((r) => Array.from({ length: r.end - r.start + 1 }, (_, i) => r.start + i));
  });
}

async function text(page: Page, r: number, c: number): Promise<string> {
  return page.evaluate(
    async ([r, c]) => {
      const t = window.__tally!;
      const [block] = await t.client.cells(t.controller.info!.doc, t.controller.sheet, [{ r0: r, c0: c, r1: r, c1: c }]);
      return block?.cells.find((x) => x.r === r && x.c === c)?.text ?? "";
    },
    [r, c] as const,
  );
}

async function openFilter(page: Page, c: number) {
  const h = await cellXY(page, 0, c);
  await page.mouse.click(h.left + h.w - 11, h.y);
  await expect(page.getByTestId("filter-popover")).toBeVisible();
}

test("AutoFilter Top 10 keeps the 10 largest values, ties included", async ({ page }) => {
  await ready(page);
  await clickCell(page, 3, 1);
  await page.locator('[data-command="data.filter"]').click();
  // Qty (D): 12 30 8 20 15 10 40 24 6 5 18 12 50 9 in rows 2-15.
  await openFilter(page, 3);
  await page.getByTestId("filter-top10").click();
  await expect(page.getByTestId("filter-popover")).toHaveCount(0);
  const ops = await recorded(page);
  expect(ops.at(-1)).toEqual({ type: "setColumnFilter", sheet: 1, col: 3, filter: { values: null, condition: { op: "topValues", value: "10" } } });
  // The 10th largest is 10: 9, 8, 6 and 5 (rows 15, 4, 10, 11) are hidden.
  await expect.poll(() => hiddenRows(page)).toEqual([3, 9, 10, 14]);
  // Largest 8 through the condition list: the 8th largest is 12, twice, so 9 rows stay.
  await openFilter(page, 3);
  const pop = page.getByTestId("filter-popover");
  await expect(pop.getByTestId("filter-op")).toHaveValue("topValues");
  await expect(pop.getByTestId("filter-value")).toHaveValue("10");
  await pop.getByTestId("filter-value").fill("8");
  await pop.getByTestId("filter-apply").click();
  await expect.poll(() => hiddenRows(page)).toEqual([3, 6, 9, 10, 14]);
  // The smallest 10 %: 14 numbers → offset 1, threshold 6: 5 and 6 stay.
  await openFilter(page, 3);
  await pop.getByTestId("filter-op").selectOption("bottomPercent");
  await pop.getByTestId("filter-value").fill("10");
  await pop.getByTestId("filter-apply").click();
  await expect.poll(async () => (await hiddenRows(page)).length).toBe(12);
});

test("Find & Replace: search in notes, current selection only, Replace", async ({ page }) => {
  await ready(page);
  // Notes on B3 and C5 (0-based (2,1) and (4,2)).
  await page.evaluate(async () => {
    const ctl = window.__tally!.controller;
    const note = (r: number, c: number, text: string) => ({ type: "setNote" as const, sheet: ctl.sheet, r, c, note: { r, c, text, author: null, created: null, shown: false } });
    await ctl.apply("notes", [note(2, 1, "Rush order"), note(4, 2, "rush: call first")]);
  });
  await clickCell(page, 0, 0);
  await run(page, "edit.findReplace");
  const dlg = page.getByTestId("find-replace-dialog");
  await expect(dlg).toBeVisible();
  await expect(page.getByTestId("find-selection-only")).not.toBeChecked();
  await page.getByTestId("replace-find").fill("rush");
  await page.getByTestId("replace-with").fill("Urgent");
  await page.getByTestId("find-search-in").selectOption("notes");
  // Find next goes to the first note that matches.
  await dlg.getByRole("button", { name: "Find next" }).click();
  await expect(page.getByTestId("name-box")).toHaveValue("B3");
  await dlg.getByTestId("dialog-ok").click();
  const notes = () =>
    page.evaluate(async () => {
      const t = window.__tally!;
      return (await t.client.notes(t.controller.info!.doc, t.controller.sheet)).map((n) => n.text);
    });
  await expect.poll(notes).toEqual(["Urgent order", "Urgent: call first"]);
  await dlg.getByTestId("dialog-cancel").click();
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(notes).toEqual(["Rush order", "rush: call first"]);

  // Current selection only: A2:G5 holds Paid in G2 and G4; G6 stays.
  await clickCell(page, 1, 0);
  await clickCell(page, 4, 6, "Shift");
  await run(page, "edit.findReplace");
  await expect(page.getByTestId("find-selection-only")).toBeChecked();
  await page.getByTestId("find-search-in").selectOption("values");
  await page.getByTestId("replace-find").fill("Paid");
  await page.getByTestId("replace-with").fill("Settled");
  await dlg.getByTestId("dialog-ok").click();
  await expect.poll(() => text(page, 1, 6)).toBe("Settled");
  expect(await text(page, 3, 6)).toBe("Settled");
  expect(await text(page, 5, 6)).toBe("Paid");
  const last = (await recorded(page)).filter((o) => o.type === "replaceAll").at(-1) as { query: { selection: unknown; sheet: number } };
  expect(last.query.selection).toEqual([{ r0: 1, c0: 0, r1: 4, c1: 6 }]);

  // Replace (one): off the selection scope, replaces the active hit, then moves to the next.
  await page.getByTestId("find-selection-only").uncheck();
  await dlg.getByRole("button", { name: "Find next" }).click();
  await expect(page.getByTestId("name-box")).toHaveValue("G6");
  await page.getByTestId("replace-one").click();
  await expect.poll(() => text(page, 5, 6)).toBe("Settled");
  await expect(page.getByTestId("name-box")).toHaveValue("G8");
  expect(await text(page, 7, 6)).toBe("Paid");
});

test("CSV export: Text Export options, remembered; Save asks to keep the format", async ({ page }) => {
  await ready(page);
  const saved = () => page.evaluate(() => JSON.parse(JSON.stringify((window.__tally!.client as unknown as { savedCsv: unknown[] }).savedCsv)) as { title: string; csv: Record<string, unknown> | null }[]);
  await run(page, "file.exportCsv");
  const dlg = page.getByTestId("csv-export-dialog");
  await expect(dlg).toBeVisible();
  await expect(dlg).toContainText("Invoices.csv");
  await expect(page.getByTestId("csv-field-delimiter")).toHaveValue(",");
  await expect(page.getByTestId("csv-as-shown")).toBeChecked();
  await page.getByTestId("csv-field-delimiter").selectOption("other");
  await page.getByTestId("csv-other-delimiter").fill("|");
  await page.getByTestId("csv-text-delimiter").selectOption("'");
  await page.getByTestId("csv-quote-all").check();
  await page.getByTestId("csv-as-shown").uncheck();
  await page.getByTestId("csv-charset").selectOption("utf16");
  await page.getByTestId("csv-export-ok").click();
  await expect(dlg).toHaveCount(0);
  await expect
    .poll(saved)
    .toEqual([{ doc: expect.any(Number), title: "Invoices.csv", csv: { fieldDelimiter: "|", textDelimiter: "'", quoteAllText: true, asShown: false, charset: "utf16" } }]);
  // The next export starts from the last choices; Cancel saves nothing.
  await run(page, "file.exportCsv");
  await expect(page.getByTestId("csv-other-delimiter")).toHaveValue("|");
  await expect(page.getByTestId("csv-charset")).toHaveValue("utf16");
  await page.keyboard.press("Escape");
  await expect(dlg).toHaveCount(0);
  expect(await saved()).toHaveLength(1);
  // Save of the CSV document: AOO's "Keep current format" prompt.
  await run(page, "file.save");
  const keep = page.getByTestId("csv-keep-format");
  await expect(keep).toBeVisible();
  await page.getByTestId("csv-keep-keep").click();
  await expect.poll(async () => (await saved()).length).toBe(2);
  expect((await saved())[1]?.csv).toEqual({ fieldDelimiter: "|", textDelimiter: "'", quoteAllText: true, asShown: false, charset: "utf16" });
  // Use ODF Format! saves as ODS instead.
  await run(page, "file.save");
  await page.getByTestId("csv-keep-odf").click();
  await expect.poll(() => page.evaluate(() => window.__tally!.controller.info?.title)).toBe("Invoices.ods");
  expect(await saved()).toHaveLength(2);
});

test("CSV export warns that only the active sheet is saved", async ({ page }) => {
  await ready(page);
  await run(page, "sheet.insert");
  await expect.poll(() => page.evaluate(() => window.__tally!.controller.info?.sheets.length)).toBe(2);
  await run(page, "file.exportCsv");
  await expect(page.getByTestId("csv-active-sheet")).toContainText("Only the active sheet");
  await page.getByTestId("csv-export-cancel").click();
  await expect(page.getByTestId("csv-export-dialog")).toHaveCount(0);
});
