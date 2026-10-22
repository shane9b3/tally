/**
 * Font pickers in the browser build (MockClient lists FALLBACK_FONTS): the
 * toolbar font box applies a family to a range as one ApplyStyle and undo
 * restores it; the size box takes typed half points; Format Cells ▸ Font
 * sets Bold Italic through the Typeface list.
 */
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

async function clickCell(page: Page, r: number, c: number, modifier?: "Shift") {
  const box = await page.getByTestId("grid").boundingBox();
  if (!box) throw new Error("grid not visible");
  const { headerW, headerH, xs, w } = await page.evaluate((c) => {
    const ctl = window.__tally!.controller;
    let x = 0;
    for (let i = 0; i < c; i++) x += ctl.layoutSize("col", i);
    const geo = ctl.grid!.getGeometry();
    return { headerW: geo.headerW, headerH: geo.headerH, xs: x, w: ctl.layoutSize("col", c) };
  }, c);
  if (modifier) await page.keyboard.down(modifier);
  await page.mouse.click(box.x + headerW + xs + w / 2, box.y + headerH + r * ROW_H + ROW_H / 2);
  if (modifier) await page.keyboard.up(modifier);
}

const recorded = (page: Page): Promise<Rec[]> =>
  page.evaluate(() => JSON.parse(JSON.stringify((window.__tally!.client as unknown as { recorded: unknown[] }).recorded)) as Rec[]);

const familyAt = (page: Page, r: number, c: number): Promise<string | null> =>
  page.evaluate(
    async ([r, c]) => {
      const t = window.__tally!;
      const doc = t.controller.info!.doc;
      const [b] = await t.client.cells(doc, t.controller.sheet, [{ r0: r, c0: c, r1: r, c1: c }]);
      const [s] = await t.client.styles(doc, [b?.cells[0]?.style ?? 0]);
      return s?.fontFamily ?? null;
    },
    [r, c] as const,
  );

test("the toolbar font box applies a font to a range; undo restores it", async ({ page }) => {
  await ready(page);
  await clickCell(page, 1, 0);
  await clickCell(page, 3, 2, "Shift");
  const box = page.getByTestId("font-name-input");
  await expect(box).toHaveValue("Inter");
  await page.getByTestId("font-name-open").click();
  const list = page.getByTestId("font-name-list");
  await expect(list).toBeVisible();
  await expect(list.getByRole("option", { name: "Georgia" }).first()).toBeVisible();
  await box.fill("geo");
  await expect(list.getByRole("option")).toHaveText(["Georgia"]);
  await box.press("Enter");
  await expect(list).toBeHidden();
  await expect.poll(() => page.evaluate(() => document.activeElement?.closest('[data-testid="grid"]') != null)).toBe(true);
  const ops = await recorded(page);
  expect(ops.at(-1)).toMatchObject({ type: "applyStyle", ranges: [{ r0: 1, c0: 0, r1: 3, c1: 2 }], patch: { fontFamily: "Georgia", bold: null, fontSizePt: null } });
  await expect.poll(() => familyAt(page, 2, 1)).toBe("Georgia");
  await expect(box).toHaveValue("Georgia");
  // Recently used fonts lead the list next time.
  await page.getByTestId("font-name-open").click();
  await expect(page.getByTestId("font-name-list").locator('[role="presentation"]').first()).toHaveText("Recently used");
  await page.keyboard.press("Escape");
  await page.keyboard.press("Escape");
  await page.keyboard.press("Meta+z");
  await expect.poll(() => familyAt(page, 2, 1)).toBeNull();
  await expect(box).toHaveValue("Inter");
  // The size box takes a typed value, with a decimal comma too; focus goes back to the grid.
  const size = page.getByTestId("font-size-input");
  await size.fill("10,5");
  await size.press("Enter");
  expect((await recorded(page)).at(-1)).toMatchObject({ type: "applyStyle", patch: { fontSizePt: 10.5 } });
  await expect(size).not.toBeFocused();
  await expect.poll(() => page.evaluate(() => document.activeElement?.closest('[data-testid="grid"]') != null)).toBe(true);
  // Over 409 pt: refused, nothing applied.
  const n = (await recorded(page)).length;
  await size.fill("500");
  await size.press("Enter");
  expect((await recorded(page)).length).toBe(n);
  await size.press("Escape");
  await size.press("Escape");
  // Enter on the unchanged font box: no op, no undo step.
  await box.click();
  await box.press("Enter");
  expect((await recorded(page)).length).toBe(n);
});

test("Format Cells ▸ Font: typeface Bold Italic, a font that is not installed", async ({ page }) => {
  await ready(page);
  await clickCell(page, 2, 1);
  await page.keyboard.press("Meta+1");
  const dlg = page.getByTestId("format-cells");
  await expect(dlg).toBeVisible();
  await dlg.getByTestId("tab-font").click();
  await expect(dlg.getByTestId("cells-font")).toHaveValue("Inter");
  await expect(dlg.getByTestId("cells-typeface").getByRole("option")).toHaveText(["Regular", "Italic", "Bold", "Bold Italic"]);
  await dlg.getByTestId("cells-typeface").getByRole("option", { name: "Bold Italic" }).click();
  await expect(dlg.getByTestId("cells-typeface").getByRole("option", { name: "Bold Italic" })).toHaveAttribute("aria-selected", "true");
  await expect(dlg.getByTestId("cells-font-note")).not.toContainText("not installed");
  await dlg.getByTestId("cells-font").fill("Nonexistent Sans");
  await expect(dlg.getByTestId("cells-font-note")).toContainText("This font is not installed; a substitute will be used.");
  // The field can be emptied (it means the document font) and shows what is typed.
  await dlg.getByTestId("cells-font").fill("");
  await expect(dlg.getByTestId("cells-font")).toHaveValue("");
  await dlg.getByTestId("cells-font").pressSequentially("Ar");
  await expect(dlg.getByTestId("cells-font")).toHaveValue("Ar");
  await dlg.getByTestId("cells-font").fill("Verdana");
  await expect(dlg.getByTestId("cells-font-note")).not.toContainText("not installed");
  await expect(dlg.getByTestId("cells-font-preview").locator("span")).toHaveCSS("font-style", "italic");
  await dlg.getByTestId("dialog-ok").click();
  const ops = await recorded(page);
  expect(ops.at(-1)).toMatchObject({ type: "applyStyle", patch: { bold: true, italic: true, fontFamily: "Verdana", fontSizePt: null, underline: null } });
});
