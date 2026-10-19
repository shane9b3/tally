import type { StyleDto } from "@tally/bridge";
import { useEffect, useState } from "react";
import { useDocStore } from "./stores/docStore";
import { useSelectionStore } from "./stores/selectionStore";

/** The resolved style of the active cell (re-reads when the grid's data arrives). */
export function useActiveStyle(): StyleDto | undefined {
  const source = useDocStore((s) => s.source);
  const rev = useDocStore((s) => s.rev);
  const active = useSelectionStore((s) => s.selection.active);
  const [, setTick] = useState(0);
  useEffect(() => {
    if (!source) return undefined;
    let raf = 0;
    const off = source.subscribe(() => {
      if (raf) return;
      raf = requestAnimationFrame(() => {
        raf = 0;
        setTick((t) => t + 1);
      });
    });
    return () => {
      off();
      if (raf) cancelAnimationFrame(raf);
    };
  }, [source]);
  void rev;
  if (!source) return undefined;
  const cell = source.getCell(active.r, active.c);
  return source.getStyle(cell?.style ?? 0);
}

/** "just now", "5 min ago", "2 h ago", "3 days ago". */
export function relativeTime(then: number, now: number = Date.now()): string {
  const s = Math.max(0, Math.round((now - then) / 1000));
  if (s < 45) return "just now";
  const m = Math.round(s / 60);
  if (m < 60) return `${m} min ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h} h ago`;
  const d = Math.round(h / 24);
  if (d === 1) return "yesterday";
  if (d < 30) return `${d} days ago`;
  return new Date(then).toLocaleDateString();
}
