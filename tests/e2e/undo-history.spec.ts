/**
 * Undo / redo history drop-downs and the undo steps option (AOO 4.1 parity,
 * U17; mock engine):
 * - the toolbar Undo arrow lists the steps next first; hovering or arrowing
 *   marks 1..n ("Actions to undo: n"); choosing undoes n steps at once;
 * - the Redo arrow does the same the other way, by keyboard alone;
 * - Tools ▸ Options ▸ Memory ▸ "Number of steps" limits the history of the
 *   open document at once;
 * - axe finds no serious violations with the list open.
 */
import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

async function ready(page: Page, url = "/") {
  await page.goto(url);
  await expect(page.getByTestId("grid")).toBeVisible();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);
}

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

async function typeFive(page: Page) {
  for (let i = 0; i < 5; i++) await enter(page, i, 0, `v${i}`);
  await expect.poll(() => cellText(page, 4, 0)).toBe("v4");
}

test("the Undo drop-down undoes the marked steps at once; the Redo list works by keyboard", async ({ page }) => {
  await ready(page);
  await typeFive(page);

  await page.getByTestId("undo-history-arrow").click();
  const list = page.getByRole("listbox", { name: "Undo history" });
  await expect(list).toBeVisible();
  await expect(list).toBeFocused();
  const options = list.getByRole("option");
  await expect(options).toHaveCount(5);
  await expect(options.first()).toHaveAttribute("aria-selected", "true");
  await expect(page.getByTestId("undo-history-count")).toHaveText("Actions to undo: 1");
  // Hovering the third entry marks 1..3.
  await options.nth(2).hover();
  await expect(page.getByTestId("undo-history-count")).toHaveText("Actions to undo: 3");
  await expect(options.nth(1)).toHaveAttribute("aria-selected", "true");
  await expect(options.nth(3)).toHaveAttribute("aria-selected", "false");

  const axe = await new AxeBuilder({ page }).include('[data-testid="undo-history"]').withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "best-practice"]).analyze();
  expect(axe.violations.filter((v) => v.impact === "serious" || v.impact === "critical").map((v) => v.id)).toEqual([]);

  await options.nth(2).click();
  await expect(list).toBeHidden();
  await expect.poll(() => cellText(page, 1, 0)).toBe("v1");
  await expect.poll(() => cellText(page, 2, 0)).toBe("");
  await expect.poll(() => cellText(page, 4, 0)).toBe("");

  // Redo by keyboard: Tab to the arrow, open, ArrowDown twice, Enter.
  await page.getByTestId("redo-history-arrow").focus();
  await page.keyboard.press("Enter");
  const redo = page.getByRole("listbox", { name: "Redo history" });
  await expect(redo).toBeFocused();
  await expect(redo.getByRole("option")).toHaveCount(3);
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("ArrowDown");
  await expect(page.getByTestId("redo-history-count")).toHaveText("Actions to redo: 3");
  await page.keyboard.press("ArrowUp");
  await expect(page.getByTestId("redo-history-count")).toHaveText("Actions to redo: 2");
  await page.keyboard.press("Enter");
  await expect(redo).toBeHidden();
  await expect.poll(() => cellText(page, 3, 0)).toBe("v3");
  await expect.poll(() => cellText(page, 4, 0)).toBe("");

  // Escape closes without undoing.
  await page.getByTestId("undo-history-arrow").click();
  await expect(page.getByRole("listbox", { name: "Undo history" })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("listbox", { name: "Undo history" })).toBeHidden();
  expect(await cellText(page, 3, 0)).toBe("v3");
});

test("Options ▸ Memory ▸ Number of steps limits the open document's history", async ({ page }) => {
  await ready(page);
  await typeFive(page);
  await page.keyboard.press("Meta+,");
  const dlg = page.getByTestId("options-dialog");
  await expect(dlg).toBeVisible();
  await dlg.getByTestId("tab-memory").click();
  const steps = dlg.getByLabel("Number of steps");
  await expect(steps).toHaveValue("100");
  await steps.fill("2");
  await dlg.getByRole("button", { name: "OK" }).click();
  await expect(dlg).toBeHidden();

  await page.getByTestId("undo-history-arrow").click();
  await expect(page.getByRole("listbox", { name: "Undo history" }).getByRole("option")).toHaveCount(2);
  await page.keyboard.press("End");
  await page.keyboard.press("Enter");
  await expect.poll(() => cellText(page, 3, 0)).toBe("");
  // Older steps are gone: the third value stays.
  expect(await cellText(page, 2, 0)).toBe("v2");
  await expect(page.getByTestId("undo-history-arrow")).toHaveAttribute("aria-disabled", "true");
});
