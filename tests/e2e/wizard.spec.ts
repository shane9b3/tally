/**
 * Function Wizard (⌘F2) against the browser mock: argument fields come
 * from `FunctionInfo.params` (optional and repeating parameters), the
 * example is shown, the live result comes from `previewFormula`, and
 * Insert commits the exact call.
 */
import { expect, test, type Page } from "@playwright/test";

const ROW_H = 24;
const HEADER_H = 24;

async function ready(page: Page) {
  await page.goto("/");
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

async function typeColumn(page: Page, c: number, values: string[]) {
  for (const [r, v] of values.entries()) {
    await clickCell(page, r, c);
    await page.keyboard.type(v);
    await page.keyboard.press("Enter");
  }
}

test("SUMIF through the wizard: params fields, example, live result, exact formula", async ({ page }) => {
  await ready(page);
  await typeColumn(page, 0, ["1", "2", "3"]);
  await typeColumn(page, 1, ["10", "20", "30"]);
  await clickCell(page, 0, 2);
  await page.keyboard.press("Meta+F2");
  const dialog = page.getByTestId("function-wizard");
  await expect(dialog).toBeVisible();
  await page.getByTestId("wizard-category").selectOption({ label: "All" });
  await page.getByTestId("wizard-search").fill("sumif");
  await expect(page.getByTestId("wizard-list").getByRole("option", { selected: true })).toHaveText("SUMIF");
  await expect(page.getByTestId("wizard-example")).toContainText('=SUMIF(A1:A10;">5";B1:B10)');

  // One field per declared parameter; sum_range is optional.
  await expect(page.getByTestId("wizard-arg-label-0")).toContainText("range");
  await expect(page.getByTestId("wizard-arg-label-0")).toContainText("required");
  await expect(page.getByTestId("wizard-arg-label-1")).toContainText("criterion");
  await expect(page.getByTestId("wizard-arg-label-2")).toContainText("sum_range");
  await expect(page.getByTestId("wizard-arg-label-2")).toContainText("optional");
  await expect(page.getByTestId("wizard-arg-3")).toHaveCount(0);

  await page.getByTestId("wizard-arg-0").fill("A1:A3");
  await page.getByTestId("wizard-arg-1").fill('">1"');
  // Without sum_range the range itself is summed: 2 + 3.
  await expect(page.getByTestId("wizard-result")).toContainText("5");
  await page.getByTestId("wizard-arg-2").fill("B1:B3");
  await expect(page.getByTestId("wizard-call")).toHaveText('=SUMIF(A1:A3;">1";B1:B3)');
  await expect(page.getByTestId("wizard-result")).toContainText("50");

  await page.getByTestId("dialog-ok").click();
  await expect(dialog).toBeHidden();
  await clickCell(page, 0, 2);
  await expect(page.getByTestId("formula-input")).toHaveValue('=SUMIF(A1:A3;">1";B1:B3)');
});

test("a repeating parameter grows a new field as the last one fills (SUM)", async ({ page }) => {
  await ready(page);
  await typeColumn(page, 0, ["4", "5"]);
  await clickCell(page, 3, 3);
  await page.keyboard.press("Meta+F2");
  const dialog = page.getByTestId("function-wizard");
  await expect(dialog).toBeVisible();
  await page.getByTestId("wizard-category").selectOption({ label: "All" });
  await page.getByTestId("wizard-search").fill("sum");
  await expect(page.getByTestId("wizard-list").getByRole("option", { selected: true })).toHaveText("SUM");
  await expect(page.getByTestId("wizard-arg-label-0")).toContainText("number1");
  await expect(page.getByTestId("wizard-arg-1")).toHaveCount(0);
  await page.getByTestId("wizard-arg-0").fill("A1");
  await expect(page.getByTestId("wizard-arg-label-1")).toContainText("number2");
  await expect(page.getByTestId("wizard-arg-label-1")).toContainText("optional");
  await page.getByTestId("wizard-arg-1").fill("A2");
  await expect(page.getByTestId("wizard-arg-label-2")).toContainText("number3");
  await expect(page.getByTestId("wizard-result")).toContainText("9");
  await page.getByTestId("dialog-ok").click();
  await expect(dialog).toBeHidden();
  await clickCell(page, 3, 3);
  await expect(page.getByTestId("formula-input")).toHaveValue("=SUM(A1;A2)");
});
