/**
 * Font pickers, as in AOO's Formatting toolbar and Format ▸ Cells ▸ Font:
 *
 * - `FontNameBox`: the toolbar's font-name combo box. It shows the
 *   selection's family (the document font when the cells have none; blank
 *   when the selection mixes fonts), filters as you type (↑ ↓ Enter Esc),
 *   lists recently used fonts, the fonts in this document, then every
 *   installed family, each name drawn in its own face (a virtualised list,
 *   so hundreds of families stay fast). A name that is not installed can be
 *   typed and applied (a substitute draws it).
 * - `FontSizeBox`: the editable size combo (6–96 offered, 1–409 typed).
 * - `FontListField`: the dialog's search field + always-visible list.
 * - `FontRowsList`: the shared virtualised listbox.
 */
import { cssFontFamily } from "@tally/grid";
import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { cx, useController } from "../context";
import { useFontStore, type FontShell } from "../fonts/fontStore";
import { APP_FONT, FONT_SIZE_LIST, documentFonts, fontRows, formatFontSize, isInstalled, parseFontSize, sameFont, selectionFont, stepRow, typeAheadIndex, type FontRow, type FontSection, type SelectionFont } from "../fonts/fontModel";
import { t } from "../i18n";
import { useDocStore } from "../stores/docStore";
import { selectionRanges, useSelectionStore } from "../stores/selectionStore";
import { useW56Store } from "../data/store";
import { ChevronIcon } from "./icons";
import styles from "./FontPickers.module.css";

/** The CSS family list that previews `family` (its substitutes, then the app font). */
export function previewFamily(family: string): string {
  return cssFontFamily(family, "var(--tally-font-family)");
}

export function sectionLabel(s: FontSection): string {
  return s === "recent" ? t("font.recent", "Recently used") : s === "document" ? t("font.document", "Fonts in this document") : t("font.all", "All fonts");
}

/** What the font boxes show: the selection's font, the document font and the document's fonts. */
export interface FontInfo {
  /** `null` fields are mixed; `undefined` until the styles are loaded. */
  sel: SelectionFont | undefined;
  /** What cells without a family of their own use (style 0 / the document default). */
  docFont: string;
  docFonts: string[];
}

/**
 * The font boxes' data with ONE source subscription that re-renders only
 * when the loaded styles or the active cell's style change (coalesced per
 * frame); the selection scan and the document's font list are memoised on
 * (selection, document revision, loaded styles).
 */
