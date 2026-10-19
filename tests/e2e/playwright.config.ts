/**
 * Browser e2e for the app chrome against MockClient (`pnpm e2e` at the root).
 * `@playwright/test` is installed in apps/desktop and mapped via tsconfig paths.
 */
import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: ".",
  testMatch: "*.spec.ts",
  fullyParallel: false,
  workers: 1,
  retries: process.env.CI ? 1 : 0,
  reporter: [["list"]],
  outputDir: "../../apps/desktop/test-results",
  use: {
    baseURL: "http://localhost:5173",
    viewport: { width: 1280, height: 820 },
    trace: "retain-on-failure",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"], viewport: { width: 1280, height: 820 } } }],
  webServer: {
    command: "pnpm --filter tally-desktop dev:web",
    url: "http://localhost:5173",
    reuseExistingServer: !process.env.CI,
    timeout: 60_000,
  },
});
