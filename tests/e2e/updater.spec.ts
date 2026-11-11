/**
 * Auto-update UI against the browser mock's simulated update server
 * (`?update=<version>`, MockClient.updateHooks; the desktop shell's
 * updater.rs does the real check, download, signature check and install):
 * the automatic prompt and its release notes as plain text, Later, Skip
 * This Version (a manual check ignores it, a newer version prompts again),
 * Install and Restart (progress, then the restart request, after the
 * unsaved-changes guard), Check for Updates… when up to date and when
 * offline, and the Options ▸ Online Update switch.
 */
import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

interface Hooks {
  restartRequested: number;
}

async function ready(page: Page, url = "/") {
  await page.goto(url);
  await expect(page.getByTestId("grid")).toBeVisible();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);
}

const restarts = (page: Page): Promise<number> => page.evaluate(() => (window.__tally!.client as unknown as { updateHooks: Hooks }).updateHooks.restartRequested);

/** The native menu's Help ▸ Check for Updates… (the shell sends `menu-action`). */
const menuCheck = (page: Page) => page.evaluate(() => (window.__tally!.client as unknown as { emitMenuAction(id: string): void }).emitMenuAction("help.checkUpdates"));

async function axe(page: Page): Promise<string[]> {
  // After the overlay's fade-in (contrast is measured on the final colours).
  await page.waitForTimeout(300);
  const r = await new AxeBuilder({ page }).include('[data-testid="update-prompt"]').withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "best-practice"]).analyze();
  return r.violations.filter((v) => v.impact === "serious" || v.impact === "critical").map((v) => `${v.id}: ${v.help}`);
}

test("an available update prompts once with its notes as plain text; Later closes it", async ({ page }) => {
  await ready(page, "/?update=0.2.0");
  const dlg = page.getByTestId("update-prompt");
  await expect(dlg).toBeVisible();
  await expect(dlg).toHaveAttribute("role", "dialog");
  await expect(dlg.getByRole("heading")).toHaveText("Tally 0.2.0 is available — you have 0.1.0");
  await expect(dlg).toContainText("Released 2026-11-20.");
  // Untrusted text: the markup shows as written, nothing is rendered.
  const notes = dlg.getByTestId("update-notes");
  await expect(notes).toContainText("<b>Save As</b> keeps the format");
  expect(await notes.locator("b").count()).toBe(0);
  await expect(dlg.getByTestId("update-install")).toBeFocused();
  expect(await axe(page)).toEqual([]);
  // Keyboard: Tab stays in the dialog; Escape = Later.
  for (let i = 0; i < 6; i++) await page.keyboard.press("Tab");
  expect(await page.evaluate(() => document.activeElement?.closest('[data-testid="update-prompt"]') != null)).toBe(true);
  await page.keyboard.press("Escape");
  await expect(dlg).toBeHidden();
  await dlg.waitFor({ state: "hidden" });
  await page.getByTestId("grid").click();
  await page.waitForTimeout(800);
  await expect(dlg).toBeHidden();
  expect(await restarts(page)).toBe(0);
});

test("Skip This Version silences that version only; a manual check still offers it", async ({ page }) => {
  await ready(page, "/?update=0.2.0");
  const dlg = page.getByTestId("update-prompt");
  await expect(dlg).toBeVisible();
  await dlg.getByTestId("update-skip").click();
  await expect(dlg).toBeHidden();
  await ready(page, "/?update=0.2.0");
  await page.waitForTimeout(1200);
  await expect(dlg).toBeHidden();
  await menuCheck(page);
  await expect(dlg.getByRole("heading")).toHaveText("Tally 0.2.0 is available — you have 0.1.0");
  await dlg.getByTestId("update-later").click();
  await ready(page, "/?update=0.3.0");
  await expect(dlg.getByRole("heading")).toHaveText("Tally 0.3.0 is available — you have 0.1.0");
});