export function useFontInfo(): FontInfo {
  const source = useDocStore((s) => s.source);
  const sheet = useDocStore((s) => s.activeSheet);
  const rev = useDocStore((s) => s.rev);
  const selection = useSelectionStore((s) => s.selection);
  const cellStyles = useW56Store((s) => s.cellStyles);
  const keyOf = () => {
    if (!source) return "";
    const a = useSelectionStore.getState().selection.active;
    return `${source.styleCount()}|${source.getCell(a.r, a.c)?.style ?? 0}`;
  };
  const [key, setKey] = useState(keyOf);
  useEffect(() => {
    if (!source) return undefined;
    let raf = 0;
    const off = source.subscribe(() => {
      if (raf) return;
      raf = requestAnimationFrame(() => {
        raf = 0;
        setKey(keyOf());
      });
    });
    return () => {
      off();
      if (raf) cancelAnimationFrame(raf);
    };
    // keyOf reads the latest state itself.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [source]);
  const docFont = useMemo(() => source?.getStyle(0)?.fontFamily || APP_FONT, [source, rev, key]);
  const sel = useMemo(
    () => (source ? selectionFont(source, selectionRanges(selection), selection.active, source.getLayout(sheet).used ?? null, APP_FONT) : undefined),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [source, selection, sheet, rev, key],
  );
  const styleCount = key.split("|")[0];
  const docFonts = useMemo(
    () => documentFonts(docFont, [...(source?.knownStyles() ?? []), ...cellStyles.map((s) => s.style)]),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [source, styleCount, cellStyles, docFont],
  );
  return { sel, docFont, docFonts };
}

/** The document's font for dialogs (no subscription: dialogs open on loaded documents). */
export function useDocumentFont(): string {
  const source = useDocStore((s) => s.source);
  useDocStore((s) => s.rev);
  return source?.getStyle(0)?.fontFamily || APP_FONT;
}

/** The fonts used in the document, for dialogs (computed once per open). */
export function useDocumentFonts(): string[] {
  const source = useDocStore((s) => s.source);
  const cellStyles = useW56Store((s) => s.cellStyles);
  const docFont = useDocumentFont();
  return useMemo(() => documentFonts(docFont, [...(source?.knownStyles() ?? []), ...cellStyles.map((s) => s.style)]), [source, cellStyles, docFont]);
}

/** Loads the installed font list once (the shell's `list_fonts`). */
export function useInstalledFonts() {
  const ctl = useController();
  const families = useFontStore((s) => s.families);
  const loaded = useFontStore((s) => s.loaded);
  useEffect(() => {
    if (!loaded) void useFontStore.getState().load(ctl.shell as unknown as FontShell);
  }, [ctl, loaded]);
  return families;
}

// ---------------------------------------------------------------------------
// The virtualised list
// ---------------------------------------------------------------------------

/** A row of a combo list: a header, or an option with its value. */
export type ComboRow = { kind: "header"; label: string } | { kind: "option"; value: string; label: string; style?: CSSProperties; hint?: string };

const ROW_H = 28;
const OVERSCAN = 6;

export function optionId(listId: string, i: number): string {
  return `${listId}-opt-${i}`;
}

interface RowsListProps {
  id: string;
  rows: readonly ComboRow[];
  active: number;
  /** Visible rows (the list scrolls beyond). */
  maxRows?: number;
  label: string;
  testId?: string;
  className?: string;
  onPick(i: number): void;
  onHover?(i: number): void;
  /** Keep `maxRows` rows of height however many rows there are (dialog lists). */
  fixed?: boolean;
}

/** A listbox drawing only the rows in view (plus a few), keeping the active row in view. */
export function FontRowsList({ id, rows, active, maxRows = 12, label, testId, className, onPick, onHover, fixed = false }: RowsListProps) {
  const ref = useRef<HTMLDivElement>(null);
  const [top, setTop] = useState(0);
  const height = (fixed ? maxRows : Math.max(1, Math.min(rows.length, maxRows))) * ROW_H;
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el || active < 0) return;
    const y = active * ROW_H;
    if (y < el.scrollTop) el.scrollTop = y;
    else if (y + ROW_H > el.scrollTop + height) el.scrollTop = y + ROW_H - height;
    setTop(el.scrollTop);
  }, [active, height]);
  const first = Math.max(0, Math.floor(top / ROW_H) - OVERSCAN);
  const last = Math.min(rows.length, Math.ceil((top + height) / ROW_H) + OVERSCAN);
  const items: ReactNode[] = [];
  for (let i = first; i < last; i++) {
    const row = rows[i]!;
    const pos: CSSProperties = { top: i * ROW_H, height: ROW_H };
    if (row.kind === "header") {
      items.push(
        <div key={i} role="presentation" className={styles.header} style={pos}>
          {row.label}
        </div>,
      );
    } else {
      items.push(
        <div
          key={i}
          id={optionId(id, i)}
          role="option"
          aria-selected={i === active}
          className={cx(styles.option, i === active && styles.optionOn)}
          style={pos}
          title={row.value}
          data-testid={testId ? `${testId}-option` : undefined}
          data-value={row.value}
          onMouseDown={(e) => e.preventDefault() /* keep focus in the field */}
          onMouseMove={() => onHover?.(i)}
          onClick={() => onPick(i)}
        >
          <span className={styles.optionText} style={row.style}>
            {row.label}
          </span>
          {row.hint ? <span className={styles.hint}>{row.hint}</span> : null}
        </div>,
      );
    }
  }
  return (
    <div ref={ref} id={id} role="listbox" aria-label={label} className={cx(styles.list, className)} style={{ height }} data-testid={testId} onScroll={(e) => setTop(e.currentTarget.scrollTop)}>
      <div style={{ position: "relative", height: rows.length * ROW_H }}>{items}</div>
    </div>
  );
}

/** Font rows as combo rows, each name in its own face. */
export function fontComboRows(rows: readonly FontRow[], installed: (name: string) => boolean): ComboRow[] {
  return rows.map((r) =>
    r.kind === "header" ? { kind: "header", label: sectionLabel(r.section) } : { kind: "option", value: r.family, label: r.family, style: { fontFamily: previewFamily(r.family) }, hint: installed(r.family) ? undefined : t("font.missing", "not installed") },
  );
}

