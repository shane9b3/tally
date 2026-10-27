/**
 * Input behaviour taken from AOO (docs/parity-deviations.md U7, U22):
 *  - on a protected sheet Tab / Shift+Tab / Enter move between unlocked cells;
 *  - Tools ▸ Options ▸ Input "Show overwrite warning when pasting data" and the
 *    "You are pasting data into cells that already contain data." confirmation.
 */
import { expect, test, type Page } from "@playwright/test";

const ROW_H = 24;
const HEADER_H = 24;

interface Rec {
  type: string;
  [k: string]: unknown;
}

async function ready(page: Page) {
  await page.goto("/?template=invoices");
  await expect(page.getByTestId("grid")).toBeVisible();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);
}

async function clickCell(page: Page, r: number, c: number) {
  const box = await page.getByTestId("grid").boundingBox();
  if (!box) throw new Error("grid not visible");
  const { headerW, xs, w } = await page.evaluate((c) => {
    const ctl = window.__tally!.controller;
    let x = 0;
    for (let i = 0; i < c; i++) x += ctl.layoutSize("col", i);
    return { headerW: (ctl.grid as unknown as { headerW: number }).headerW, xs: x, w: ctl.layoutSize("col", c) };
  }, c);
  await page.mouse.click(box.x + headerW + xs + w / 2, box.y + HEADER_H + r * ROW_H + ROW_H / 2);
}

const recorded = (page: Page): Promise<Rec[]> =>
  page.evaluate(() => JSON.parse(JSON.stringify((window.__tally!.client as unknown as { recorded: unknown[] }).recorded)) as Rec[]);

const run = (page: Page, id: string) => page.evaluate((id) => window.__tally!.controller.registry.run(id), id);

const activeCell = (page: Page) => page.getByTestId("name-box");

async function text(page: Page, r: number, c: number): Promise<string> {
  return page.evaluate(
    async ([r, c]) => {
      const t = window.__tally!;
      const [b] = await t.client.cells(t.controller.info!.doc, t.controller.sheet, [{ r0: r, c0: c, r1: r, c1: c }]);
      return b?.cells.find((x) => x.r === r && x.c === c)?.text ?? "";
    },
    [r, c] as const,
  );
}

test("Tab, Shift+Tab and Enter skip locked cells on a protected sheet", async ({ page }) => {
  await ready(page);
  // Unlock B3:B4 and D3, merge-and-unlock G6:H6 beside a locked I6 and an unlocked J6, hide row 9 (unlocked B9), then protect.
  await page.evaluate(() =>
    window.__tally!.controller.apply("Unlock", [
      { type: "mergeCells", sheet: 1, range: { r0: 5, c0: 6, r1: 5, c1: 7 } },
      { type: "setCellProtection", sheet: 1, ranges: [{ r0: 2, c0: 1, r1: 3, c1: 1 }, { r0: 2, c0: 3, r1: 2, c1: 3 }, { r0: 5, c0: 6, r1: 5, c1: 7 }, { r0: 5, c0: 9, r1: 5, c1: 9 }, { r0: 8, c0: 1, r1: 8, c1: 1 }], locked: false, hiddenFormula: null },
      { type: "setRowsHidden", sheet: 1, start: 8, end: 8, hidden: true },
    ]),
  );
  // Not protected yet: Tab is a plain move.
  await clickCell(page, 2, 1);
  await page.keyboard.press("Tab");
  await expect(activeCell(page)).toHaveValue("C3");
  await run(page, "tools.protectSheet");
  await page.getByTestId("protect-sheet-dialog").getByTestId("dialog-ok").click();
  await expect.poll(async () => (await recorded(page)).at(-1)?.type).toBe("protectSheet");
  await clickCell(page, 2, 1);
  await expect(activeCell(page)).toHaveValue("B3");
  await page.keyboard.press("Tab");
  await expect(activeCell(page)).toHaveValue("D3");
  await page.keyboard.press("Tab");
  await expect(activeCell(page)).toHaveValue("B4");
  await page.keyboard.press("Tab"); // the merge G6:H6 is one stop; its covered H6, the locked I6 and hidden row 9 are skipped
  await expect(activeCell(page)).toHaveValue("G6:H6");
  await page.keyboard.press("Tab");
  await expect(activeCell(page)).toHaveValue("J6");
  await page.keyboard.press("Tab"); // wraps to the first unlocked cell
  await expect(activeCell(page)).toHaveValue("B3");
  await page.keyboard.press("Shift+Tab");
  await expect(activeCell(page)).toHaveValue("J6");
  await page.keyboard.press("Shift+Tab");
  await expect(activeCell(page)).toHaveValue("G6:H6");
  await page.keyboard.press("Shift+Tab");
  await expect(activeCell(page)).toHaveValue("B4");
  // Enter walks down the columns between unlocked cells.
  await clickCell(page, 2, 1);
  await page.keyboard.press("Enter");
  await expect(activeCell(page)).toHaveValue("B4");
  await page.keyboard.press("Enter");
  await expect(activeCell(page)).toHaveValue("D3");
  // Typing in an unlocked cell and pressing Tab commits and jumps to the next unlocked cell.
  await clickCell(page, 2, 1);
  await page.keyboard.type("7");
  await page.keyboard.press("Tab");
  await expect(activeCell(page)).toHaveValue("D3");
  await expect.poll(() => text(page, 2, 1)).toBe("7");
});

