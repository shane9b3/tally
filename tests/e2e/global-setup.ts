/**
 * Warms the dev server before the first test: loads the app once, opens the
 * lazy chunks (command palette, Format Cells) and waits until no more
 * requests arrive and Vite's dependency optimizer has stopped reloading the
 * page. On a cold Vite cache a late "optimized dependencies changed" reload
 * would otherwise land mid-test and fail it.
 */
import { chromium, type FullConfig } from "@playwright/test";

export default async function globalSetup(config: FullConfig): Promise<void> {
  const baseURL = String(config.projects[0]?.use.baseURL ?? "http://localhost:5173");
  const browser = await chromium.launch();
  try {
    const page = await browser.newPage();
    let navigations = 0;
    page.on("framenavigated", (f) => {
      if (f === page.mainFrame()) navigations++;
    });
    for (const url of ["/?template=invoices", "/"]) {
      await page.goto(baseURL + url);
      await page.getByTestId("grid").waitFor({ timeout: 60_000 });
      await page.waitForFunction(() => (window as unknown as { __tally?: { controller: { grid: unknown } } }).__tally?.controller.grid != null);
      for (const key of ["Meta+k", "Meta+1"]) {
        await page.keyboard.press(key);
        await page.waitForTimeout(500);
        await page.keyboard.press("Escape");
      }
      await page.waitForLoadState("networkidle");
    }
    // Stable: no further reload for three consecutive seconds.
    for (let stable = 0; stable < 3; ) {
      const before = navigations;
      await page.waitForTimeout(1000);
      await page.waitForLoadState("networkidle");
      stable = navigations === before ? stable + 1 : 0;
    }
  } finally {
    await browser.close();
  }
}
