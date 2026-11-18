/**
 * The in-app menu bar of the Windows title bar (`?titlebar=1` mounts the
 * title bar over a mock shell whose menu model is the desktop shell's own,
 * `packages/app/src/dev/menuModel.json`): mouse and keyboard, the pop-ups
 * over the sheet, the focus coming back, the app's enabled state, axe.
 * The native side (menu.rs: the model, `menu_activate`) has Rust tests; a
 * real Windows build is checked by hand (docs/TODO.md).
 */
import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

async function ready(page: Page, url = "/?titlebar=1") {
  await page.goto(url);
  await expect(page.getByTestId("grid")).toBeVisible();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);
  await expect(page.getByRole("menubar").getByRole("menuitem")).toHaveCount(10);
  await page.evaluate(() => window.__tally!.controller.focusGrid());
}

const bar = (page: Page) => page.getByRole("menubar");
const top = (page: Page, name: string) => bar(page).getByRole("menuitem", { name, exact: true });
const menu = (page: Page, name: string) => page.getByRole("menu", { name, exact: true });
const item = (page: Page, menuName: string, name: string) => menu(page, menuName).getByRole("menuitem", { name, exact: true });
const activated = (page: Page) => page.evaluate(() => window.__tally!.titleBar!.activated.slice());
const focusInGrid = (page: Page) => page.evaluate(() => !!document.activeElement?.closest(".tally-grid"));
const focusedId = (page: Page) => page.evaluate(() => (document.activeElement as HTMLElement | null)?.dataset.id ?? null);

test("the title bar holds the menus, the title and the caption buttons above the app", async ({ page }) => {
  await ready(page);
  const strip = await page.getByTestId("title-bar").boundingBox();
  expect(strip).toMatchObject({ x: 0, y: 0, width: 1280, height: 32 });
  await expect(bar(page).getByRole("menuitem")).toHaveText(["File", "Edit", "Insert", "Format", "Data", "View", "Sheet", "Tools", "Window", "Help"]);
  // The app fills the rest of the window: nothing is pushed below the fold.
  const grid = await page.getByTestId("grid").boundingBox();
  expect(grid!.y).toBeGreaterThan(32);
  expect(await page.evaluate(() => document.documentElement.scrollHeight)).toBe(820);
  // Menus left, the title (the drag region) in the remaining space, caption buttons right.
  const help = (await top(page, "Help").boundingBox())!;
  const title = (await page.locator(".tally-titlebar-title").boundingBox())!;
  const minimize = (await page.getByRole("button", { name: "Minimize" }).boundingBox())!;
  expect(help.x + help.width).toBeLessThanOrEqual(title.x);
  expect(title.x + title.width).toBeLessThanOrEqual(minimize.x);
  expect(title.width).toBeGreaterThan(300);
  await expect(page.locator(".tally-titlebar-title")).toHaveAttribute("data-tauri-drag-region", "true");
  expect(await bar(page).evaluate((el) => el.closest("[data-tauri-drag-region]")?.getAttribute("data-tauri-drag-region"))).toBe("false");
  // Caption buttons.
  await page.getByRole("button", { name: "Maximize" }).click();
  await expect(page.getByRole("button", { name: "Restore" })).toBeVisible();
  await page.getByRole("button", { name: "Minimize" }).click();
  expect(await page.evaluate(() => window.__tally!.titleBar!.windowCalls)).toEqual(["toggleMaximize", "minimize"]);
});