test("pasting over data asks first; No cancels, Yes pastes, Undo restores", async ({ page }) => {
  await ready(page);
  const before = await text(page, 3, 1);
  await clickCell(page, 1, 1); // B2 (has data)
  await page.keyboard.press("ControlOrMeta+c");
  // Into an empty cell: no dialog.
  await clickCell(page, 25, 1);
  await page.evaluate(() => window.__tally!.controller.pasteFromClipboard(false));
  await expect(page.getByTestId("confirm-dialog")).toHaveCount(0);
  await expect.poll(async () => (await recorded(page)).at(-1)?.type).toBe("paste");
  // Over data: the AOO confirmation.
  await clickCell(page, 3, 1);
  const n = (await recorded(page)).length;
  void page.evaluate(() => window.__tally!.controller.pasteFromClipboard(false));
  const dlg = page.getByTestId("confirm-dialog");
  await expect(dlg).toBeVisible();
  await expect(dlg).toContainText("You are pasting data into cells that already contain data.");
  await expect(dlg.getByTestId("dialog-cancel")).toHaveText("No");
  await expect(dlg.getByTestId("dialog-ok")).toHaveText("Yes");
  await page.keyboard.press("Escape");
  await expect(dlg).toHaveCount(0);
  expect((await recorded(page)).length).toBe(n);
  expect(await text(page, 3, 1)).toBe(before);
  // Yes by keyboard: Yes has the focus, Enter confirms.
  void page.evaluate(() => window.__tally!.controller.pasteFromClipboard(false));
  await expect(dlg).toBeVisible();
  await expect(dlg.getByTestId("dialog-ok")).toBeFocused(); // AOO's Yes / No box defaults to Yes
  await page.keyboard.press("Enter");
  await expect(dlg).toHaveCount(0);
  await expect.poll(async () => (await recorded(page)).at(-1)?.type).toBe("paste");
  expect(await text(page, 3, 1)).toBe(await text(page, 1, 1));
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(() => text(page, 3, 1)).toBe(before);
});

test("pasting external text writes one cell, so a selection over data elsewhere does not warn", async ({ page }) => {
  await ready(page);
  // A1:A... holds data further down only: select B20:B30 (empty), put data in B25, paste "x" at B20.
  await page.evaluate(() => window.__tally!.controller.apply("Fill", [{ type: "setCellInput", sheet: 1, r: 24, c: 1, input: "keep" }]));
  await clickCell(page, 19, 1);
  await page.keyboard.down("Shift");
  await clickCell(page, 29, 1);
  await page.keyboard.up("Shift");
  await page.evaluate(() => window.__tally!.controller.pasteData("x", null, false));
  await expect(page.getByTestId("confirm-dialog")).toHaveCount(0);
  await expect.poll(async () => (await recorded(page)).at(-1)?.type).toBe("paste");
  expect(await text(page, 24, 1)).toBe("keep");
});

test("Paste Special over data asks too, and the Input option turns the warning off", async ({ page }) => {
  await ready(page);
  await clickCell(page, 1, 1);
  await page.keyboard.press("ControlOrMeta+c");
  await clickCell(page, 3, 1);
  void page.evaluate(() => window.__tally!.controller.pasteSpecial({ what: "values", transpose: false, skipEmpty: false, operation: "none" }));
  const dlg = page.getByTestId("confirm-dialog");
  await expect(dlg).toBeVisible();
  await dlg.getByTestId("dialog-cancel").click();
  await expect(dlg).toHaveCount(0);
  expect((await recorded(page)).some((o) => o.type === "pasteSpecial")).toBe(false);
  // Tools ▸ Options ▸ Input.
  await page.keyboard.press("Meta+,");
  const opts = page.getByTestId("options-dialog");
  await opts.getByTestId("tab-input").click();
  await expect(opts.getByTestId("opt-paste-warning")).toBeChecked();
  await expect(opts).toContainText("Show overwrite warning when pasting data");
  await opts.getByTestId("opt-paste-warning").uncheck();
  await opts.getByTestId("dialog-ok").click();
  const prefs = await page.evaluate(() => JSON.parse(localStorage.getItem("tally.prefs") ?? "{}") as Record<string, unknown>);
  expect(prefs).toMatchObject({ pasteOverwriteWarning: false });
  await clickCell(page, 3, 1);
  await page.evaluate(() => window.__tally!.controller.pasteSpecial({ what: "values", transpose: false, skipEmpty: false, operation: "none" }));
  await expect(dlg).toHaveCount(0);
  await expect.poll(async () => (await recorded(page)).at(-1)?.type).toBe("pasteSpecial");
});
