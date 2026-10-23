/**
 * Keyboard-only dialogs (1.0 a11y pass): each dialog opens by its shortcut
 * or from the command palette (⌘K), takes the focus, keeps Tab / Shift+Tab
 * inside with a visible focus ring on every stop, closes with Escape and
 * gives the focus back to where it came from. axe checks: axe.spec.ts;
 * VoiceOver checklist: docs/qa/a11y.md.
 */
import { expect, test, type Page } from "@playwright/test";

async function ready(page: Page, url = "/?template=invoices") {
  await page.goto(url);
  await expect(page.getByTestId("grid")).toBeVisible();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);
  await page.evaluate(() => window.__tally!.controller.focusGrid());
}

const MODAL = "[role=dialog][aria-modal=true], [role=alertdialog][aria-modal=true]";

/** Runs a command from the palette by typing its title, as a keyboard user would. */
async function fromPalette(page: Page, command: string) {
  const title = await page.evaluate((id) => window.__tally!.controller.registry.get(id)?.title ?? id, command);
  await page.keyboard.press("ControlOrMeta+k");
  const input = page.getByTestId("palette-input");
  await expect(input).toBeFocused();
  await input.fill(title.replace(/…$/, ""));
  // ↓ to the item whose title matches exactly (others may match the text too).
  const selected = page.locator("[cmdk-item][data-selected=true]");
  await expect(selected).toBeVisible();
  for (let i = 0; i < 12; i++) {
    const text = (await selected.innerText()).trim();
    if (text === title || text.startsWith(`${title}\n`) || text.startsWith(title + " ")) break;
    await page.keyboard.press("ArrowDown");
  }
  await page.keyboard.press("Enter");
}

/** Where the focus is: inside the open modal, its ring, and whether the grid's editor has it. */
function focusState(page: Page) {
  return page.evaluate((sel) => {
    const a = document.activeElement as HTMLElement | null;
    const modal = [...document.querySelectorAll<HTMLElement>(sel)].pop() ?? null;
    let ring = false;
    if (a) {
      const cs = getComputedStyle(a);
      ring = (cs.outlineStyle !== "none" && parseFloat(cs.outlineWidth) > 0) || cs.boxShadow !== "none";
    }
    return {
      inside: !!(modal && a && modal.contains(a)),
      ring,
      what: a ? `${a.tagName.toLowerCase()}${a.getAttribute("role") ? `[role=${a.getAttribute("role")}]` : ""} "${(a.getAttribute("aria-label") ?? a.textContent ?? "").trim().slice(0, 30)}" ${a.getAttribute("data-testid") ?? ""}` : "none",
      inGrid: !!a?.closest(".tally-grid"),
    };
  }, MODAL);
}

const DIALOGS: readonly { name: string; open: string | { command: string } }[] = [
  { name: "Format Cells", open: "ControlOrMeta+1" },
  { name: "Options", open: "ControlOrMeta+Comma" },
  { name: "Function Wizard", open: "ControlOrMeta+F2" },
  { name: "Define Name", open: "ControlOrMeta+F3" },
  { name: "Find & Replace", open: "Alt+ControlOrMeta+KeyF" },
  { name: "Paste Special", open: "Alt+ControlOrMeta+KeyV" },
  { name: "Hyperlink", open: "Alt+ControlOrMeta+KeyK" },
  { name: "Keyboard shortcuts", open: "ControlOrMeta+Slash" },
  { name: "Page Setup", open: { command: "file.pageSetup" } },
  { name: "Customize Toolbars", open: { command: "view.customizeToolbars" } },
  { name: "Sort", open: { command: "data.sort" } },
  { name: "Standard Filter", open: { command: "data.standardFilter" } },
  { name: "Conditional formatting", open: { command: "format.conditional" } },
  { name: "Validity", open: { command: "data.validation" } },
  { name: "AutoFormat", open: { command: "format.autoFormat" } },
  { name: "DataPilot", open: { command: "data.dataPilot" } },
  { name: "Subtotals", open: { command: "data.subtotals" } },
  { name: "Goal Seek", open: { command: "tools.goalSeek" } },
  { name: "Properties", open: { command: "file.properties" } },
];