test("Install and Restart shows the download's progress, then requests the restart", async ({ page }) => {
  await ready(page, "/?update=0.2.0");
  await page.evaluate(() => {
    const h = (window.__tally!.client as unknown as { updateHooks: { downloadStepMs: number } }).updateHooks;
    h.downloadStepMs = 250;
  });
  const dlg = page.getByTestId("update-prompt");
  await dlg.getByTestId("update-install").click();
  await expect(dlg).toHaveAttribute("data-phase", "downloading");
  await expect(dlg.getByRole("heading")).toHaveText("Downloading Tally 0.2.0…");
  await expect(dlg.getByTestId("update-progress")).toBeVisible();
  await expect(dlg).toContainText("% downloaded");
  // Nothing to click while downloading; Escape does not close it.
  expect(await dlg.getByRole("button").count()).toBe(0);
  expect(await axe(page)).toEqual([]);
  await page.keyboard.press("Escape");
  await expect(dlg).toHaveAttribute("data-phase", "downloading");
  await expect.poll(() => restarts(page)).toBe(1);
  await expect(dlg).toBeHidden();
});

test("the restart goes through the unsaved-changes guard; Cancel keeps the update downloaded", async ({ page }) => {
  await ready(page, "/?update=0.2.0");
  const dlg = page.getByTestId("update-prompt");
  await expect(dlg).toBeVisible();
  await dlg.getByTestId("update-later").click();
  await page.evaluate(() => window.__tally!.controller.startEdit({ r: 0, c: 0 }, "42", "grid"));
  await page.evaluate(() => window.__tally!.controller.commitEdit("none"));
  await menuCheck(page);
  await dlg.getByTestId("update-install").click();
  const save = page.getByTestId("save-prompt");
  await expect(save).toBeVisible();
  await save.getByTestId("save-prompt-cancel").click();
  await expect(dlg).toHaveAttribute("data-phase", "restartCancelled");
  await expect(dlg).toContainText("Tally 0.2.0 is downloaded but not installed, and every document is still open.");
  expect(await restarts(page)).toBe(0);
  expect(await axe(page)).toEqual([]);
  // Install and Restart again, Don't Save this time: no second download.
  await dlg.getByTestId("update-install").click();
  await save.getByTestId("save-prompt-discard").click();
  await expect.poll(() => restarts(page)).toBe(1);
});

test("Check for Updates… when up to date, and when offline", async ({ page }) => {
  await ready(page);
  await page.waitForTimeout(600);
  const dlg = page.getByTestId("update-prompt");
  await expect(dlg).toBeHidden();
  await menuCheck(page);
  await expect(dlg.getByRole("heading")).toHaveText("Tally is up to date");
  await expect(dlg).toContainText("You have the newest version, 0.1.0.");
  expect(await axe(page)).toEqual([]);
  await expect(dlg.getByTestId("update-ok")).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(dlg).toBeHidden();

  await ready(page, "/?update=0.2.0&updateError=offline");
  await page.waitForTimeout(600);
  await expect(dlg).toBeHidden();
  await menuCheck(page);
  await expect(dlg).toHaveAttribute("role", "alertdialog");
  await expect(dlg.getByRole("heading")).toHaveText("Tally could not check for updates");
  await expect(dlg.getByTestId("update-error")).toHaveText("offline");
  await dlg.getByTestId("update-ok").click();
  await expect(dlg).toBeHidden();
});

test("Options ▸ Online Update: the switch (default on) stops the automatic prompt", async ({ page }) => {
  await ready(page);
  await page.keyboard.press("Meta+,");
  const opts = page.getByTestId("options-dialog");
  await expect(opts).toBeVisible();
  await opts.getByTestId("tab-update").click();
  const box = opts.getByTestId("opt-check-updates");
  await expect(box).toBeChecked();
  await expect(opts.getByTestId("opt-update-version")).toHaveText("Installed version: Tally 0.1.0");
  const a11y = await new AxeBuilder({ page }).include('[data-testid="options-dialog"]').withTags(["wcag2a", "wcag2aa"]).analyze();
  expect(a11y.violations.filter((v) => v.impact === "serious" || v.impact === "critical").map((v) => v.id)).toEqual([]);
  await box.press("Space");
  await expect(box).not.toBeChecked();
  await opts.getByRole("button", { name: "OK" }).click();
  await expect(opts).toBeHidden();

  await ready(page, "/?update=0.2.0");
  await page.waitForTimeout(1200);
  await expect(page.getByTestId("update-prompt")).toBeHidden();
  // Asking still works.
  await menuCheck(page);
  await expect(page.getByTestId("update-prompt").getByRole("heading")).toHaveText("Tally 0.2.0 is available — you have 0.1.0");
});
