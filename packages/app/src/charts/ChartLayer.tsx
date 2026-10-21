/**
 * Mounts the live ECharts instances and pictures into the grid's object
 * layer (W5-5). Renders nothing itself: an effect runs `ChartMounts.sync`
 * whenever the objects, their data, the theme, the zoom or the viewport
 * change (the grid has already placed the layer nodes by then: the `Grid`
 * component's update effect runs before this sibling's).
 */
import { useEffect, useMemo, useRef } from "react";
import { useController } from "../context";
import { useDocStore } from "../stores/docStore";
import { useW5Store } from "../objects/w5Store";
import { useUiStore } from "../stores/uiStore";
import { ChartMounts } from "./chartMounts";
import { useChartStore } from "./chartStore";
import { LIGHT_CHART_THEME, type ChartTheme } from "./toEcharts";

/** The chart theme for the resolved UI theme (CSS token colours). */
export function chartThemeFor(resolved: "light" | "dark"): ChartTheme {
  if (typeof getComputedStyle !== "function" || typeof document === "undefined") return { ...LIGHT_CHART_THEME, dark: resolved === "dark" };
  const css = getComputedStyle(document.documentElement);
  const read = (name: string, fallback: string): string => css.getPropertyValue(name).trim() || fallback;
  return {
    dark: resolved === "dark",
    text: read("--tally-text", LIGHT_CHART_THEME.text),
    textMuted: read("--tally-text-muted", LIGHT_CHART_THEME.textMuted),
    gridLine: read("--tally-grid-line", LIGHT_CHART_THEME.gridLine),
    bg: read("--tally-bg", LIGHT_CHART_THEME.bg),
    fontFamily: read("--tally-font-family", LIGHT_CHART_THEME.fontFamily),
  };
}

export function ChartLayer() {
  const ctl = useController();
  const charts = useChartStore((s) => s.charts);
  const images = useChartStore((s) => s.images);
  const data = useChartStore((s) => s.data);
  const imageData = useChartStore((s) => s.imageData);
  const tick = useChartStore((s) => s.viewportTick);
  const resolved = useUiStore((s) => s.theme);
  const zoom = useUiStore((s) => s.zoom);
  const sheet = useDocStore((s) => s.activeSheet);
  const theme = useMemo(() => chartThemeFor(resolved), [resolved]);
  const split = useW5Store((s) => s.split);
  const mounts = useRef<ChartMounts | null>(null);
  /** The second pane of a split window gets its own instances. */
  const secondary = useRef<ChartMounts | null>(null);

  useEffect(() => {
    const m = new ChartMounts();
    const m2 = new ChartMounts(false);
    mounts.current = m;
    secondary.current = m2;
    ctl.w5.charts.mounts = m;
    return () => {
      m.dispose();
      m2.dispose();
      if (ctl.w5.charts.mounts === m) ctl.w5.charts.mounts = null;
      mounts.current = null;
      secondary.current = null;
    };
  }, [ctl]);

  useEffect(() => {
    const m = mounts.current;
    const m2 = secondary.current;
    if (!m || !m2) return;
    const run = (): void => {
      if (ctl.grid) m.sync(ctl.grid, charts, data, images, imageData, theme);
      ctl.w5.objects.fetchStaleVisible();
      const g2 = split ? ctl.w5.secondaryGrid : null;
      if (g2) m2.sync(g2, charts, data, images, imageData, theme);
      else m2.clear();
    };
    // The layer nodes appear after the grid's own update effect; sync now and once more next frame.
    run();
    const raf = requestAnimationFrame(run);
    return () => cancelAnimationFrame(raf);
  }, [ctl, charts, data, images, imageData, theme, zoom, tick, sheet, split]);

  return null;
}
