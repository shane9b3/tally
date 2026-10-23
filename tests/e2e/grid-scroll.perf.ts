/**
 * Frame times while scrolling a large sheet (1M cells loaded through the dev
 * mock). Not part of `pnpm e2e`; run with `pnpm --filter tally-desktop exec
 * playwright test -c ../../tests/e2e/perf.config.ts`. Prints one JSON line
 * per run; asserts only a very loose ceiling so it never flakes in CI.
 */
import { expect, test } from "@playwright/test";

const ROWS = 50_000;
const COLS = 20;

test("grid scroll frame times on a 1M-cell sheet", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("grid")).toBeVisible();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);

  const loadMs = await page.evaluate(
    async ({ rows, cols }) => {
      const ctl = window.__tally!.controller;
      const t0 = performance.now();
      const chunk = 2500;
      for (let r0 = 0; r0 < rows; r0 += chunk) {
        const block: string[][] = [];
        for (let r = r0; r < Math.min(rows, r0 + chunk); r++) {
          const line: string[] = [];
          for (let c = 0; c < cols; c++) line.push(c % 5 === 4 ? `item ${r}-${c}` : String((r * 31 + c * 7) % 100000));
          block.push(line);
        }
        await ctl.apply("load", [{ type: "setRangeInputs", sheet: ctl.sheet, r: r0, c: 0, rows: block }]);
      }
      return performance.now() - t0;
    },
    { rows: ROWS, cols: COLS },
  );

  // Scroll the grid's native scroller one step per animation frame and record
  // the gap between frames. Vertical, then horizontal, then diagonal.
  const result = await page.evaluate(async () => {
    const root = document.querySelector('[data-testid="grid"]') as HTMLElement;
    const scroller = Array.from(root.querySelectorAll<HTMLElement>("div")).find((d) => {
      const o = getComputedStyle(d).overflow;
      return /auto|scroll/.test(o) && d.scrollHeight > d.clientHeight;
    });
    if (!scroller) throw new Error("scroller not found");
    const raf = () => new Promise<number>((res) => requestAnimationFrame(res));
    const run = async (frames: number, dx: number, dy: number): Promise<number[]> => {
      const gaps: number[] = [];
      let prev = await raf();
      for (let i = 0; i < frames; i++) {
        scroller.scrollTop += dy;
        scroller.scrollLeft += dx;
        const now = await raf();
        gaps.push(now - prev);
        prev = now;
      }
      return gaps;
    };
    const stats = (g: number[]) => {
      const s = [...g].sort((a, b) => a - b);
      const q = (p: number) => s[Math.min(s.length - 1, Math.floor(p * s.length))]!;
      return {
        frames: g.length,
        mean: g.reduce((a, b) => a + b, 0) / g.length,
        p50: q(0.5),
        p95: q(0.95),
        p99: q(0.99),
        max: s[s.length - 1]!,
        over20ms: g.filter((x) => x > 20).length,
      };
    };
    scroller.scrollTop = 0;
    scroller.scrollLeft = 0;
    await run(10, 0, 0);
    const vertical = stats(await run(300, 0, 240));
    scroller.scrollTop = 0;
    const horizontal = stats(await run(150, 160, 0));
    scroller.scrollLeft = 0;
    const diagonal = stats(await run(300, 120, 240));
    return { vertical, horizontal, diagonal };
  });

  console.log("PERF " + JSON.stringify({ cells: ROWS * COLS, loadMs: Math.round(loadMs), ...result }));
  expect(result.vertical.p95).toBeLessThan(100);
});