// ---------------------------------------------------------------------------
// The toolbar combo boxes
// ---------------------------------------------------------------------------

interface ComboProps {
  label: string;
  testId: string;
  className?: string | undefined;
  listClassName?: string | undefined;
  /** What the box shows when not being edited ("" = mixed). */
  value: string;
  /** The rows for a typed query (`null`: the whole list) and the row to highlight. */
  rowsFor(query: string | null): { rows: ComboRow[]; active: number };
  /** Applies a picked or typed value; `false` keeps the box open (invalid input). */
  onCommit(text: string): boolean;
  /** Leaves the box (Esc, or after a commit). */
  onDone(): void;
  inputMode?: "text" | "decimal";
  maxRows?: number;
}

/** An editable combo box (role=combobox) with a pop-up listbox. */
export function ComboBox({ label, testId, className, listClassName, value, rowsFor, onCommit, onDone, inputMode = "text", maxRows }: ComboProps) {
  const listId = useId().replace(/:/g, "") + "-list";
  const inputRef = useRef<HTMLInputElement>(null);
  const [editing, setEditing] = useState(false);
  const [text, setText] = useState(value);
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState<string | null>(null);
  const [active, setActive] = useState(-1);
  const [anchor, setAnchor] = useState<{ x: number; y: number; w: number } | null>(null);
  const { rows } = useMemo(() => (open ? rowsFor(query) : { rows: [] as ComboRow[] }), [open, query, rowsFor]);

  const place = () => {
    const r = inputRef.current?.getBoundingClientRect();
    if (r) setAnchor({ x: r.left, y: r.bottom + 4, w: r.width });
  };
  const openList = (q: string | null) => {
    place();
    const res = rowsFor(q);
    setQuery(q);
    setActive(res.active);
    setOpen(true);
  };
  const close = () => {
    setOpen(false);
    setQuery(null);
    setActive(-1);
  };
  const finish = () => {
    close();
    setEditing(false);
    onDone();
  };
  const commit = (s: string) => {
    if (!onCommit(s)) return;
    finish();
  };
  const shown = editing ? text : value;
  const activeRow = active >= 0 ? rows[active] : undefined;
  return (
    <div className={cx(styles.combo, className)} data-testid={testId}>
      <input
        ref={inputRef}
        className={styles.comboInput}
        role="combobox"
        aria-label={label}
        aria-expanded={open}
        aria-controls={listId}
        aria-autocomplete="list"
        aria-activedescendant={open && activeRow?.kind === "option" ? optionId(listId, active) : undefined}
        title={label}
        value={shown}
        spellCheck={false}
        autoComplete="off"
        inputMode={inputMode}
        data-testid={`${testId}-input`}
        onFocus={(e) => {
          setEditing(true);
          setText(value);
          e.currentTarget.select();
        }}
        onBlur={() => {
          close();
          setEditing(false);
        }}
        onChange={(e) => {
          setEditing(true);
          setText(e.target.value);
          openList(e.target.value);
        }}
        onKeyDown={(e) => {
          if (e.key === "ArrowDown" || e.key === "ArrowUp") {
            e.preventDefault();
            if (!open) {
              openList(null);
              return;
            }
            const next = active < 0 ? stepComboRow(rows, e.key === "ArrowDown" ? -1 : rows.length, e.key === "ArrowDown" ? 1 : -1) : stepComboRow(rows, active, e.key === "ArrowDown" ? 1 : -1);
            setActive(next);
            const row = rows[next];
            if (row?.kind === "option") setText(row.value);
          } else if (e.key === "Enter") {
            e.preventDefault();
            const row = open && active >= 0 ? rows[active] : undefined;
            commit(row?.kind === "option" ? row.value : text);
          } else if (e.key === "Escape") {
            e.preventDefault();
            e.stopPropagation();
            if (open) close();
            else {
              setText(value);
              finish();
            }
          } else if (e.key === "Tab") {
            close();
          }
        }}
      />
      <button
        type="button"
        className={styles.comboButton}
        tabIndex={-1}
        aria-label={label}
        aria-hidden="true"
        data-testid={`${testId}-open`}
        onMouseDown={(e) => {
          e.preventDefault();
          if (open) {
            close();
            return;
          }
          inputRef.current?.focus();
          openList(null);
        }}
      >
        <ChevronIcon />
      </button>
      {open && anchor && rows.length > 0 ? (
        <div
          className={cx(styles.popup, listClassName)}
          style={{ left: anchor.x, top: anchor.y, minWidth: anchor.w }}
          // Headers, padding and the scrollbar must not take the focus from the field (which would close the list).
          onMouseDown={(e) => e.preventDefault()}
        >
          <FontRowsList
            id={listId}
            rows={rows}
            active={active}
            maxRows={maxRows}
            label={label}
            testId={`${testId}-list`}
            onHover={() => undefined}
            onPick={(i) => {
              const row = rows[i];
              if (row?.kind === "option") commit(row.value);
            }}
          />
        </div>
      ) : null}
    </div>
  );
}

