/**
 * Format Cells ▸ Numbers (G14, AOO SvxNumberFormatTabPage): Category and
 * Format lists, Language, the currency list, the Options that regenerate the
 * code (GenerateFormat) and read it back, the cell's value in the preview,
 * keyboard-only use and an axe check of the tab.
 */
import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

const ROW_H = 24;

interface Rec {
  type: string;
  [k: string]: unknown;
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
}

const recorded = (page: Page): Promise<Rec[]> =>
  page.evaluate(() => JSON.parse(JSON.stringify((window.__tally!.client as unknown as { recorded: unknown[] }).recorded)) as Rec[]);

const run = (page: Page, id: string) => page.evaluate((id) => window.__tally!.controller.registry.run(id), id);

/** The cell as the grid shows it (text and colour). */
const shown = (page: Page, r: number, c: number): Promise<{ text: string; color: number | null }> =>
  page.evaluate(
    async ([r, c]) => {
      const t = window.__tally!;
      const client = t.client as unknown as { cellsWith(doc: unknown, sheet: number, ranges: unknown[], opts: unknown): Promise<{ cells: { text: string; color: number | null }[] }[]> };
      const [b] = await client.cellsWith(t.controller.info!.doc, t.controller.sheet, [{ r0: r, c0: c, r1: r, c1: c }], { showFormulas: false, valueHighlighting: false, hideZeroValues: false });
      return { text: b?.cells[0]?.text ?? "", color: b?.cells[0]?.color ?? null };
    },
    [r, c] as const,
  );

/**
 * The number format of the style the grid's cached view holds for a cell. Format Cells
 * (`controller.activeStyle`) takes the style id from that cache, which only follows the
 * engine once the apply result has been merged into the grid source.
 */
const cachedFormat = (page: Page, r: number, c: number): Promise<string | undefined> =>
  page.evaluate(
    async ([r, c]) => {
      const t = window.__tally!;
      const g = t.controller.grid as unknown as { props: { source: { getCell(r: number, c: number): { style?: number } | undefined } } };
      const [style] = await t.client.styles(t.controller.info!.doc, [g.props.source.getCell(r, c)?.style ?? 0]);
      return style?.numberFormat;
    },
    [r, c] as const,
  );

async function typeInto(page: Page, r: number, c: number, text: string) {
  await clickCell(page, r, c);
  await page.keyboard.type(text);
  await page.keyboard.press("Enter");
}

const option = (page: Page, list: string, value: string) => page.getByTestId("format-cells").locator(`[data-testid=${list}-option][data-value="${value}"]`);

