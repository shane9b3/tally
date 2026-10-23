/**
 * Grid scroll frame-time benchmark (`pnpm perf:e2e`, see docs/perf-baseline.md).
 * Kept out of `pnpm e2e`: the default config only matches `*.spec.ts`, this
 * one only matches `*.perf.ts`. Runs against the web build with MockClient.
 */
import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: ".",
  testMatch: "*.perf.ts",
  fullyParallel: false,
  workers: 1,
  timeout: 300_000,
  reporter: [["list"]],
  outputDir: "../../apps/desktop/test-results-perf",
  use: { baseURL: "http://localhost:5173", viewport: { width: 1280, height: 820 } },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"], viewport: { width: 1280, height: 820 } } }],
  webServer: {
    command: "pnpm --filter tally-desktop dev:web",
    url: "http://localhost:5173",
    reuseExistingServer: !process.env.CI,
    timeout: 60_000,
  },
});
