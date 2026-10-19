/**
 * Scroll-perf harness for the WP6 acceptance check ("a scroll fling across
 * 1M rows keeps p95 frame time under 16 ms on an M-series Mac").
 *
 * How to run today (manual):
 *   pnpm --filter @tally/grid dev        # opens http://localhost:5174
 *   - fling the trackpad, or press "Fling test" in the toolbar; the demo
 *     prints p50/p95/max frame times measured inside the grid.
 *   - `?autorun=1` in the URL runs the fling on load and writes the result
 *     to `window.__tallyPerf` (what a Playwright spec would read).
 *
 * Future e2e (e2e/scroll-perf.spec.ts, once @playwright/test is added to
 * this package): open the dev server with `?autorun=1`, wait for
 * `window.__tallyPerf`, assert `p95 < 16` in Chromium and WebKit.
 *
 * Frame times come from `Grid.getStats().frameTimes`: the interval between
 * consecutive animation frames that redrew the main canvas.
 */
import type { Grid } from "../src/grid";

export interface PerfResult {
  frames: number;
  p50: number;
  p95: number;
  max: number;
  /** Rows travelled. */
  rows: number;
  durationMs: number;
  /** Average main-canvas draw time (ms), excluding browser compositing. */
  drawAvg: number;
}

export function percentile(values: readonly number[], p: number): number {
  if (values.length === 0) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  const i = Math.min(sorted.length - 1, Math.max(0, Math.ceil((p / 100) * sorted.length) - 1));
  return sorted[i] ?? 0;
}

/**
 * Scrolls `grid` from the top to `rows` with an ease-out curve (a fling)
 * over `durationMs`, one step per animation frame.
 */
export function runFling(grid: Grid, rows = 1_048_000, durationMs = 4000): Promise<PerfResult> {
  return new Promise((resolve) => {
    grid.scrollTo({ r: 0, c: 0 });
    grid.resetStats();
    const draws: number[] = [];
    let start = 0;
    const step = (t: number): void => {
      if (start === 0) start = t;
      const k = Math.min(1, (t - start) / durationMs);
      const eased = 1 - (1 - k) ** 3;
      grid.scrollTo({ r: Math.round(eased * rows), c: 0 });
      draws.push(grid.getStats().last.ms);
      if (k < 1) {
        requestAnimationFrame(step);
        return;
      }
      requestAnimationFrame(() => {
        const ft = grid.getStats().frameTimes;
        resolve({
          frames: ft.length,
          p50: percentile(ft, 50),
          p95: percentile(ft, 95),
          max: ft.length ? Math.max(...ft) : 0,
          rows,
          durationMs,
          drawAvg: draws.reduce((a, b) => a + b, 0) / Math.max(1, draws.length),
        });
      });
    };
    requestAnimationFrame(step);
  });
}