test("Currency, 2 decimals, red negatives: the code, the options and the cell", async ({ page }) => {
  await ready(page);
  await typeInto(page, 2, 7, "-1234.5");
  await clickCell(page, 2, 7);
  await page.keyboard.press("Meta+1");
  const dlg = page.getByTestId("format-cells");
  await expect(dlg).toBeVisible();
  await expect(option(page, "numbers-category", "number")).toHaveAttribute("aria-selected", "true");
  await expect(option(page, "numbers-format", "General")).toHaveAttribute("aria-selected", "true");
  // The preview shows the cell's value.
  await expect(dlg.getByTestId("cells-preview")).toHaveText("-1234.5");

  await option(page, "numbers-category", "currency").click();
  const code = dlg.getByTestId("cells-format-code");
  await expect(code).toHaveValue("[$$-409]#,##0.00;[RED]-[$$-409]#,##0.00");
  await expect(dlg.getByTestId("numbers-currency")).toBeVisible();
  // Options regenerate the code (GenerateFormat) …
  await dlg.getByTestId("numbers-red").uncheck();
  await dlg.getByTestId("numbers-decimals").fill("0");
  await expect(code).toHaveValue("[$$-409]#,##0;-[$$-409]#,##0");
  await expect(option(page, "numbers-format", "[$$-409]#,##0;-[$$-409]#,##0")).toHaveAttribute("aria-selected", "true");
  await dlg.getByTestId("numbers-decimals").fill("2");
  await dlg.getByTestId("numbers-red").check();
  await expect(code).toHaveValue("[$$-409]#,##0.00;[RED]-[$$-409]#,##0.00");
  await expect(dlg.getByTestId("cells-preview")).toHaveText("-$1,234.50");
  await expect(dlg.getByTestId("cells-preview")).toHaveCSS("color", "rgb(220, 38, 38)");
  // … and the code edits the options.
  await code.fill("#,##0.000");
  await expect(option(page, "numbers-category", "number")).toHaveAttribute("aria-selected", "true");
  await expect(dlg.getByTestId("numbers-decimals")).toHaveValue("3");
  await expect(dlg.getByTestId("numbers-thousands")).toBeChecked();
  await expect(dlg.getByTestId("numbers-red")).not.toBeChecked();
  await option(page, "numbers-category", "currency").click();
  // A euro from the currency list.
  await dlg.getByTestId("numbers-currency").selectOption("EUR-407");
  await expect(code).toHaveValue("#,##0.00 [$€-407];[RED]-#,##0.00 [$€-407]");
  await dlg.getByTestId("numbers-currency").selectOption("USD-409");
  await expect(code).toHaveValue("[$$-409]#,##0.00;[RED]-[$$-409]#,##0.00");

  await dlg.getByTestId("dialog-ok").click();
  await expect(dlg).toBeHidden();
  const ops = await recorded(page);
  expect(ops.at(-1)).toMatchObject({ type: "applyStyle", patch: { numberFormat: "[$$-409]#,##0.00;[RED]-[$$-409]#,##0.00" } });
  await expect.poll(() => shown(page, 2, 7)).toEqual({ text: "-$1,234.50", color: 0xff0000ff });
});

test("native numbers and era codes in the preview (G7)", async ({ page }) => {
  await ready(page);
  await typeInto(page, 2, 7, "46314");
  await clickCell(page, 2, 7);
  await page.keyboard.press("Meta+1");
  const dlg = page.getByTestId("format-cells");
  await expect(dlg).toBeVisible();
  const code = dlg.getByTestId("cells-format-code");
  await code.fill("[NatNum1][$-411]#,##0");
  await expect(dlg.getByTestId("cells-preview")).toHaveText("四六、三一四");
  await code.fill("[NatNum4][$-411]0");
  await expect(dlg.getByTestId("cells-preview")).toHaveText("四万六千三百十四");
  await code.fill('[$-411]GGGE"年"M"月"D"日"');
  await expect(dlg.getByTestId("cells-preview")).toHaveText("令和8年10月19日");
  await code.fill("[NatNum1][$-41E][~buddhist]D/M/YYYY");
  await expect(dlg.getByTestId("cells-preview")).toHaveText("๑๙/๑๐/๒๕๖๙");
  await dlg.getByTestId("dialog-ok").click();
  await expect(dlg).toBeHidden();
  const ops = await recorded(page);
  expect(ops.at(-1)).toMatchObject({ type: "applyStyle", patch: { numberFormat: "[NatNum1][$-41E][~buddhist]D/M/YYYY" } });
});