for (const d of DIALOGS) {
  test(`keyboard only: ${d.name}`, async ({ page }) => {
    await ready(page);
    if (typeof d.open === "string") await page.keyboard.press(d.open);
    else await fromPalette(page, d.open.command);
    const dialog = page.locator(MODAL).last();
    await expect(dialog).toBeVisible();
    // Named by its title.
    const name = await dialog.evaluate((el) => {
      const by = el.getAttribute("aria-labelledby");
      return (by ? document.getElementById(by)?.textContent : el.getAttribute("aria-label"))?.trim() ?? "";
    });
    expect(name).not.toBe("");
    // The focus starts inside.
    await expect.poll(async () => (await focusState(page)).inside).toBe(true);

    // Tab through every stop (and wrap): the focus stays inside and each stop shows a ring.
    const stops = await dialog.evaluate((el) => el.querySelectorAll('a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])').length);
    const seen = new Set<string>();
    for (let i = 0; i < Math.min(stops + 1, 60); i++) {
      await page.keyboard.press("Tab");
      const f = await focusState(page);
      expect(f.inside, `Tab #${i + 1} left the dialog (${f.what})`).toBe(true);
      expect(f.ring, `no focus ring on ${f.what}`).toBe(true);
      seen.add(f.what);
    }
    if (stops > 1) expect(seen.size).toBeGreaterThan(1);
    await page.keyboard.press("Shift+Tab");
    expect((await focusState(page)).inside, "Shift+Tab left the dialog").toBe(true);

    // Escape cancels and the focus returns to the grid (the opener).
    await page.keyboard.press("Escape");
    await expect(dialog).toBeHidden();
    await expect.poll(async () => (await focusState(page)).inGrid).toBe(true);
  });
}

test("keyboard only: a toolbar menu walks with the arrows; its dialog closes to the grid (as in Calc)", async ({ page }) => {
  await ready(page);
  const sort = page.locator('button[data-command="data.sort"]');
  await sort.focus();
  await page.keyboard.press("Enter");
  const menu = page.getByRole("menu");
  await expect(menu).toBeVisible();
  // ↓/↑/Home/End move between the items.
  await page.keyboard.press("End");
  await expect(menu.getByRole("menuitem").last()).toBeFocused();
  await page.keyboard.press("Home");
  await expect(menu.getByRole("menuitem").first()).toBeFocused();
  await page.keyboard.press("ArrowUp");
  await expect(menu.getByRole("menuitem", { name: "Sort…" })).toBeFocused();
  await page.keyboard.press("Enter");
  const dialog = page.getByTestId("sort-dialog");
  await expect(dialog).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  // Cancel sends the focus to the grid on purpose; it is not pulled back to the opener.
  await expect.poll(async () => (await focusState(page)).inGrid).toBe(true);
  await expect(sort).not.toBeFocused();

  // Escape on the menu itself (no command run) gives the focus back to its button.
  await sort.focus();
  await page.keyboard.press("Enter");
  await expect(menu).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(menu).toBeHidden();
  await expect(sort).toBeFocused();
});

test("keyboard only: Enter submits with the default button; tabs and radio groups follow the arrows", async ({ page }) => {
  await ready(page);
  // Options: ←/→ walk the tab list and the panel follows.
  await page.keyboard.press("ControlOrMeta+Comma");
  const options = page.getByTestId("options-dialog");
  await expect(options).toBeVisible();
  const tabs = options.getByRole("tab");
  await tabs.first().focus();
  await page.keyboard.press("ArrowRight");
  await expect(tabs.nth(1)).toBeFocused();
  await expect(tabs.nth(1)).toHaveAttribute("aria-selected", "true");
  await page.keyboard.press("End");
  await expect(tabs.last()).toHaveAttribute("aria-selected", "true");
  await page.keyboard.press("Escape");
  await expect(options).toBeHidden();

  // Define Name: type a name, Enter = OK.
  await page.keyboard.press("ControlOrMeta+F3");
  const define = page.locator(MODAL).last();
  await expect(define).toBeVisible();
  await page.keyboard.type("KeyboardName");
  await page.keyboard.press("Enter");
  await expect(define).toBeHidden();
  const names = await page.evaluate(async () => {
    const t = window.__tally!;
    return (await t.client.names(t.controller.info!.doc)).map((n) => n.name);
  });
  expect(names).toContain("KeyboardName");
});

test("keyboard only: the sheet tabs and the toolbar overflow", async ({ page }) => {
  await ready(page);
  // Sheet tabs: one tab stop, ←/→ switch sheets.
  await page.evaluate(() => window.__tally!.controller.addSheet());
  const tabs = page.getByRole("tablist", { name: "Sheets" }).getByRole("tab");
  await expect(tabs).toHaveCount(2);
  await tabs.nth(1).focus();
  await page.keyboard.press("ArrowLeft");
  await expect(tabs.first()).toHaveAttribute("aria-selected", "true");
  await expect(tabs.first()).toBeFocused();
  await expect(tabs.nth(1)).toHaveAttribute("tabindex", "-1");

  // A narrow window: "»" opens the rest of the bar from the keyboard and Esc comes back to it.
  await page.setViewportSize({ width: 640, height: 800 });
  const more = page.locator('button[aria-label="More buttons"]').first();
  await expect(more).toBeVisible();
  await more.focus();
  await page.keyboard.press("Enter");
  await expect(more).toHaveAttribute("aria-expanded", "true");
  const panelId = await more.getAttribute("aria-controls");
  const panel = page.locator(`#${panelId}`);
  await expect(panel).toHaveAttribute("role", "toolbar");
  await expect.poll(() => panel.evaluate((p) => p.contains(document.activeElement))).toBe(true);
  await page.keyboard.press("Escape");
  await expect(panel).toBeHidden();
  await expect(more).toBeFocused();
});
