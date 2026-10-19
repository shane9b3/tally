/** A small swatch palette (24 colours + custom hex) in a floating card. */
import { useState } from "react";
import { hexToRgba, rgbaToHex, SWATCHES, TRANSPARENT } from "../palette";
import { useDismiss, useFloating } from "./Popover";
import styles from "./ColorPicker.module.css";

export interface ColorPickerProps {
  x: number;
  y: number;
  label: string;
  /** Current colour (`0xRRGGBBAA`), highlighted. */
  value: number | null;
  /** Offer "No fill" (a transparent fill). */
  allowNone?: boolean;
  onPick: (rgba: number) => void;
  onClose: () => void;
}

export function ColorPicker({ x, y, label, value, allowNone, onPick, onClose }: ColorPickerProps) {
  const { ref, style } = useFloating(x, y);
  useDismiss(ref, onClose);
  const [hex, setHex] = useState(value !== null && (value & 255) > 0 ? rgbaToHex(value) : "#");
  const parsed = hexToRgba(hex);
  const current = value !== null ? rgbaToHex(value) : null;
  const pick = (rgba: number) => {
    onClose();
    onPick(rgba);
  };
  return (
    <div ref={ref} className={styles.card} style={style} role="dialog" aria-label={label} data-testid="color-picker">
      <div className={styles.title}>{label}</div>
      {allowNone ? (
        <button type="button" className={styles.none} onClick={() => pick(TRANSPARENT)}>
          <span className={styles.noneSwatch} /> No fill
        </button>
      ) : null}
      <div className={styles.grid}>
        {SWATCHES.map((c) => (
          <button
            key={c}
            type="button"
            className={styles.swatch}
            style={{ background: c }}
            title={c}
            aria-label={c}
            aria-pressed={current === c}
            data-color={c}
            onClick={() => pick(hexToRgba(c) ?? 0)}
          />
        ))}
      </div>
      <form
        className={styles.custom}
        onSubmit={(e) => {
          e.preventDefault();
          if (parsed !== null) pick(parsed);
        }}
      >
        <span className={styles.preview} style={{ background: parsed !== null ? hex : "transparent" }} />
        <input
          className={styles.hex}
          aria-label="Custom colour (hex)"
          value={hex}
          spellCheck={false}
          maxLength={7}
          onChange={(e) => setHex(e.target.value.startsWith("#") ? e.target.value : `#${e.target.value}`)}
        />
        <button type="submit" className={styles.apply} disabled={parsed === null}>
          Apply
        </button>
      </form>
    </div>
  );
}
