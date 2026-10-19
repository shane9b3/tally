/**
 * Name box, "fx", the formula text and the plain-English hint. The text
 * field edits the SAME EditSession as the in-cell editor; when not focused
 * it shows the committed input prettified (display only).
 *
 * While a formula is being edited its references are coloured like the
 * grid outlines: a mirror div behind the input draws the coloured text and
 * the input's own text turns transparent (caret and selection stay native).
 */
import { Fragment, type ReactNode, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useController } from "../context";
import { formulaHighlights } from "../formulaEdit";
import { pretty } from "../pretty";
import { useDocStore } from "../stores/docStore";
import { useEditStore } from "../stores/editStore";
import { useUiStore } from "../stores/uiStore";
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
  const mirror = useRef<HTMLDivElement>(null);
  const theme = useUiStore((s) => s.theme);
  const sheetName = useDocStore((s) => s.info?.sheets.find((x) => x.id === s.activeSheet)?.name ?? null);

  const current = active && active.cell.r === cell.r && active.cell.c === cell.c ? active : null;
  const raw = current?.input ?? "";
  const value = edit ? edit.text : focused ? raw : pretty(raw);
  const hint = edit ? null : current?.hint;
  const spans = useMemo(() => (edit ? formulaHighlights(edit.text, sheetName, theme).spans : []), [edit, sheetName, theme]);
  const coloured = spans.length > 0;

  const syncScroll = () => {
    if (mirror.current && input.current) mirror.current.scrollLeft = input.current.scrollLeft;
  };

  // Programmatic changes (point mode, autocomplete, editor undo) move the caret here too.
  const seq = edit?.seq ?? 0;
  useLayoutEffect(() => {
    const el = input.current;
    const e = useEditStore.getState().edit;
    if (!el || !e || document.activeElement !== el) return;
    el.setSelectionRange(e.caret, e.caret);
    syncScroll();
  }, [seq]);

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
      <div className={styles.field}>
        {coloured ? (
          <div ref={mirror} className={styles.mirror} aria-hidden="true" data-testid="formula-mirror">
            {renderSpans(value, spans)}
          </div>
        ) : null}
        <input
        ref={input}
        className={styles.formula}
        style={coloured ? { color: "transparent", background: "transparent" } : undefined}
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
        onScroll={syncScroll}
        onSelect={(e) => {
          syncScroll();
          const st = useEditStore.getState();
          const el = e.currentTarget;
          if (st.edit && st.origin === "bar" && el.value === st.edit.text && el.selectionStart !== null && el.selectionStart !== st.edit.caret) {
            ctl.onEditChange(el.value, el.selectionStart);
          }
        }}
        onChange={(e) => {
          ensureEdit();
          ctl.onEditChange(e.target.value, e.target.selectionStart ?? e.target.value.length);
        }}
        onKeyDown={(e) => {
          if (e.nativeEvent.isComposing) return;
          if (useEditStore.getState().edit && ctl.handleEditorKey(e)) {
            e.preventDefault();
            e.stopPropagation();
            return;
          }
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
      </div>
      {hint ? (
        <span className={styles.hint} data-testid="formula-hint" title={hint}>
          {hint}
        </span>
      ) : null}
    </div>
  );
}

/** The text with coloured reference spans (for the mirror behind the input). */
function renderSpans(text: string, spans: readonly { start: number; end: number; color: string }[]) {
  const out: ReactNode[] = [];
  let at = 0;
  spans.forEach((h, i) => {
    const s0 = Math.max(at, Math.min(text.length, h.start));
    const s1 = Math.max(s0, Math.min(text.length, h.end));
    if (s0 > at) out.push(<Fragment key={`t${i}`}>{text.slice(at, s0)}</Fragment>);
    if (s1 > s0)
      out.push(
        <span key={`r${i}`} style={{ color: h.color }}>
          {text.slice(s0, s1)}
        </span>,
      );
    at = Math.max(at, s1);
  });
  if (at < text.length) out.push(<Fragment key="tail">{text.slice(at)}</Fragment>);
  return out;
}
