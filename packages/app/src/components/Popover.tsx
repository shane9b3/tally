/** A small floating menu at fixed client coordinates. */
import { useEffect, useRef, type CSSProperties } from "react";
import { cx } from "../context";
import styles from "./Popover.module.css";

export interface PopoverItem {
  id: string;
  label: string;
  hint?: string;
  checked?: boolean;
  danger?: boolean;
  disabled?: boolean;
  onSelect: () => void;
}

export interface PopoverProps {
  x: number;
  y: number;
  /** Anchor the bottom edge at `y` (menus that open upwards). */
  above?: boolean;
  /** Anchor the right edge at `x`. */
  alignRight?: boolean;
  items: PopoverItem[];
  onClose: () => void;
  label: string;
}

export function Popover({ x, y, above, alignRight, items, onClose, label }: PopoverProps) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const down = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("mousedown", down, true);
    window.addEventListener("keydown", key, true);
    ref.current?.querySelector<HTMLButtonElement>("button:not([disabled])")?.focus();
    return () => {
      window.removeEventListener("mousedown", down, true);
      window.removeEventListener("keydown", key, true);
    };
  }, [onClose]);
  const style: CSSProperties = {
    ...(alignRight ? { right: window.innerWidth - x } : { left: x }),
    ...(above ? { bottom: window.innerHeight - y } : { top: y }),
  };
  return (
    <div ref={ref} className={styles.menu} style={style} role="menu" aria-label={label}>
      {items.map((it) => (
        <button
          key={it.id}
          type="button"
          role="menuitem"
          className={cx(styles.item, it.danger && styles.danger, it.checked && styles.checked)}
          disabled={it.disabled}
          onClick={() => {
            onClose();
            it.onSelect();
          }}
        >
          <span className={styles.check}>{it.checked ? "✓" : ""}</span>
          <span className={styles.label}>{it.label}</span>
          {it.hint ? <span className={styles.hint}>{it.hint}</span> : null}
        </button>
      ))}
    </div>
  );
}
