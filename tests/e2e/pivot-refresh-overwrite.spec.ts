/**
 * DataPilot refresh over occupied cells (AOO `STR_PIVOT_NOTEMPTY`,
 * `dbdocfun.cxx`; mock engine): when the refreshed output would grow over
 * cells that hold data, Tally asks first. Cancel (button or Escape) leaves
 * the sheet and the pivot as they were; Yes refreshes with `overwrite` and
 * the cells are replaced; one Undo brings them back.
 */
import { expect, test, type Page } from "@playwright/test";

const ROW_H = 24;
/** Invoices template: the Status column (G). */
const STATUS_COL = 6;
/** Where the pivot goes: J2, right of the data. */
const TARGET = { r: 1, c: 9 };
const NEW_STATUS = "Zz disputed";
/** Capitalized already: AutoCorrect leaves it as typed. */
const KEEP = "Keep me";

interface Rec {
  type: string;
  [k: string]: unknown;
}

interface Range {
  r0: number;
  c0: number;
  r1: number;
  c1: number;
}

async function ready(page: Page, url = "/?template=invoices") {
  await page.goto(url);
  await expect(page.getByTestId("grid")).toBeVisible();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);
}

async function clickCell(page: Page, r: number, c: number) {
  const box = await page.getByTestId("grid").boundingBox();
  if (!box) throw new Error("grid not visible");
  const { headerW, headerH, xs, w } = await page.evaluate((c) => {
    const ctl = window.__tally!.controller;
    let x = 0;
    for (let i = 0; i < c; i++) x += ctl.layoutSize("col", i);
    const geo = ctl.grid!.getGeometry();
    return { headerW: geo.headerW, headerH: geo.headerH, xs: x, w: ctl.layoutSize("col", c) };
  }, c);
  await page.mouse.click(box.x + headerW + xs + w / 2, box.y + headerH + r * ROW_H + ROW_H / 2);
  // Nothing below depends on where the pointer rests: park it off the page content.
  await page.mouse.move(0, 0);
}

const run = (page: Page, id: string) => page.evaluate((id) => window.__tally!.controller.registry.run(id), id);

const recorded = (page: Page): Promise<Rec[]> =>
  page.evaluate(() => JSON.parse(JSON.stringify((window.__tally!.client as unknown as { recorded: unknown[] }).recorded)) as Rec[]);

const refreshOps = async (page: Page): Promise<boolean[]> => (await recorded(page)).filter((o) => o.type === "refreshPivot").map((o) => o.overwrite === true);

async function enter(page: Page, r: number, c: number, text: string) {
  await page.evaluate(([r, c, text]) => window.__tally!.controller.startEdit({ r: r as number, c: c as number }, text as string, "grid"), [r, c, text] as const);
  await page.evaluate(() => window.__tally!.controller.commitEdit("none"));
  await expect.poll(() => cellText(page, r, c)).toBe(text);
}

const cellText = (page: Page, r: number, c: number): Promise<string> =>
  page.evaluate(
    async ([r, c]) => {
      const t = window.__tally!;
      const [b] = await t.client.cells(t.controller.info!.doc, t.controller.sheet, [{ r0: r, c0: c, r1: r, c1: c }]);
      return b?.cells.find((v) => v.r === r && v.c === c)?.text ?? "";
    },
    [r, c] as const,
  );

/** The non-empty texts of a range, row by row (`r,c=text`). */
const rangeTexts = (page: Page, range: Range): Promise<string[]> =>
  page.evaluate(async (range) => {
    const t = window.__tally!;
    const blocks = await t.client.cells(t.controller.info!.doc, t.controller.sheet, [range]);
    return blocks
      .flatMap((b) => b.cells)
      .filter((v) => v.text !== "")
      .sort((a, b) => a.r - b.r || a.c - b.c)
      .map((v) => `${v.r},${v.c}=${v.text}`);
  }, range);

const pivotOutput = (page: Page): Promise<Range | null> =>
  page.evaluate(async () => {
    const t = window.__tally!;
    const p = (await t.client.pivots(t.controller.info!.doc))[0];
    return p?.output ? { r0: p.output.r0, c0: p.output.c0, r1: p.output.r1, c1: p.output.c1 } : null;
  });

