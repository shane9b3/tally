/**
 * Multi-sheet selection, UI side: the tab gestures (⌘/Ctrl-click, Shift-click,
 * a plain click inside and outside the group), the keys (⇧⌃PgDn extends,
 * ⌃PgDn collapses, Shift+→ in the tab list), the status bar count, the
 * transaction group sent with an edit, the group tab menu, the guarded
 * commands and the print range. The engine side (the edit reaching every
 * grouped sheet) runs on the browser mock.
 */
import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

async function ready(page: Page) {
  await page.goto("/");
  await expect(page.getByTestId("grid")).toBeVisible();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);
}

/** Four sheets, the first active; records every transaction's group on `window.__groups`. */
async function fourSheets(page: Page): Promise<number[]> {
  return page.evaluate(async () => {
    const t = window.__tally!;
    const c = t.controller;
    for (let i = 0; i < 3; i++) await c.addSheet();
    const ids = c.info!.sheets.map((s) => s.id);
    c.showSheet(ids[0]!, false);
    const w = window as unknown as { __groups: (number[] | null)[] };
    w.__groups = [];
    const client = t.client as unknown as { apply: (doc: number, tx: { group?: number[] | null }) => Promise<unknown> };
    const apply = client.apply.bind(client);
    client.apply = (doc, tx) => {
      w.__groups.push(tx.group ?? null);
      return apply(doc, tx);
    };
    return ids;
  });
}

const tabs = (page: Page) => page.getByRole("tablist", { name: "Sheets" }).getByRole("tab");
const sheet = (page: Page) => page.evaluate(() => window.__tally!.controller.sheet);
const selected = (page: Page) => page.evaluate(() => window.__tally!.controller.selectedSheets);
const enabled = (page: Page, id: string) => page.evaluate((id) => window.__tally!.controller.registry.isEnabled(id), id);

test("tab gestures select several sheets; the status bar counts them", async ({ page }) => {
  await ready(page);
  const ids = await fourSheets(page);
  const list = page.getByRole("tablist", { name: "Sheets" });
  await expect(list).toHaveAttribute("aria-multiselectable", "true");
  await expect(tabs(page)).toHaveCount(4);
  await expect(page.getByTestId("stat-sheets")).toHaveCount(0);

  // ⌘/Ctrl-click adds a tab; the active one stays active.
  await tabs(page).nth(2).click({ modifiers: ["ControlOrMeta"] });
  await expect.poll(() => selected(page)).toEqual([ids[0], ids[2]]);
  expect(await sheet(page)).toBe(ids[0]);
  await expect(tabs(page).nth(2)).toHaveAttribute("aria-selected", "true");
  await expect(tabs(page).nth(1)).toHaveAttribute("aria-selected", "false");
  await expect(tabs(page).nth(2)).toHaveClass(/grouped/);
  await expect(page.getByTestId("stat-sheets")).toHaveText("2 sheets selected");

  // ⌘/Ctrl-click again removes it; the active tab can't be removed.
  await tabs(page).nth(2).click({ modifiers: ["ControlOrMeta"] });
  await expect.poll(() => selected(page)).toEqual([ids[0]]);
  await tabs(page).nth(0).click({ modifiers: ["ControlOrMeta"] });
  await expect.poll(() => selected(page)).toEqual([ids[0]]);

  // Shift-click selects the run from the active tab.
  await tabs(page).nth(2).click({ modifiers: ["Shift"] });
  await expect.poll(() => selected(page)).toEqual([ids[0], ids[1], ids[2]]);
  await expect(page.getByTestId("stat-sheets")).toHaveText("3 sheets selected");

  // A click inside the group makes that tab active and keeps the group.
  await tabs(page).nth(1).click();
  await expect.poll(() => sheet(page)).toBe(ids[1]);
  await expect.poll(() => selected(page)).toEqual([ids[0], ids[1], ids[2]]);
  await expect(tabs(page).nth(1)).toHaveAttribute("tabindex", "0");

  // A click outside it selects only that tab.
  await tabs(page).nth(3).click();
  await expect.poll(() => selected(page)).toEqual([ids[3]]);
  await expect(page.getByTestId("stat-sheets")).toHaveCount(0);
});

