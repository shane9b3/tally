/**
 * Small AOO 4.1 options (mock engine):
 * - Tools ▸ Options ▸ Input ▸ "Year (two digits)" (AOO General): the window
 *   for documents created from now on; `1/1/45` is 1945 with 1930–2029 and
 *   2045 with 1950–2049;
 * - Calculate ▸ "Limit decimals for general number format": π shows 3.14;
 * - View ▸ Zoom… at 20 % and 400 % (AOO MINZOOM / MAXZOOM), and the grid renders.
 */
import { expect, test, type Page } from "@playwright/test";

interface Rec {
  type: string;
  [k: string]: unknown;
}

async function ready(page: Page, url = "/") {
  await page.goto(url);
  await expect(page.getByTestId("grid")).toBeVisible();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);
}

const recorded = (page: Page): Promise<Rec[]> =>
  page.evaluate(() => JSON.parse(JSON.stringify((window.__tally!.client as unknown as { recorded: unknown[] }).recorded)) as Rec[]);

async function enter(page: Page, r: number, c: number, text: string) {
  await page.evaluate(([r, c, text]) => window.__tally!.controller.startEdit({ r: r as number, c: c as number }, text as string, "grid"), [r, c, text] as const);
  await page.evaluate(() => window.__tally!.controller.commitEdit("none"));
}

async function cellText(page: Page, r: number, c: number): Promise<string> {
  return page.evaluate(
    async ([r, c]) => {
      const t = window.__tally!;
      const info = t.controller.info!;
      const [block] = await t.client.cells(info.doc, t.controller.sheet, [{ r0: r, c0: c, r1: r, c1: c }]);
      return block?.cells.find((v) => v.r === r && v.c === c)?.text ?? "";
    },
    [r, c] as const,
  );
}

async function inputLine(page: Page, a1: string): Promise<string> {
  await page.evaluate((ref) => window.__tally!.controller.goTo(ref), a1);
  const input = page.getByTestId("formula-input");
  await expect(input).not.toHaveValue("");
  return input.inputValue();
}

test("Year (two digits): 1930–2029 by default, 1950–2049 for documents created after the change", async ({ page }) => {
  await ready(page);
  await enter(page, 0, 0, "1/1/45");
  await expect.poll(() => cellText(page, 0, 0)).toBe("01/01/45");
  expect(await inputLine(page, "A1")).toBe("01/01/1945");

  await page.keyboard.press("Meta+,");
  const dlg = page.getByTestId("options-dialog");
  await expect(dlg).toBeVisible();
  await dlg.getByTestId("tab-input").click();
  await expect(dlg.getByTestId("opt-year-start")).toHaveValue("1930");
  await expect(dlg.getByTestId("opt-year-end")).toHaveText("and 2029");
  await dlg.getByTestId("opt-year-start").fill("1950");
  await expect(dlg.getByTestId("opt-year-end")).toHaveText("and 2049");
  // Out of AOO's range 1583–9857: "????" and no OK.
  await dlg.getByTestId("opt-year-start").fill("1500");
  await expect(dlg.getByTestId("opt-year-end")).toHaveText("and ????");
  await dlg.getByTestId("opt-year-start").fill("1950");
  await dlg.getByTestId("dialog-ok").click();
  await expect(dlg).toBeHidden();
  const prefs = await page.evaluate(() => JSON.parse(localStorage.getItem("tally.prefs") ?? "{}") as Record<string, unknown>);
  expect(prefs.twoDigitYearStart).toBe(1950);

  // The open document keeps its window (AOO copies the option when a document is created or loaded).
  await enter(page, 1, 0, "1/1/45");
  expect(await inputLine(page, "A2")).toBe("01/01/1945");

  // A new document (here: the reloaded page's blank workbook) takes 1950–2049.
  await page.reload();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);
  await enter(page, 0, 0, "1/1/45");
  expect(await inputLine(page, "A1")).toBe("01/01/2045");
  await enter(page, 1, 0, "1/1/50");
  expect(await inputLine(page, "A2")).toBe("01/01/1950");
  const calc = await page.evaluate(async () => {
    const t = window.__tally!;
    return (t.client as unknown as { calcSettings(doc: number): Promise<{ twoDigitYear?: number | null }> }).calcSettings(t.controller.info!.doc);
  });
  expect(calc.twoDigitYear).toBe(1950);
});

