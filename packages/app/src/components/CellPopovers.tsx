/**
 * Popovers anchored at a cell: the note shown on hover, the note editor
 * (⇧F2), the validation list (the cell's ▾) and the validation input help.
 */
import type { CellPos } from "@tally/bridge";
import { useEffect, useRef, useState } from "react";
import { cx, useController } from "../context";
import { useW56Store } from "../data/store";
import { t } from "../i18n";
import { rgbaToHex } from "../palette";
import { useV3Store, type CellPopover } from "../stores/v3Store";
import { useDismiss, useFloating } from "./Popover";
import styles from "./CellPopovers.module.css";

export function CellPopovers() {
  const p = useV3Store((s) => s.popover);
  useW56Store((s) => s.i18nRev);
  if (!p) return null;
  switch (p.kind) {
    case "note":
      return <NoteView p={p} />;
    case "noteEdit":
      return <NoteEditor key={`${p.cell.r},${p.cell.c}`} p={p} />;
    case "list":
      return <ListPopover p={p} />;
    case "inputHelp":
      return <InputHelp p={p} />;
  }
}

const closePopover = () => useV3Store.getState().setPopover(null);

function when(iso: string | null): string {
  if (!iso) return "";
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? "" : d.toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" });
}

function NoteView({ p }: { p: Extract<CellPopover, { kind: "note" }> }) {
  const ctl = useController();
  const { ref, style } = useFloating(p.rect.right + 6, p.rect.top);
  const meta = [p.note.author, when(p.note.created)].filter(Boolean).join(" · ");
  return (
    <div
      ref={ref}
      className={styles.note}
      style={style}
      role="tooltip"
      data-testid="note-popover"
      onMouseEnter={() => useV3Store.getState().setPopover({ ...p, pinned: true })}
      onMouseLeave={closePopover}
    >
      {meta ? <div className={styles.meta}>{meta}</div> : null}
      <div className={styles.noteText}>{p.note.text}</div>
      <div className={styles.noteActions}>
        <button type="button" className={styles.linkBtn} onClick={() => void ctl.v3.editNote(p.cell)}>
          {t("dialog.edit", "Edit")}
        </button>
        <button type="button" className={cx(styles.linkBtn, styles.danger)} onClick={() => void ctl.v3.deleteNote(p.cell)}>
          {t("dialog.delete", "Delete")}
        </button>
      </div>
    </div>
  );
}

function NoteEditor({ p }: { p: Extract<CellPopover, { kind: "noteEdit" }> }) {
  const ctl = useController();
  const { ref, style } = useFloating(p.rect.right + 6, p.rect.top);
  const [text, setText] = useState(p.text);
  const ta = useRef<HTMLTextAreaElement>(null);
  const cancel = () => {
    closePopover();
    ctl.focusGrid();
  };
  useDismiss(ref, cancel);
  useEffect(() => {
    ta.current?.focus();
    ta.current?.setSelectionRange(text.length, text.length);
    // Focus once on open.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const save = () => void ctl.v3.saveNote(p.cell, text, p.author);
  return (
    <div ref={ref} className={cx(styles.note, styles.editor)} style={style} role="dialog" aria-label={t("popover.note", "Note")} data-testid="note-editor">
      <div className={styles.meta}>{p.author ? t("popover.noteBy", "Note by {author}", { author: p.author }) : t("popover.note", "Note")}</div>
      <textarea
        ref={ta}
        className={styles.textarea}
        value={text}
        rows={4}
        placeholder={t("popover.addNote", "Add a note…")}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          e.stopPropagation();
          if (e.key === "Escape") {
            e.preventDefault();
            cancel();
          } else if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            save();
          }
        }}
        data-testid="note-text"
      />
      <div className={styles.editorActions}>
        {p.exists ? (
          <button type="button" className={cx(styles.btn, styles.danger)} onClick={() => void ctl.v3.deleteNote(p.cell)}>
            {t("dialog.delete", "Delete")}
          </button>
        ) : null}
        <span className={styles.spacer} />
        <button type="button" className={styles.btn} onClick={cancel}>
          {t("dialog.cancel", "Cancel")}
        </button>
        <button type="button" className={cx(styles.btn, styles.primary)} onClick={save} data-testid="note-save" title="⌘↩">
          {t("dialog.save", "Save")}
        </button>
      </div>
    </div>
  );
}

function ListPopover({ p }: { p: Extract<CellPopover, { kind: "list" }> }) {
  const ctl = useController();
  const { ref, style } = useFloating(p.rect.right, p.rect.bottom + 4, false, true);
  const [index, setIndex] = useState(() => Math.max(0, p.items.indexOf(p.current)));
  const cancel = () => {
    closePopover();
    ctl.focusGrid();
  };
  useDismiss(ref, cancel);
  const pick = (cell: CellPos, item: string) => void ctl.v3.pickListItem(cell, item);
  useEffect(() => {
    ref.current?.focus();
  }, [ref]);
  return (
    <div
      ref={ref}
      className={styles.list}
      style={style}
      role="listbox"
      aria-label={t("popover.allowedValues", "Allowed values")}
      tabIndex={-1}
      data-testid="validation-list"
      onKeyDown={(e) => {
        if (e.key === "ArrowDown") setIndex((i) => Math.min(p.items.length - 1, i + 1));
        else if (e.key === "ArrowUp") setIndex((i) => Math.max(0, i - 1));
        else if (e.key === "Enter") {
          const item = p.items[index];
          if (item !== undefined) pick(p.cell, item);
        } else return;
        e.preventDefault();
      }}
    >
      {p.items.length === 0 ? <div className={styles.empty}>{t("popover.noItems", "No items")}</div> : null}
      {p.items.map((item, i) => (
        <button
          key={`${i}:${item}`}
          type="button"
          role="option"
          aria-selected={item === p.current}
          className={cx(styles.item, i === index && styles.itemActive)}
          onMouseEnter={() => setIndex(i)}
          onClick={() => pick(p.cell, item)}
          data-item={item}
        >
          <span className={styles.check}>{item === p.current ? "✓" : ""}</span>
          {p.chips?.[item] ? (
            <span className={styles.chip} style={{ background: rgbaToHex(p.chips[item].bg), color: rgbaToHex(p.chips[item].fg) }}>
              {item}
            </span>
          ) : (
            item
          )}
        </button>
      ))}
    </div>
  );
}

function InputHelp({ p }: { p: Extract<CellPopover, { kind: "inputHelp" }> }) {
  const { ref, style } = useFloating(p.rect.left, p.rect.bottom + 6);
  return (
    <div ref={ref} className={cx(styles.note, styles.help)} style={style} role="tooltip" data-testid="input-help">
      {p.title ? <div className={styles.helpTitle}>{p.title}</div> : null}
      {p.text ? <div className={styles.noteText}>{p.text}</div> : null}
    </div>
  );
}
