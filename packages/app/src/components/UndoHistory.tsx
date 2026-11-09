/**
 * The toolbar Undo / Redo drop-downs (AOO's Undo and Redo toolbox list
 * boxes, `SvxUndoRedoControl`): a split button whose arrow opens a listbox
 * of the steps, next one first. Hovering or arrowing to entry n marks
 * entries 1..n and the footer counts them; clicking it, Enter or Space
 * undoes (redoes) n steps as one user action. Escape, Tab or a click
 * outside closes the list.
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { cx, useController } from "../context";
import { useDocStore } from "../stores/docStore";
import { useEditStore } from "../stores/editStore";
import { undoListFooter, undoListKey, undoListLabel, undoListTitle, type UndoListKind } from "../undoList";
import { t } from "../i18n";
import { useDismiss, useFloating } from "./Popover";
import { useReturnFocus, useTitleId } from "./dialogA11y";
import styles from "./UndoHistory.module.css";

interface ListProps {
  kind: UndoListKind;
  labels: readonly string[];
  x: number;
  y: number;
  onPick(count: number): void;
  onClose(): void;
}

/** The open list. */
export function UndoHistoryList({ kind, labels, x, y, onPick, onClose }: ListProps) {
  const { ref, style } = useFloating(x, y);
  useDismiss(ref, onClose);
  useReturnFocus();
  const listRef = useRef<HTMLDivElement>(null);
  const [count, setCount] = useState(1);
  const id = useTitleId();
  const footerId = `${id}-count`;
  useEffect(() => listRef.current?.focus(), []);
  useEffect(() => {
    listRef.current?.querySelector(`#${CSS.escape(`${id}-${count - 1}`)}`)?.scrollIntoView?.({ block: "nearest" });
  }, [count, id]);
  return (
    <div ref={ref} className={styles.popup} style={style} data-testid={`${kind}-history`}>
      <div
        ref={listRef}
        className={styles.list}
        role="listbox"
        tabIndex={0}
        aria-label={undoListTitle(kind)}
        aria-multiselectable="true"
        aria-activedescendant={labels.length > 0 ? `${id}-${count - 1}` : undefined}
        aria-describedby={footerId}
        onKeyDown={(e) => {
          const r = undoListKey(e.key, count, labels.length);
          if (r === null) return;
          e.preventDefault();
          e.stopPropagation();
          if (r === "pick") onPick(count);
          else if (r === "close") onClose();
          else setCount(r.count);
        }}
      >
        {labels.map((label, i) => (
          <div
            key={i}
            id={`${id}-${i}`}
            role="option"
            aria-selected={i < count}
            className={cx(styles.option, i < count && styles.marked)}
            data-index={i}
            onMouseEnter={() => setCount(i + 1)}
            onMouseDown={(e) => e.preventDefault() /* keep the focus in the list */}
            onClick={() => onPick(i + 1)}
          >
            {undoListLabel(label)}
          </div>
        ))}
      </div>
      <div id={footerId} className={styles.footer} aria-live="polite" data-testid={`${kind}-history-count`}>
        {undoListFooter(kind, count)}
      </div>
    </div>
  );
}

/** The arrow half of the Undo or Redo split button, with its list. */
export function UndoHistoryArrow({ kind, className }: { kind: UndoListKind; className?: string | undefined }) {
  const ctl = useController();
  const can = useDocStore((s) => (kind === "undo" ? s.undo.canUndo : s.undo.canRedo));
  const readOnly = useDocStore((s) => s.info?.readOnly ?? false);
  const editing = useEditStore((s) => s.edit !== null);
  const [open, setOpen] = useState<{ x: number; y: number; labels: string[] } | null>(null);
  const seq = useRef(0);
  // The outside-click handler closes the list on mousedown; the click that follows must not reopen it.
  const wasOpen = useRef(false);
  const enabled = can && !editing && !readOnly;
  const close = useCallback(() => setOpen(null), []);
  const title = undoListTitle(kind);
  return (
    <>
      <button
        type="button"
        className={cx(styles.arrow, className)}
        aria-label={title}
        title={title}
        aria-haspopup="listbox"
        aria-expanded={open !== null}
        aria-disabled={!enabled}
        data-testid={`${kind}-history-arrow`}
        onMouseDown={(e) => {
          e.preventDefault(); // keep focus in the grid
          wasOpen.current = open !== null;
        }}
        onClick={(e) => {
          if (open || wasOpen.current) {
            wasOpen.current = false;
            setOpen(null);
            return;
          }
          if (!enabled) return;
          const r = e.currentTarget.getBoundingClientRect();
          const n = ++seq.current;
          void ctl.undoHistory().then(
            (h) => {
              const labels = kind === "undo" ? h.undo : h.redo;
              if (n === seq.current && labels.length > 0) setOpen({ x: r.left, y: r.bottom + 6, labels });
            },
            (err: unknown) => ctl.notifyError(err),
          );
        }}
      >
        <svg width="8" height="8" viewBox="0 0 8 8" aria-hidden="true">
          <path d="M1 2.5 4 5.5 7 2.5" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      </button>
      {open ? (
        <UndoHistoryList
          kind={kind}
          labels={open.labels}
          x={open.x}
          y={open.y}
          onClose={close}
          onPick={(count) => {
            setOpen(null);
            void ctl.undoRedoSteps(kind === "undo", count).then(() => ctl.focusGrid());
          }}
        />
      ) : null}
    </>
  );
}

/** Tooltip of the Undo / Redo button: "Undo: Typing" (AOO's `STR_UNDO` + the comment). */
export function undoTooltip(kind: UndoListKind, label: string | null): string | undefined {
  if (!label) return undefined;
  return kind === "undo" ? t("undoList.undoWhat", "Undo: {what}", { what: label }) : t("undoList.redoWhat", "Redo: {what}", { what: label });
}