test("a cell with a date format opens on Date with its entry selected; Language and Cancel", async ({ page }) => {
  await ready(page);
  await typeInto(page, 3, 7, "46314");
  await clickCell(page, 3, 7);
  await run(page, "format.number.date");
  await expect.poll(() => shown(page, 3, 7).then((s) => s.text)).toBe("10/19/2026");
  // Wait for the grid's cached view (what Format Cells reads), not only the engine.
  await expect.poll(() => cachedFormat(page, 3, 7)).toBe("MM/DD/YYYY");
  const before = (await recorded(page)).length;
  await page.keyboard.press("Meta+1");
  const dlg = page.getByTestId("format-cells");
  await expect(dlg).toBeVisible();
  await expect(option(page, "numbers-category", "date")).toHaveAttribute("aria-selected", "true");
  await expect(option(page, "numbers-format", "MM/DD/YYYY")).toHaveAttribute("aria-selected", "true");
  await expect(dlg.getByTestId("cells-preview")).toHaveText("10/19/2026");
  // Options do not apply to dates.
  for (const id of ["numbers-decimals", "numbers-leading", "numbers-red", "numbers-thousands"]) await expect(dlg.getByTestId(id)).toBeDisabled();
  // German: the same entry of the German list, with the language in the code.
  await dlg.getByTestId("numbers-language").selectOption("407");
  await expect(dlg.getByTestId("cells-format-code")).toHaveValue("[$-407]DD.MM.YYYY");
  await expect(option(page, "numbers-format", "[$-407]DD.MM.YYYY")).toHaveAttribute("aria-selected", "true");
  await expect(dlg.getByTestId("cells-preview")).toHaveText("19.10.2026");
  // Cancel discards.
  await dlg.getByRole("button", { name: "Cancel" }).click();
  await expect(dlg).toBeHidden();
  expect((await recorded(page)).length).toBe(before);
});

test("keyboard only: category, format and decimals, then OK", async ({ page }) => {
  await ready(page);
  await typeInto(page, 4, 7, "0.25");
  await clickCell(page, 4, 7);
  await page.keyboard.press("Meta+1");
  const dlg = page.getByTestId("format-cells");
  await expect(dlg).toBeVisible();
  const focusIn = (testId: string) => page.evaluate((id) => !!document.activeElement?.closest(`[data-testid="${id}"]`) || document.activeElement?.getAttribute("data-testid") === id, testId);
  const tabTo = async (testId: string) => {
    for (let i = 0; i < 25 && !(await focusIn(testId)); i++) await page.keyboard.press("Tab");
    expect(await focusIn(testId), testId).toBe(true);
  };
  await tabTo("numbers-category");
  await page.keyboard.press("Home");
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("ArrowDown");
  await expect(option(page, "numbers-category", "percent")).toHaveAttribute("aria-selected", "true");
  await expect(dlg.getByTestId("cells-format-code")).toHaveValue("0%");
  await tabTo("numbers-format");
  await page.keyboard.press("ArrowDown");
  await expect(dlg.getByTestId("cells-format-code")).toHaveValue("0.00%");
  await tabTo("numbers-decimals");
  await page.keyboard.press("ArrowUp");
  await expect(dlg.getByTestId("cells-format-code")).toHaveValue("0.000%");
  await tabTo("numbers-red");
  await page.keyboard.press("Space");
  await expect(dlg.getByTestId("cells-format-code")).toHaveValue("0.000%;[RED]-0.000%");
  await expect(dlg.getByTestId("cells-preview")).toHaveText("25.000%");
  await tabTo("dialog-ok");
  await page.keyboard.press("Enter");
  await expect(dlg).toBeHidden();
  expect((await recorded(page)).at(-1)).toMatchObject({ type: "applyStyle", patch: { numberFormat: "0.000%;[RED]-0.000%" } });
});

test("axe: the Numbers tab (Currency selected)", async ({ page }) => {
  await ready(page);
  await page.keyboard.press("Meta+1");
  const dlg = page.getByTestId("format-cells");
  await expect(dlg).toBeVisible();
  await option(page, "numbers-category", "currency").click();
  await expect(dlg.getByTestId("numbers-currency")).toBeVisible();
  await page.waitForFunction(() => document.getAnimations().every((a) => a.playState !== "running"));
  const result = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "best-practice"]).include("[data-testid=format-cells]").analyze();
  const serious = result.violations.filter((v) => v.impact === "serious" || v.impact === "critical").map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ")).join(", ")}`);
  expect(serious).toEqual([]);
});