test("keys: ⇧⌃PgDn extends, ⌃PgUp collapses, Shift+→ in the tab list extends", async ({ page }) => {
  await ready(page);
  const ids = await fourSheets(page);
  await page.getByTestId("grid").click();
  await page.keyboard.press("Control+Shift+PageDown");
  await page.keyboard.press("Control+Shift+PageDown");
  await expect.poll(() => selected(page)).toEqual([ids[0], ids[1], ids[2]]);
  expect(await sheet(page)).toBe(ids[2]);
  await page.keyboard.press("Control+PageUp");
  await expect.poll(() => selected(page)).toEqual([ids[1]]);

  await tabs(page).nth(1).focus();
  await page.keyboard.press("Shift+ArrowRight");
  await expect.poll(() => selected(page)).toEqual([ids[1], ids[2]]);
  await expect(tabs(page).nth(2)).toBeFocused();
  await page.keyboard.press("ArrowLeft");
  await expect.poll(() => selected(page)).toEqual([ids[1]]);
  await expect(tabs(page).nth(1)).toBeFocused();
});

test("an edit carries the group and reaches every selected sheet", async ({ page }) => {
  await ready(page);
  const ids = await fourSheets(page);
  await tabs(page).nth(1).click({ modifiers: ["ControlOrMeta"] });
  await tabs(page).nth(3).click({ modifiers: ["ControlOrMeta"] });
  await tabs(page).nth(3).click();
  await expect.poll(() => sheet(page)).toBe(ids[3]);
  await page.evaluate(() => window.__tally!.controller.apply("Typing", [{ type: "setCellInput", sheet: window.__tally!.controller.sheet, r: 0, c: 0, input: "42" }]));
  const groups = await page.evaluate(() => (window as unknown as { __groups: (number[] | null)[] }).__groups);
  expect(groups.at(-1)).toEqual([ids[3], ids[0], ids[1]]);
  const a1 = (s: number) =>
    page.evaluate(async (s) => {
      const t = window.__tally!;
      const [block] = await t.client.cells(t.controller.info!.doc, s, [{ r0: 0, c0: 0, r1: 0, c1: 0 }]);
      return block?.cells.find((v) => v.r === 0 && v.c === 0)?.text ?? "";
    }, s);
  await expect.poll(() => a1(ids[1]!)).toBe("42");
  expect(await a1(ids[2]!)).toBe("");
});

test("the tab menu acts on the group; guarded commands are off", async ({ page }) => {
  await ready(page);
  const ids = await fourSheets(page);
  expect(await enabled(page, "edit.fillSheet")).toBe(false);
  await tabs(page).nth(1).click({ modifiers: ["Shift"] });
  await expect.poll(() => selected(page)).toEqual([ids[0], ids[1]]);
  for (const id of ["data.sortAsc", "data.filter", "insert.chart", "data.defineName", "sheet.insert", "sheet.rename"]) expect(await enabled(page, id), id).toBe(false);
  expect(await enabled(page, "edit.fillSheet")).toBe(true);
  await expect(page.getByRole("button", { name: "Add sheet" })).toHaveAttribute("aria-disabled", "true");

  await tabs(page).nth(1).click({ button: "right" });
  const menu = page.getByRole("menu", { name: "Sheet" });
  await expect(menu.getByRole("menuitem", { name: "Rename…" })).toHaveAttribute("aria-disabled", "true");
  await expect(menu.getByRole("menuitem", { name: "Insert sheet" })).toHaveAttribute("aria-disabled", "true");
  await expect(menu.getByRole("menuitem", { name: "Hide 2 sheets" })).toBeVisible();
  await menu.getByRole("menuitem", { name: "Delete 2 sheets" }).click();
  await expect(tabs(page)).toHaveCount(2);
  await expect.poll(() => selected(page)).toEqual([ids[2]]);
  // One undo step brings both back.
  await page.evaluate(() => window.__tally!.controller.undo());
  await expect(tabs(page)).toHaveCount(4);

  await tabs(page).nth(0).click({ button: "right" });
  await page.getByRole("menu", { name: "Sheet" }).getByRole("menuitem", { name: "Select all sheets", exact: true }).click();
  await expect(page.getByTestId("stat-sheets")).toHaveText("4 sheets selected");
  await tabs(page).nth(0).click({ button: "right" });
  await expect(page.getByRole("menu", { name: "Sheet" }).getByRole("menuitem", { name: "Delete 4 sheets" })).toHaveAttribute("aria-disabled", "true");
  await page.getByRole("menu", { name: "Sheet" }).getByRole("menuitem", { name: "Deselect all sheets" }).click();
  await expect(page.getByTestId("stat-sheets")).toHaveCount(0);
});