test("Limit decimals for general number format: π shows 3.14", async ({ page }) => {
  await ready(page);
  await enter(page, 0, 0, "3.14159265358979");
  await expect.poll(() => cellText(page, 0, 0)).toBe("3.14159265358979");
  await page.keyboard.press("Meta+,");
  const dlg = page.getByTestId("options-dialog");
  await expect(dlg.getByTestId("opt-decimal-places")).toBeDisabled();
  await dlg.getByTestId("opt-limit-decimals").check();
  await expect(dlg.getByTestId("opt-decimal-places")).toBeEnabled();
  await dlg.getByTestId("opt-decimal-places").fill("2");
  await dlg.getByTestId("dialog-ok").click();
  expect((await recorded(page)).at(-1)).toMatchObject({ type: "setCalcSettings", settings: { standardDecimals: 2 } });
  await expect.poll(() => cellText(page, 0, 0)).toBe("3.14");
  await enter(page, 1, 0, "1.5");
  await expect.poll(() => cellText(page, 1, 0)).toBe("1.5");
  // The value is kept: the input line shows all digits.
  expect(await inputLine(page, "A1")).toBe("3.14159265358979");
  // Re-open: the document reports the limit.
  await page.keyboard.press("Meta+,");
  await expect(dlg.getByTestId("opt-limit-decimals")).toBeChecked();
  await expect(dlg.getByTestId("opt-decimal-places")).toHaveValue("2");
  await dlg.getByTestId("dialog-cancel").click();
});

async function zoomTo(page: Page, value: string) {
  await page.evaluate(() => window.__tally!.controller.registry.run("view.zoomDialog"));
  const dlg = page.getByTestId("zoom-dialog");
  await expect(dlg).toBeVisible();
  await dlg.getByTestId("zoom-value").fill(value);
  await expect(dlg.getByTestId("zoom-variable")).toBeChecked();
  await dlg.getByTestId("dialog-ok").click();
  await expect(dlg).toBeHidden();
}

const geometry = (page: Page) =>
  page.evaluate(() => {
    const g = window.__tally!.controller.grid!.getGeometry();
    return { zoom: g.zoom, headerH: g.headerH, headerW: g.headerW, width: g.width };
  });

const savedZoom = (page: Page) =>
  page.evaluate(async () => {
    const t = window.__tally!;
    return (await t.client.viewOptions(t.controller.info!.doc, t.controller.sheet)).zoom ?? null;
  });

test("View ▸ Zoom: 20 % and 400 %, and the grid renders", async ({ page }) => {
  await ready(page, "/?template=invoices");
  await zoomTo(page, "20");
  await expect(page.getByTestId("zoom")).toHaveText("20%");
  await expect.poll(() => geometry(page).then((g) => g.zoom)).toBe(0.2);
  const small = await geometry(page);
  // Headers stay at their 50 % size (legible); the canvas has its full width.
  expect(small.headerH).toBe(12);
  expect(small.width).toBeGreaterThan(400);
  await expect.poll(() => savedZoom(page)).toBe(20);
  const canvas = page.getByTestId("grid").locator("canvas").first();
  await expect(canvas).toBeVisible();

  await zoomTo(page, "400");
  await expect(page.getByTestId("zoom")).toHaveText("400%");
  await expect.poll(() => geometry(page).then((g) => g.zoom)).toBe(4);
  expect((await geometry(page)).headerH).toBe(96);
  await expect.poll(() => savedZoom(page)).toBe(400);

  // Beyond the limits the dialog clamps (AOO SetLimits(MINZOOM, MAXZOOM)).
  await zoomTo(page, "10");
  await expect(page.getByTestId("zoom")).toHaveText("20%");
  await page.keyboard.press("Meta+=");
  await expect(page.getByTestId("zoom")).toHaveText("25%");

  // The 100 % choice and the status bar presets still work.
  await page.evaluate(() => window.__tally!.controller.registry.run("view.zoomDialog"));
  await page.getByTestId("zoom-dialog").getByTestId("zoom-100").check();
  await page.getByTestId("zoom-dialog").getByTestId("dialog-ok").click();
  await expect(page.getByTestId("zoom")).toHaveText("100%");
  await expect.poll(() => savedZoom(page)).toBeNull();
  // A cell is still where the grid draws it.
  await expect.poll(() => cellText(page, 0, 0)).not.toBe("");
});