test("mouse: click opens over the sheet, hover switches, an item runs its command", async ({ page }) => {
  await ready(page);
  await top(page, "Format").click();
  await expect(menu(page, "Format")).toBeVisible();
  await expect(top(page, "Format")).toHaveAttribute("aria-expanded", "true");
  // The pop-up hangs below its bar item and paints over the document area.
  const box = (await menu(page, "Format").boundingBox())!;
  const anchor = (await top(page, "Format").boundingBox())!;
  expect(Math.round(box.x)).toBe(Math.round(anchor.x));
  expect(box.y).toBeGreaterThanOrEqual(anchor.y + anchor.height - 1);
  expect(box.y).toBeLessThanOrEqual(32);
  const grid = (await page.getByTestId("grid").boundingBox())!;
  expect(box.y + box.height).toBeGreaterThan(grid.y + 40);
  const onTop = await page.evaluate(([x, y]) => !!document.elementFromPoint(x!, y!)?.closest('[role="menu"]'), [box.x + 30, Math.max(grid.y + 20, box.y + 20)]);
  expect(onTop).toBe(true);
  // Hovering another bar item switches menus.
  await top(page, "View").hover();
  await expect(menu(page, "View")).toBeVisible();
  await expect(menu(page, "Format")).toHaveCount(0);
  // A long menu stays inside the window and scrolls.
  const view = (await menu(page, "View").boundingBox())!;
  expect(view.y + view.height).toBeLessThanOrEqual(820);
  // A submenu opens on hover, beside its parent, and its item runs.
  await item(page, "View", "Toolbars").hover();
  await expect(menu(page, "Toolbars")).toBeVisible();
  const sub = (await menu(page, "Toolbars").boundingBox())!;
  expect(sub.x).toBeGreaterThan(view.x + view.width - 10);
  await expect(page.getByTestId("formula-input")).toBeVisible();
  await item(page, "Toolbars", "Formula Bar").click();
  await expect(page.getByRole("menu")).toHaveCount(0);
  await expect(page.getByTestId("formula-input")).toHaveCount(0);
  expect(await activated(page)).toEqual(["view.formulaBar"]);
  // The focus is back in the grid, not left in the bar.
  expect(await focusInGrid(page)).toBe(true);
  // Shortcut text is shown right-aligned and is not part of the item's name.
  await top(page, "File").click();
  await expect(item(page, "File", "Save")).toHaveAttribute("aria-keyshortcuts", "Ctrl+S");
  const label = (await item(page, "File", "Save").locator("span").nth(1).boundingBox())!;
  const accel = (await item(page, "File", "Save").locator("span").nth(2).boundingBox())!;
  expect(accel.x).toBeGreaterThan(label.x + 60);
  // A click elsewhere closes the menu.
  await page.getByTestId("grid").click({ position: { x: 600, y: 400 } });
  await expect(page.getByRole("menu")).toHaveCount(0);
  // A second click on the bar item closes too.
  await top(page, "Data").click();
  await expect(menu(page, "Data")).toBeVisible();
  await top(page, "Data").click();
  await expect(page.getByRole("menu")).toHaveCount(0);
});

test("keyboard: Alt and F10 focus the bar; arrows, letters, Enter and Esc navigate", async ({ page }) => {
  await ready(page);
  await page.getByTestId("grid").click({ position: { x: 300, y: 200 } });
  await page.mouse.move(0, 0);
  await page.keyboard.press("Alt");
  await expect(top(page, "File")).toBeFocused();
  await expect(page.getByRole("menu")).toHaveCount(0);
  await page.keyboard.press("ArrowRight");
  await expect(top(page, "Edit")).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(menu(page, "Edit")).toBeVisible();
  expect(await focusedId(page)).toBe("edit.undo");
  await page.keyboard.press("End");
  expect(await focusedId(page)).toBe("edit.editMode");
  await page.keyboard.press("Home");
  // Letters: Fill, then Find…, then Find & Replace….
  await page.keyboard.press("f");
  expect(await focusedId(page)).toBe("menu/Edit/Fill");
  await page.keyboard.press("ArrowRight");
  await expect(menu(page, "Fill")).toBeVisible();
  expect(await focusedId(page)).toBe("edit.fillDown");
  await page.keyboard.press("Escape");
  await expect(menu(page, "Fill")).toHaveCount(0);
  expect(await focusedId(page)).toBe("menu/Edit/Fill");
  await page.keyboard.press("f");
  expect(await focusedId(page)).toBe("edit.find");
  // Right on a plain item: the next menu; Left: back.
  await page.keyboard.press("ArrowRight");
  await expect(menu(page, "Insert")).toBeVisible();
  await page.keyboard.press("ArrowLeft");
  await expect(menu(page, "Edit")).toBeVisible();
  await page.keyboard.press("f");
  await page.keyboard.press("f");
  expect(await focusedId(page)).toBe("edit.find");
  await page.keyboard.press("Enter");
  await expect(page.getByRole("menu")).toHaveCount(0);
  await expect(page.getByTestId("find-bar")).toBeVisible();
  expect(await activated(page)).toEqual(["edit.find"]);
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("find-bar")).toHaveCount(0);

  // F10, Esc twice: the menu closes, then the focus goes back to the grid.
  await page.evaluate(() => window.__tally!.controller.focusGrid());
  await page.keyboard.press("F10");
  await expect(top(page, "Edit")).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(menu(page, "Edit")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("menu")).toHaveCount(0);
  await expect(top(page, "Edit")).toBeFocused();
  await page.keyboard.press("Escape");
  expect(await focusInGrid(page)).toBe(true);
  // Alt again leaves the bar; Alt with a digit (the tab shortcuts) never enters it.
  await page.keyboard.press("Alt");
  await expect(top(page, "Edit")).toBeFocused();
  await page.keyboard.press("Alt");
  expect(await focusInGrid(page)).toBe(true);
  await page.keyboard.press("Alt+1");
  expect(await focusInGrid(page)).toBe(true);
  // Typing in the sheet is not swallowed afterwards.
  await page.keyboard.type("42");
  await page.keyboard.press("Enter");
  expect(await page.evaluate(() => window.__tally!.controller.selection.active.r)).toBeGreaterThan(0);
});

