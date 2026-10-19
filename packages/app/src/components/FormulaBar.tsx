/**
 * Name box, "fx", the formula text and the plain-English hint. The text
 * field edits the SAME EditSession as the in-cell editor; when not focused
 * it shows the committed input prettified (display only).
 */
import { useEffect, useRef, useState } from "react";
import { useController } from "../context";
import { pretty } from "../pretty";
import { useEditStore } from "../stores/editStore";
import { useSelectionStore } from "../stores/selectionStore";
import { rangeAddress } from "../sumRange";
import styles from "./FormulaBar.module.css";

function NameBox() {
  const ctl = useController();
  const range = useSelectionStore((s) => s.selection.range);
  const address = rangeAddress(range);
  const [text, setText] = useState<string | null>(null);
  return (
    <input
      className={styles.nameBox}
      aria-label="Name box"
      data-testid="name-box"
      spellCheck={false}
      autoComplete="off"
      value={text ?? address}
      onFocus={(e) => {
        setText(address);
        e.currentTarget.select();
      }}
      onBlur={() => setText(null)}
      onChange={(e) => setText(e.target.value)}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          const t = (text ?? "").trim();
          if (t !== "") {
            void ctl.goTo(t).then((ok) => {
              if (ok) setText(null);
            });
          }
        } else if (e.key === "Escape") {
          e.preventDefault();
          setText(null);
          ctl.focusGrid();
        }
      }}
    />
  );
}

export function FormulaBar() {
  const ctl = useController();
  const edit = useEditStore((s) => s.edit);
  const active = useEditStore((s) => s.active);
  const origin = useEditStore((s) => s.origin);
  const cell = useSelectionStore((s) => s.selection.active);
  const [focused, setFocused] = useState(false);
  const input = useRef<HTMLInputElement>(null);

  const current = active && active.cell.r === cell.r && active.cell.c === cell.c ? active : null;
  const raw = current?.input ?? "";
  const value = edit ? edit.text : focused ? raw : pretty(raw);
  const hint = edit ? null : current?.hint;

  // Keep the caret where the user is typing when the grid editor changes the text.
  useEffect(() => {
    const el = input.current;
    if (!el || !edit || origin === "bar" || document.activeElement === el) return;
    el.scrollLeft = el.scrollWidth;
  }, [edit, origin]);

  const ensureEdit = () => {
    const st = useEditStore.getState();
    if (!st.edit) ctl.startEdit(cell, undefined, "bar");
    else if (st.origin !== "bar") st.start(st.edit.cell, st.edit.text, "bar");
  };

  return (
    <div className={styles.bar}>
      <NameBox />
      <span className={styles.fx} aria-hidden="true">
        fx
      </span>
      <input
        ref={input}
        className={styles.formula}
        aria-label="Formula"
        data-testid="formula-input"
        spellCheck={false}
        autoComplete="off"
        value={value}
        onFocus={() => {
          setFocused(true);
          ensureEdit();
        }}
        onBlur={() => setFocused(false)}
        onChange={(e) => {
          ensureEdit();
          ctl.onEditChange(e.target.value, e.target.selectionStart ?? e.target.value.length);
        }}
        onKeyDown={(e) => {
          if (e.nativeEvent.isComposing) return;
          if (e.key === "Enter") {
            e.preventDefault();
            void ctl.commitEdit(e.shiftKey ? "up" : "down");
          } else if (e.key === "Tab") {
            e.preventDefault();
            void ctl.commitEdit(e.shiftKey ? "left" : "right");
          } else if (e.key === "Escape") {
            e.preventDefault();
            ctl.cancelEdit();
          }
        }}
      />
      {hint ? (
        <span className={styles.hint} data-testid="formula-hint" title={hint}>
          {hint}
        </span>
      ) : null}
    </div>
  );
}