/** Status by Sum of Total at J2 through the DataPilot dialog; returns its output range. */
async function createPivot(page: Page): Promise<Range> {
  await clickCell(page, 2, 1);
  await run(page, "data.dataPilot");
  const dlg = page.getByTestId("datapilot-dialog");
  await expect(dlg).toBeVisible();
  await dlg.getByTestId("pivot-add-row-Status").click();
  await dlg.getByTestId("pivot-add-data-Total").click();
  await expect(dlg.getByTestId("pivot-area-data")).toContainText("Sum - Total");
  await dlg.getByTestId("pivot-more").click();
  await dlg.getByTestId("pivot-target-sheet").selectOption({ index: 1 });
  await dlg.getByTestId("pivot-target").fill("J2");
  await dlg.getByTestId("dialog-ok").click();
  await expect(dlg).toBeHidden();
  await expect.poll(() => cellText(page, TARGET.r + 2, TARGET.c)).toBe("Status");
  const out = await pivotOutput(page);
  if (!out) throw new Error("the pivot has no output range");
  expect(out.r0).toBe(TARGET.r);
  expect(out.c0).toBe(TARGET.c);
  return out;
}

/** Puts the cursor on the pivot's header cell and runs Data ▸ DataPilot ▸ Refresh. */
async function refresh(page: Page) {
  const at = { r: TARGET.r + 2, c: TARGET.c };
  await page.evaluate((at) => window.__tally!.controller.onSelectionChange({ range: { r0: at.r, c0: at.c, r1: at.r, c1: at.c }, active: at, anchor: at }), at);
  await expect.poll(() => page.evaluate(() => window.__tally!.controller.registry.isEnabled("data.refreshPivot"))).toBe(true);
  await run(page, "data.refreshPivot");
}

test("refreshing a pivot over occupied cells asks first: Cancel leaves the sheet, Yes overwrites", async ({ page }) => {
  await ready(page);
  const out = await createPivot(page);
  const below = { r: out.r1 + 1, c: out.c0 };
  // The last output row is the grand total; the row below it is free.
  const totalLabel = await cellText(page, out.r1, out.c0);
  expect(totalLabel).not.toBe("");
  expect(await cellText(page, below.r, below.c)).toBe("");

  // A refresh that fits needs no question.
  await refresh(page);
  await expect.poll(() => refreshOps(page)).toEqual([false]);
  await expect(page.getByTestId("confirm-dialog")).toHaveCount(0);
  expect(await pivotOutput(page)).toEqual(out);

  // Data right below the output, and one more status in the source: the
  // refreshed output is one row longer and would run over that cell.
  await enter(page, below.r, below.c, KEEP);
  await enter(page, 2, STATUS_COL, NEW_STATUS);
  const watched: Range = { r0: out.r0, c0: out.c0, r1: out.r1 + 2, c1: out.c1 };
  const before = await rangeTexts(page, watched);
  expect(before).toContain(`${below.r},${below.c}=${KEEP}`);
  expect(before.join("\n")).not.toContain(NEW_STATUS);

  const confirm = page.getByTestId("confirm-dialog");

  // Cancel: nothing changes.
  await refresh(page);
  await expect(confirm).toBeVisible();
  await expect(confirm).toContainText("The pivot table output will overwrite existing cells. Continue?");
  await expect(confirm.getByTestId("dialog-ok")).toHaveText("Yes");
  await confirm.getByTestId("dialog-cancel").click();
  await expect(confirm).toBeHidden();
  expect(await refreshOps(page)).toEqual([false, false]);
  expect(await rangeTexts(page, watched)).toEqual(before);
  expect(await pivotOutput(page)).toEqual(out);

  // Escape is Cancel.
  await refresh(page);
  await expect(confirm).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(confirm).toBeHidden();
  expect(await refreshOps(page)).toEqual([false, false, false]);
  expect(await rangeTexts(page, watched)).toEqual(before);
  expect(await pivotOutput(page)).toEqual(out);

  // Yes: the refresh is sent again with `overwrite` and takes the cell.
  await refresh(page);
  await expect(confirm).toBeVisible();
  await confirm.getByTestId("dialog-ok").click();
  await expect(confirm).toBeHidden();
  await expect.poll(() => refreshOps(page)).toEqual([false, false, false, false, true]);
  await expect.poll(() => pivotOutput(page)).toEqual({ ...out, r1: out.r1 + 1 });
  await expect.poll(() => cellText(page, below.r, below.c)).toBe(totalLabel);
  const after = await rangeTexts(page, watched);
  expect(after.join("\n")).not.toContain(KEEP);
  expect(after.some((s) => s.endsWith(`=${NEW_STATUS}`))).toBe(true);
  // No second question once it was answered.
  await expect(confirm).toHaveCount(0);

  // One Undo brings the overwritten cell and the previous output back.
  await run(page, "edit.undo");
  await expect.poll(() => cellText(page, below.r, below.c)).toBe(KEEP);
  await expect.poll(() => rangeTexts(page, watched)).toEqual(before);
});