function stepComboRow(rows: readonly ComboRow[], from: number, dir: 1 | -1): number {
  for (let i = from + dir; i >= 0 && i < rows.length; i += dir) if (rows[i]?.kind === "option") return i;
  return from >= 0 && from < rows.length ? from : -1;
}

/** The toolbar's font-name box. */
export function FontNameBox({ onDone, info }: { onDone(): void; info: FontInfo }) {
  const ctl = useController();
  const families = useInstalledFonts();
  const recent = useFontStore((s) => s.recent);
  const { docFont, docFonts, sel } = info;
  useW56Store((s) => s.i18nRev);
  const value = sel === undefined ? docFont : (sel.family ?? "");
  const installed = useMemo(() => {
    const set = new Set(families.map((f) => f.family.toLowerCase()));
    return (n: string) => set.has(n.toLowerCase());
  }, [families]);
  const rowsFor = useMemo(
    () => (query: string | null) => {
      const fr = fontRows(query ?? "", families, recent, docFonts);
      const rows = fontComboRows(fr, installed);
      if (query !== null) return { rows, active: typeAheadIndex(fr, query) };
      // Highlight the current family in its first section.
      const at = value === "" ? -1 : fr.findIndex((r) => r.kind === "font" && sameFont(r.family, value));
      return { rows, active: at >= 0 ? at : stepRow(fr, -1, 1) };
    },
    [families, recent, docFonts, installed, value],
  );
  return (
    <ComboBox
      label={t("font.name", "Font name")}
      testId="font-name"
      className={styles.nameBox}
      listClassName={styles.namePopup}
      value={value}
      rowsFor={rowsFor}
      onDone={onDone}
      onCommit={(text) => {
        const name = text.trim();
        // Nothing typed, or the font the selection already has: no undo step.
        if (name === "" || (value !== "" && sameFont(name, value))) return true;
        void ctl.setFontFamily(name, docFont);
        useFontStore.getState().remember(name);
        return true;
      }}
    />
  );
}

/** The toolbar's font-size box. */
export function FontSizeBox({ onDone, info }: { onDone(): void; info: FontInfo }) {
  const ctl = useController();
  const { sel } = info;
  useW56Store((s) => s.i18nRev);
  const size = sel === undefined ? 10 : sel.sizePt;
  const value = size === null ? "" : formatFontSize(size);
  const rowsFor = useMemo(
    () => (query: string | null) => {
      const rows: ComboRow[] = FONT_SIZE_LIST.map((pt) => ({ kind: "option", value: formatFontSize(pt), label: formatFontSize(pt) }));
      const want = query === null ? size : parseFontSize(query);
      return { rows, active: want === null ? -1 : FONT_SIZE_LIST.indexOf(want) };
    },
    [size],
  );
  return (
    <ComboBox
      label={t("toolbar.fontSize", "Font size")}
      testId="font-size"
      className={styles.sizeBox}
      value={value}
      inputMode="decimal"
      maxRows={10}
      rowsFor={rowsFor}
      onDone={onDone}
      onCommit={(text) => {
        const pt = parseFontSize(text);
        if (pt === null) {
          if (text.trim() !== "") ctl.notifyError({ code: "invalid_argument", message: t("font.sizeRange", "Font size must be between 1 and 409 pt.") });
          return text.trim() === "";
        }
        if (pt !== size) void ctl.setFontSize(pt);
        return true;
      }}
    />
  );
}

