/**
 * Pinch and ⌃-wheel zoom over the grid (U18). A trackpad pinch arrives as
 * `wheel` events with `ctrlKey` and small deltas: the zoom follows the
 * fingers continuously. A mouse wheel notch with ⌃ (|deltaY| ≥ 50 px, or
 * a line/page delta) steps by 10 percentage points, as AOO's
 * `SC_DELTA_ZOOM` (sc/source/ui/view/tabview.cxx). Both stay within
 * 20–400 % (`MINZOOM` / `MAXZOOM`).
 */
import { useCallback, useRef } from "react";
import { clampZoom } from "../formats";
import { useUiStore } from "../stores/uiStore";

/** Wheel delta (px) from which a ⌃-wheel event counts as one mouse-wheel notch. */
export const WHEEL_NOTCH_PX = 50;

/** The zoom after a ⌃-wheel / pinch event (`deltaY` > 0 zooms out). */
export function wheelZoom(zoom: number, deltaY: number, deltaMode: number): number {
  if (deltaY === 0 || !Number.isFinite(deltaY)) return zoom;
  if (deltaMode !== 0 || Math.abs(deltaY) >= WHEEL_NOTCH_PX) return clampZoom(zoom + (deltaY < 0 ? 0.1 : -0.1));
  return clampZoom(zoom * Math.exp(-deltaY / 100));
}

/** A callback ref that zooms the view on pinch / ⌃-wheel over the element. */
export function usePinchZoom(): (el: HTMLElement | null) => void {
  const attached = useRef<{ el: HTMLElement; fn: (e: WheelEvent) => void } | null>(null);
  return useCallback((el: HTMLElement | null) => {
    if (attached.current) attached.current.el.removeEventListener("wheel", attached.current.fn);
    attached.current = null;
    if (!el) return;
    const fn = (e: WheelEvent) => {
      if (!e.ctrlKey) return;
      // Not the browser's page zoom.
      e.preventDefault();
      const ui = useUiStore.getState();
      const next = wheelZoom(ui.zoom, e.deltaY, e.deltaMode);
      if (next !== ui.zoom) ui.setZoom(next);
    };
    el.addEventListener("wheel", fn, { passive: false });
    attached.current = { el, fn };
  }, []);
}
