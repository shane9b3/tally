/**
 * Paste Special to AOO 4.1 parity in the browser (WebMockClient emulates
 * the engine's `pasteSpecial` and records the ops): the Selection check
 * boxes, Link, Shift cells, operations as radio buttons, the presets and
 * the keyboard.
 */
import { expect, test, type Page } from "@playwright/test";

interface Rec {
  type: string;
  [k: string]: unknown;
}

async function ready(page: Page) {
  await page.goto("/");
  await expect(page.getByTestId("grid")).toBeVisible();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);
}

const recorded = (page: Page): Promise<Rec[]> =>
  page.evaluate(() => JSON.parse(JSON.stringify((window.__tally!.client as unknown as { recorded: unknown[] }).recorded)) as Rec[]);

/** Edit-line inputs of a block, row by row. */
async function inputs(page: Page, r0: number, c0: number, r1: number, c1: number): Promise<string[]> {
  return page.evaluate(
    async ([r0, c0, r1, c1]) => {
      const t = window.__tally!;
      const info = t.controller.info!;
      const out: string[] = [];
      for (let r = r0; r <= r1; r++) for (let c = c0; c <= c1; c++) out.push((await t.client.cellEditInfo(info.doc, t.controller.sheet, r, c)).input);
      return out;
    },
    [r0, c0, r1, c1] as const,
  );
}

/** A1:A3 = abc, 5, =A2*2; copies it and selects `target`. */
async function copySource(page: Page, target: string) {
  await page.evaluate(async (target) => {
    const ctl = window.__tally!.controller;
    await ctl.apply("setup", [{ type: "setRangeInputs", sheet: ctl.sheet, r: 0, c: 0, rows: [["abc"], ["5"], ["=A2*2"]] }]);
    await ctl.goTo("A1:A3");
    await ctl.copy();
    await ctl.goTo(target);
  }, target);
}

async function openDialog(page: Page) {
  await page.keyboard.press("ControlOrMeta+Alt+v");
  const dialog = page.getByTestId("paste-special-dialog");
  await expect(dialog).toBeVisible();
  return dialog;
}

test("Selection: Numbers only pastes the formula's value and no text", async ({ page }) => {
  await ready(page);
  await copySource(page, "C1");
  const dialog = await openDialog(page);
  // AOO's first-open state: Paste all off; Text, Numbers, Date & time on.
  await expect(dialog.getByTestId("paste-kind-all")).not.toBeChecked();
  for (const k of ["text", "numbers", "dateTime"]) await expect(dialog.getByTestId(`paste-kind-${k}`)).toBeChecked();
  for (const k of ["formulas", "notes", "formats"]) await expect(dialog.getByTestId(`paste-kind-${k}`)).not.toBeChecked();
  await dialog.getByTestId("paste-kind-all").check();
  await expect(dialog.getByTestId("paste-kind-text")).toBeDisabled();
  await dialog.getByTestId("paste-kind-all").uncheck();
  for (const k of ["text", "dateTime"]) await dialog.getByTestId(`paste-kind-${k}`).uncheck();
  await expect(dialog.getByTestId("paste-kind-numbers")).toBeChecked();
  await dialog.getByTestId("dialog-ok").click();
  await expect(dialog).toHaveCount(0);
  await expect.poll(async () => (await recorded(page)).at(-1)?.type).toBe("pasteSpecial");
  expect((await recorded(page)).at(-1)).toMatchObject({
    options: { selection: { text: false, numbers: true, dateTime: false, formulas: false, notes: false, formats: false }, operation: "none" },
  });
  await expect.poll(() => inputs(page, 0, 2, 2, 2)).toEqual(["", "5", "10"]);
  // The choices persist for the session.
  const again = await openDialog(page);
  await expect(again.getByTestId("paste-kind-text")).not.toBeChecked();
  await expect(again.getByTestId("paste-kind-numbers")).toBeChecked();
  await page.keyboard.press("Escape");
});