// ---------------------------------------------------------------------------
// Format Cells ▸ Font
// ---------------------------------------------------------------------------

interface FontListFieldProps {
  /** The family the field stands for ("" = indeterminate: a mixed selection); typed text shows as typed. */
  value: string;
  onChange(name: string): void;
  testId: string;
}

/** AOO's Font field: a text field (type a name or filter) over the list of families. */
export function FontListField({ value, onChange, testId }: FontListFieldProps) {
  const families = useInstalledFonts();
  const recent = useFontStore((s) => s.recent);
  const docFonts = useDocumentFonts();
  const listId = useId().replace(/:/g, "") + "-fonts";
  const [query, setQuery] = useState<string | null>(null);
  // What was typed is shown as typed (even empty); a pick shows `value` again.
  const [typed, setTyped] = useState<string | null>(null);
  const installed = useMemo(() => {
    const set = new Set(families.map((f) => f.family.toLowerCase()));
    return (n: string) => set.has(n.toLowerCase());
  }, [families]);
  const fr = useMemo(() => fontRows(query ?? "", families, recent, docFonts), [query, families, recent, docFonts]);
  const rows = useMemo(() => fontComboRows(fr, installed), [fr, installed]);
  const active = query !== null ? typeAheadIndex(fr, query) : value === "" ? -1 : fr.findIndex((r) => r.kind === "font" && sameFont(r.family, value));
  const pick = (i: number) => {
    const r = fr[i];
    if (r?.kind === "font") {
      setQuery(null);
      setTyped(null);
      onChange(r.family);
    }
  };
  return (
    <div className={styles.field}>
      <input
        className={styles.fieldInput}
        role="combobox"
        aria-label={t("font.name", "Font name")}
        aria-expanded
        aria-controls={listId}
        aria-autocomplete="list"
        aria-activedescendant={active >= 0 ? optionId(listId, active) : undefined}
        value={typed ?? value}
        placeholder={t("font.search", "Search fonts")}
        spellCheck={false}
        autoComplete="off"
        data-testid={testId}
        onChange={(e) => {
          setQuery(e.target.value);
          setTyped(e.target.value);
          onChange(e.target.value);
        }}
        onKeyDown={(e) => {
          if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
          e.preventDefault();
          const from = active >= 0 ? active : e.key === "ArrowDown" ? -1 : fr.length;
          pick(stepRow(fr, from, e.key === "ArrowDown" ? 1 : -1));
        }}
      />
      <FontRowsList id={listId} rows={rows} active={active} maxRows={8} fixed label={t("font.name", "Font name")} testId={`${testId}-list`} className={styles.fieldList} onPick={pick} />
    </div>
  );
}

/** A small fixed listbox (Typeface, Size) with optional per-option hints. */
export function SimpleList<T extends string | number>({ items, value, onPick, label, testId, maxRows = 8 }: { items: readonly { value: T; label: string; hint?: string; style?: CSSProperties }[]; value: T | null; onPick(v: T): void; label: string; testId: string; maxRows?: number }) {
  const listId = useId().replace(/:/g, "") + "-list";
  const rows: ComboRow[] = items.map((x) => ({ kind: "option", value: String(x.value), label: x.label, hint: x.hint, style: x.style }));
  const active = value === null ? -1 : items.findIndex((x) => x.value === value);
  return (
    <div
      tabIndex={0}
      className={styles.simple}
      aria-label={label}
      aria-activedescendant={active >= 0 ? optionId(listId, active) : undefined}
      onKeyDown={(e) => {
        if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
        e.preventDefault();
        const next = Math.max(0, Math.min(items.length - 1, active + (e.key === "ArrowDown" ? 1 : -1)));
        const v = items[next];
        if (v) onPick(v.value);
      }}
    >
      <FontRowsList id={listId} rows={rows} active={active} maxRows={maxRows} label={label} testId={testId} className={styles.fieldList} onPick={(i) => items[i] && onPick(items[i].value)} />
    </div>
  );
}

/** Whether `name` needs the "not installed" note. */
export function fontMissing(name: string, families: Parameters<typeof isInstalled>[1]): boolean {
  return name.trim() !== "" && !isInstalled(name.trim(), families);
}