test("items follow the app: disabled commands, check marks, the OS items", async ({ page }) => {
  await ready(page);
  await top(page, "Edit").click();
  // Nothing to undo yet: the registry's state shows in the menu.
  await expect(item(page, "Edit", "Undo")).toHaveAttribute("aria-disabled", "true");
  // (Playwright waits for an enabled target: force the click a user can still make.)
  await item(page, "Edit", "Undo").click({ force: true });
  await expect(menu(page, "Edit")).toBeVisible();
  expect(await activated(page)).toEqual([]);
  await page.keyboard.press("Escape");
  await page.keyboard.press("Escape");
  await page.evaluate(() => window.__tally!.controller.focusGrid());
  await page.keyboard.type("7");
  await page.keyboard.press("Enter");
  await top(page, "Edit").click();
  await expect(item(page, "Edit", "Undo")).not.toHaveAttribute("aria-disabled", "true");
  await item(page, "Edit", "Undo").click();
  expect(await activated(page)).toEqual(["edit.undo"]);
  // Edit ▸ Select All (handled by the OS in a native menu) selects the sheet.
  await top(page, "Edit").click();
  await item(page, "Edit", "Select All").click();
  expect(await page.evaluate(() => window.__tally!.controller.selection.range)).toMatchObject({ r0: 0, c0: 0 });
  expect(await page.evaluate(() => window.__tally!.controller.selection.range.r1)).toBeGreaterThan(1000);
  // The Window menu lists the open documents with a check mark on the front one.
  await top(page, "Window").click();
  const docs = menu(page, "Window").getByRole("menuitemcheckbox");
  await expect(docs).toHaveText(["✓Untitled 1", "Budget & Plan.ods"]);
  await docs.nth(1).click();
  await top(page, "Window").click();
  await expect(menu(page, "Window").getByRole("menuitemcheckbox", { checked: true })).toHaveText("✓Budget & Plan.ods");
  expect(await activated(page)).toEqual(["edit.undo", "native.selectAll", "window.focus.doc-2"]);
});

test("the open menus pass axe", async ({ page }) => {
  await ready(page);
  await top(page, "File").click();
  await item(page, "File", "Export").hover();
  await expect(menu(page, "Export")).toBeVisible();
  await page.mouse.move(0, 0);
  await page.waitForFunction(() => document.getAnimations().every((a) => a.playState !== "running"));
  const result = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "best-practice"]).include('[data-testid="title-bar"]').analyze();
  const serious = result.violations.filter((v) => v.impact === "serious" || v.impact === "critical");
  expect(serious.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ")).join(", ")}`)).toEqual([]);
  // The whole page with the bar closed.
  await page.keyboard.press("Escape");
  await page.keyboard.press("Escape");
  const page2 = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "best-practice"]).analyze();
  const serious2 = page2.violations.filter((v) => v.impact === "serious" || v.impact === "critical");
  expect(serious2.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ")).join(", ")}`)).toEqual([]);
});
