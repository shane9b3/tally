/**
 * Browser e2e for the app chrome against MockClient (`pnpm e2e` at the root).
 * `@playwright/test` is installed in apps/desktop and mapped via tsconfig paths.
 */
import { defineConfig, devices } from "@playwright/test";

// Port is overridable (`TALLY_E2E_PORT=5201 pnpm e2e`) so concurrent runs do not collide.
const PORT = Number(process.env.TALLY_E2E_PORT ?? 5173);
const ORIGIN = `http://localhost:${PORT}`;

export default defineConfig({
  testDir: ".",
  testMatch: "*.spec.ts",
  globalSetup: "./global-setup.ts",
  fullyParallel: false,
  workers: 1,
  retries: process.env.CI ? 1 : 0,
  reporter: [["list"]],
  outputDir: "../../apps/desktop/test-results",
  use: {
    baseURL: ORIGIN,
    viewport: { width: 1280, height: 820 },
    trace: "retain-on-failure",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"], viewport: { width: 1280, height: 820 } } }],
  webServer: {
    command: "pnpm --filter tally-desktop dev:web",
    env: { TALLY_E2E_PORT: String(PORT) },
    url: ORIGIN,
    reuseExistingServer: !process.env.CI,
    timeout: 60_000,
  },
});
