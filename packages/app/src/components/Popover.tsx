/** A small floating menu at fixed client coordinates (kept inside the viewport). */
import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { cx } from "../context";
import styles from "./Popover.module.css";
import { arrowNav, useReturnFocus } from "./dialogA11y";

export interface PopoverItem {
  id: string;
  label: string;
  hint?: string;
  checked?: boolean;
  danger?: boolean;
  disabled?: boolean;
  /** Tooltip (e.g. why the item is disabled). */
  title?: string;
  /** Draw a separator above this item. */
  separator?: boolean;
  icon?: ReactNode;
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
  testId?: string;
}

/** Closes on outside mousedown and Escape. */
export function useDismiss(ref: React.RefObject<HTMLElement | null>, onClose: () => void): void {
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
    return () => {
      window.removeEventListener("mousedown", down, true);
      window.removeEventListener("keydown", key, true);
    };
  }, [ref, onClose]);
}

/** Position style for a floating element anchored at (x, y), nudged inside the viewport after mount. */
export function useFloating(x: number, y: number, above = false, alignRight = false) {
  const ref = useRef<HTMLDivElement>(null);
  const [shift, setShift] = useState({ dx: 0, dy: 0 });
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const m = 8;
    let dx = 0;
    let dy = 0;
    if (r.right > window.innerWidth - m) dx = window.innerWidth - m - r.right;
    if (r.left + dx < m) dx = m - r.left;
    if (r.bottom > window.innerHeight - m) dy = window.innerHeight - m - r.bottom;
    if (r.top + dy < m) dy = m - r.top;
    if (dx !== 0 || dy !== 0) setShift({ dx, dy });
    // Measured once per anchor.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [x, y]);
  const style: CSSProperties = {
    ...(alignRight ? { right: window.innerWidth - x - shift.dx } : { left: x + shift.dx }),
    ...(above ? { bottom: window.innerHeight - y - shift.dy } : { top: y + shift.dy }),
  };
  return { ref, style };
}

export function Popover({ x, y, above, alignRight, items, onClose, label, testId }: PopoverProps) {
  const { ref, style } = useFloating(x, y, above, alignRight);
  useDismiss(ref, onClose);
  const restoreFocus = useReturnFocus({ overlay: true });
  useEffect(() => {
    ref.current?.querySelector<HTMLButtonElement>("button:not([disabled])")?.focus();
  }, [ref]);
  return (
    <div ref={ref} className={styles.menu} style={style} role="menu" aria-label={label} data-testid={testId} onKeyDown={(e) => void arrowNav(e)}>
      {items.map((it) => (
        <div key={it.id} className={styles.row} role="none">
          {it.separator ? <div className={styles.separator} role="separator" /> : null}
          <button
            type="button"
            role={it.checked === undefined ? "menuitem" : "menuitemcheckbox"}
            aria-checked={it.checked}
            className={cx(styles.item, it.danger && styles.danger, it.checked && styles.checked)}
            aria-disabled={it.disabled || undefined}
            title={it.title}
            data-item={it.id}
            onClick={() => {
              if (it.disabled) return;
              restoreFocus();
              onClose();
              it.onSelect();
            }}
          >
            <span className={styles.check}>{it.checked ? "✓" : (it.icon ?? "")}</span>
            <span className={styles.label}>{it.label}</span>
            {it.hint ? <span className={styles.hint}>{it.hint}</span> : null}
          </button>
        </div>
      ))}
    </div>
  );
}