test("Link pastes absolute references that follow the source", async ({ page }) => {
  await ready(page);
  await copySource(page, "E1");
  const dialog = await openDialog(page);
  await dialog.getByTestId("paste-link").check();
  await dialog.getByTestId("dialog-ok").click();
  await expect.poll(() => inputs(page, 0, 4, 1, 4)).toEqual(["=$Sheet1.$A$1", "=$Sheet1.$A$2"]);
  expect((await recorded(page)).at(-1)).toMatchObject({ type: "pasteSpecial", options: { link: true } });
  await page.evaluate(() => window.__tally!.controller.apply("edit", [{ type: "setCellInput", sheet: window.__tally!.controller.sheet, r: 1, c: 0, input: "7" }]));
  await expect
    .poll(() =>
      page.evaluate(async () => {
        const t = window.__tally!;
        const [b] = await t.client.cells(t.controller.info!.doc, t.controller.sheet, [{ r0: 1, c0: 4, r1: 1, c1: 4 }]);
        return b?.cells[0]?.text;
      }),
    )
    .toBe("7");
});

test("Shift cells down moves the target first; one Undo restores", async ({ page }) => {
  await ready(page);
  await page.evaluate(() => window.__tally!.controller.apply("setup", [{ type: "setCellInput", sheet: window.__tally!.controller.sheet, r: 0, c: 6, input: "old" }]));
  await copySource(page, "G1");
  const dialog = await openDialog(page);
  await dialog.getByTestId("paste-shift-down").check();
  await dialog.getByTestId("paste-op-add").check();
  await dialog.getByTestId("dialog-ok").click();
  // No "pasting over data" question: the cells are new.
  await expect(dialog).toHaveCount(0);
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect((await recorded(page)).at(-1)).toMatchObject({ type: "pasteSpecial", options: { shift: "down", operation: "add" } });
  // AOO MixData: text is never combined (the new, empty cell stays empty); empty counts as 0.
  // Formulas are off by default, so the formula's value is added.
  await expect.poll(() => inputs(page, 0, 6, 3, 6)).toEqual(["", "5", "10", "old"]);
  await page.evaluate(() => window.__tally!.controller.undo());
  await expect.poll(() => inputs(page, 0, 6, 3, 6)).toEqual(["old", "", "", ""]);
});

test("presets, operations needing a cell kind, keyboard", async ({ page }) => {
  await ready(page);
  await copySource(page, "J1");
  const dialog = await openDialog(page);
  await dialog.getByTestId("paste-formats").click();
  await expect(dialog.getByTestId("paste-kind-all")).not.toBeChecked();
  await expect(dialog.getByTestId("paste-kind-formats")).toBeChecked();
  await expect(dialog.getByTestId("paste-kind-text")).not.toBeChecked();
  // AOO never disables the operations.
  await expect(dialog.getByTestId("paste-op-add")).toBeEnabled();
  await dialog.getByTestId("paste-values").click();
  await expect(dialog.getByTestId("paste-kind-numbers")).toBeChecked();
  await expect(dialog.getByTestId("paste-kind-formulas")).not.toBeChecked();
  await expect(dialog.getByTestId("paste-op-add")).toBeEnabled();
  // Arrow keys move between the operation radios.
  await dialog.getByTestId("paste-op-none").focus();
  await page.keyboard.press("ArrowDown");
  await expect(dialog.getByTestId("paste-op-add")).toBeChecked();
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  expect((await recorded(page)).filter((o) => o.type === "pasteSpecial")).toHaveLength(0);
  // Shift down is greyed out where the block would leave the sheet.
  await page.evaluate(() => window.__tally!.controller.goTo("A1048575"));
  const edge = await openDialog(page);
  await expect(edge.getByTestId("paste-shift-down")).toBeDisabled();
  await expect(edge.getByTestId("paste-shift-right")).toBeEnabled();
  await page.keyboard.press("Escape");
});
