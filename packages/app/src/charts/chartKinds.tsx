/** The chart type gallery's thumbnails and labels (W5-5). */
import type { ChartKind } from "@tally/bridge";

export const CHART_KINDS: readonly { kind: ChartKind; label: string }[] = [
  { kind: "column", label: "Column" },
  { kind: "bar", label: "Bar" },
  { kind: "line", label: "Line" },
  { kind: "area", label: "Area" },
  { kind: "pie", label: "Pie" },
  { kind: "scatter", label: "XY" },
  { kind: "bubble", label: "Bubble" },
  { kind: "radar", label: "Net" },
  { kind: "stock", label: "Stock" },
];

/** A 28×20 stroke/fill thumbnail of a chart kind. */
export function KindThumb({ kind }: { kind: ChartKind }) {
  const common = { width: 28, height: 20, viewBox: "0 0 28 20", "aria-hidden": true as const };
  switch (kind) {
    case "column":
      return (
        <svg {...common}>
          <rect x="3" y="9" width="5" height="10" fill="currentColor" />
          <rect x="11" y="3" width="5" height="16" fill="currentColor" opacity="0.7" />
          <rect x="19" y="12" width="5" height="7" fill="currentColor" opacity="0.45" />
        </svg>
      );
    case "bar":
      return (
        <svg {...common}>
          <rect x="2" y="2" width="18" height="4" fill="currentColor" />
          <rect x="2" y="8" width="24" height="4" fill="currentColor" opacity="0.7" />
          <rect x="2" y="14" width="10" height="4" fill="currentColor" opacity="0.45" />
        </svg>
      );
    case "line":
      return (
        <svg {...common} fill="none" stroke="currentColor" strokeWidth="2" strokeLinejoin="round">
          <path d="M2 16 9 8 15 12 26 3" />
        </svg>
      );
    case "area":
      return (
        <svg {...common}>
          <path d="M2 18 2 14 9 7 15 11 26 3 26 18Z" fill="currentColor" opacity="0.6" />
        </svg>
      );
    case "pie":
      return (
        <svg {...common}>
          <circle cx="14" cy="10" r="8" fill="currentColor" opacity="0.4" />
          <path d="M14 10 L14 2 A8 8 0 0 1 21.5 13 Z" fill="currentColor" />
        </svg>
      );
    case "scatter":
      return (
        <svg {...common} fill="currentColor">
          <circle cx="5" cy="15" r="2" />
          <circle cx="10" cy="9" r="2" />
          <circle cx="15" cy="12" r="2" />
          <circle cx="20" cy="5" r="2" />
          <circle cx="24" cy="8" r="2" />
        </svg>
      );
    case "bubble":
      return (
        <svg {...common} fill="currentColor">
          <circle cx="7" cy="13" r="4" opacity="0.6" />
          <circle cx="16" cy="8" r="6" opacity="0.8" />
          <circle cx="24" cy="14" r="2.5" />
        </svg>
      );
    case "radar":
      return (
        <svg {...common} fill="none" stroke="currentColor" strokeWidth="1.5">
          <path d="M14 2 25 9 21 19 7 19 3 9Z" opacity="0.5" />
          <path d="M14 6 20 10 18 15 10 15 8 10Z" fill="currentColor" opacity="0.5" />
        </svg>
      );
    case "stock":
      return (
        <svg {...common} stroke="currentColor" strokeWidth="1.5">
          <path d="M6 3v14M6 6h4v7H6M14 2v16M14 5h4v9h-4M22 4v14M22 8h4v6h-4" fill="currentColor" />
        </svg>
      );
  }
}
