/**
 * Format bar borders popover: eight presets (all, outer, inner, top,
 * bottom, left, right, none), line weight (thin / medium / thick) and a
 * colour. Picking a preset applies it to the selection.
 */
import { useState } from "react";
import { BORDER_PRESETS, borderLine, DEFAULT_BORDER_COLOR, type BorderPreset, type LineWeight } from "../dataOps";
import { hexToRgba, rgbaToHex } from "../palette";
import { useDismiss, useFloating } from "./Popover";
import styles from "./BordersPicker.module.css";
import panel from "./Panel.module.css";

const COLORS = ["#1f2937", "#000000", "#6b7280", "#d1d5db", "#2563eb", "#16a34a", "#d97706", "#dc2626"];
const WEIGHTS: { id: LineWeight; label: string; px: number }[] = [
  { id: "thin", label: "Thin", px: 1 },
  { id: "medium", label: "Medium", px: 2 },
  { id: "thick", label: "Thick", px: 3 },
];

/** A 18 px preview of a preset: faint grid, the affected edges dark. */
function PresetGlyph({ preset, weight, color }: { preset: BorderPreset; weight: number; color: string }) {
  const on = (edge: "top" | "bottom" | "left" | "right" | "innerH" | "innerV"): boolean => {
    switch (preset) {
      case "all":
        return true;
      case "outer":
        return edge !== "innerH" && edge !== "innerV";
      case "inner":
        return edge === "innerH" || edge === "innerV";
      case "none":
        return false;
      default:
        return edge === preset;
    }
  };
  const faint = "var(--tally-border)";
  const w = Math.min(2.5, weight);
  const line = (key: string, x1: number, y1: number, x2: number, y2: number, lit: boolean) => (
    <line key={key} x1={x1} y1={y1} x2={x2} y2={y2} stroke={lit ? color : faint} strokeWidth={lit ? w : 1} strokeDasharray={lit ? undefined : "1.5 1.5"} />
  );
  return (
    <svg width={18} height={18} viewBox="0 0 18 18" aria-hidden="true">
      {line("ih", 2, 9, 16, 9, on("innerH"))}
      {line("iv", 9, 2, 9, 16, on("innerV"))}
      {line("t", 2, 2, 16, 2, on("top"))}
      {line("b", 2, 16, 16, 16, on("bottom"))}
      {line("l", 2, 2, 2, 16, on("left"))}
      {line("r", 16, 2, 16, 16, on("right"))}
    </svg>
  );
}

export interface BordersPickerProps {
  x: number;
  y: number;
  onClose: () => void;
  onPick: (preset: BorderPreset, line: ReturnType<typeof borderLine>) => void;
}

export function BordersPicker({ x, y, onClose, onPick }: BordersPickerProps) {
  const { ref, style } = useFloating(x, y);
  useDismiss(ref, onClose);
  const [weight, setWeight] = useState<LineWeight>("thin");
  const [color, setColor] = useState(rgbaToHex(DEFAULT_BORDER_COLOR));
  const px = WEIGHTS.find((w) => w.id === weight)?.px ?? 1;
  return (
    <div ref={ref} className={panel.card} style={{ ...style, width: 236 }} role="dialog" aria-label="Borders" data-testid="borders-picker">
      <div className={panel.title}>Borders</div>
      <div className={styles.presets}>
        {BORDER_PRESETS.map((b) => (
          <button
            key={b.id}
            type="button"
            className={styles.preset}
            title={b.label}
            aria-label={b.label}
            data-preset={b.id}
            onClick={() => {
              onClose();
              onPick(b.id, borderLine(weight, hexToRgba(color) ?? DEFAULT_BORDER_COLOR));
            }}
          >
            <PresetGlyph preset={b.id} weight={px} color={color} />
          </button>
        ))}
      </div>
      <div className={styles.label}>Line</div>
      <div className={styles.weights} role="radiogroup" aria-label="Line weight">
        {WEIGHTS.map((w) => (
          <button key={w.id} type="button" role="radio" aria-checked={weight === w.id} className={weight === w.id ? styles.weightOn : styles.weight} onClick={() => setWeight(w.id)} title={w.label} data-weight={w.id}>
            <span className={styles.sample} style={{ height: w.px, background: color }} />
          </button>
        ))}
      </div>
      <div className={styles.label}>Colour</div>
      <div className={styles.colors}>
        {COLORS.map((c) => (
          <button key={c} type="button" className={styles.swatch} style={{ background: c }} aria-label={c} aria-pressed={color === c} onClick={() => setColor(c)} data-color={c} />
        ))}
      </div>
    </div>
  );
}