test('print preview of "Selected sheets" prints the group', async ({ page }) => {
  await ready(page);
  const ids = await fourSheets(page);
  await tabs(page).nth(2).click({ modifiers: ["ControlOrMeta"] });
  await page.evaluate(() => window.__tally!.controller.registry.run("file.printPreview"));
  await expect(page.getByTestId("print-preview")).toBeVisible();
  const prepared = await page.evaluate(() =>
    JSON.parse(JSON.stringify((window.__tally!.client as unknown as { recordedCalls: { type: string; opts?: { sheets: unknown } }[] }).recordedCalls.filter((c) => c.type === "preparePrintJob"))),
  );
  expect(prepared.at(-1)?.opts?.sheets).toEqual({ type: "some", ids: [ids[0], ids[2]] });
});

test("grouped tabs and the status count pass axe (contrast included)", async ({ page }) => {
  await ready(page);
  await fourSheets(page);
  await tabs(page).nth(2).click({ modifiers: ["Shift"] });
  await expect(page.getByTestId("stat-sheets")).toBeVisible();
  for (const theme of ["light", "dark"]) {
    await page.evaluate((theme) => document.documentElement.setAttribute("data-theme", theme), theme);
    await page.waitForFunction(() => document.getAnimations().every((a) => a.playState !== "running"));
    const result = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "best-practice"]).include('[role="tablist"]').include('[data-testid="status-bar"]').analyze();
    const serious = result.violations.filter((v) => v.impact === "serious" || v.impact === "critical").map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ")).join(", ")}`);
    expect(serious, theme).toEqual([]);
  }
});

test("grouped: existing conditional formats can't be edited, new rules can; Move sheet right moves the group", async ({ page }) => {
  await ready(page);
  const ids = await fourSheets(page);
  await tabs(page).nth(2).click({ modifiers: ["ControlOrMeta"] });
  await page.evaluate(() => window.__tally!.controller.v3.highlight("duplicates"));
  const groups = await page.evaluate(() => (window as unknown as { __groups: (number[] | null)[] }).__groups);
  expect(groups.at(-1)).toEqual([ids[0], ids[2]]);
  await page.evaluate(() => window.__tally!.controller.registry.run("format.conditional"));
  const dlg = page.getByTestId("cf-manager");
  await expect(dlg).toBeVisible();
  await expect(page.getByTestId("cf-grouped")).toHaveText("Not available with several sheets selected");
  await expect(page.getByTestId("cf-edit-0")).toBeDisabled();
  await expect(page.getByTestId("cf-delete-0")).toBeDisabled();
  await expect(page.getByTestId("cf-new")).toBeEnabled();
  await page.keyboard.press("Escape");
  await expect(dlg).toBeHidden();

  await page.evaluate(() => window.__tally!.controller.registry.run("sheet.moveRight"));
  await expect.poll(() => page.evaluate(() => window.__tally!.controller.info!.sheets.map((s) => s.id))).toEqual([ids[1], ids[0], ids[2], ids[3]]);
  await expect.poll(() => selected(page)).toEqual([ids[0], ids[2]]);
});
