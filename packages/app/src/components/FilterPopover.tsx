/**
 * The AutoFilter popover of one column (opened from the ▾ in its header
 * cell): sort shortcuts, a searchable checklist of the column's distinct
 * values with counts, an optional condition (operator + value), and Clear.
 * Apply sends one `setColumnFilter`.
 */
import type { FilterOp } from "@tally/bridge";
import { useEffect, useMemo, useRef, useState } from "react";
import { useController } from "../context";
import { columnFilterFrom, FILTER_OPS } from "../dataOps";
import { useDataStore } from "../stores/dataStore";
import { useDocStore } from "../stores/docStore";
import { useUiStore, type FilterPopoverState } from "../stores/uiStore";
import { useDismiss, useFloating } from "./Popover";
import styles from "./Panel.module.css";

export function FilterPopover() {
  const pop = useUiStore((s) => s.filterPopover);
  const filter = useDataStore((s) => s.filter);
  if (!pop || !filter?.range) return null;
  return <Panel key={`${pop.col}:${pop.x}:${pop.y}`} pop={pop} />;
}

/** Label for an entry of `unique` (empty cells are `""`). */
export function valueLabel(v: string): string {
  return v === "" ? "(empty)" : v;
}

function Panel({ pop }: { pop: FilterPopoverState }) {
  const ctl = useController();
  const filter = useDataStore((s) => s.filter);
  const source = useDocStore((s) => s.source);
  const info = filter?.columns.find((c) => c.col === pop.col);
  const all = useMemo(() => info?.unique ?? [], [info]);
  const counts = info?.counts ?? [];
  const active = info?.active ?? null;
  const [checked, setChecked] = useState<Set<string>>(() => new Set(active?.values ?? all));
  const [search, setSearch] = useState("");
  const [op, setOp] = useState<FilterOp | null>(active?.condition?.op ?? null);
  const [value, setValue] = useState(active?.condition?.value ?? "");
  const close = () => useUiStore.getState().openFilterPopover(null);
  const { ref, style } = useFloating(pop.x, pop.y);
  useDismiss(ref, close);
  const searchRef = useRef<HTMLInputElement>(null);
  useEffect(() => searchRef.current?.focus(), []);

  const header = filter?.range ? (source?.getCell(filter.range.r0, pop.col)?.text ?? "") : "";
  const q = search.trim().toLowerCase();
  const shown = all.map((v, i) => ({ v, n: counts[i] ?? 0 })).filter((x) => q === "" || valueLabel(x.v).toLowerCase().includes(q));
  const allShownChecked = shown.length > 0 && shown.every((x) => checked.has(x.v));
  const needsValue = FILTER_OPS.find((o) => o.op === op)?.value ?? true;
  const result = columnFilterFrom(all, checked, { op, value });
  const nothing = checked.size === 0 && op === null;

  const toggle = (v: string) =>
    setChecked((prev) => {
      const next = new Set(prev);
      if (next.has(v)) next.delete(v);
      else next.add(v);
      return next;
    });
  const toggleShown = () =>
    setChecked((prev) => {
      const next = new Set(prev);
      for (const x of shown) {
        if (allShownChecked) next.delete(x.v);
        else next.add(x.v);
      }
      return next;
    });
  const apply = () => {
    close();
    void ctl.setColumnFilter(pop.col, result);
    ctl.focusGrid();
  };
  const sort = (asc: boolean) => {
    const range = filter?.range;
    close();
    if (!range) return;
    void ctl
      .sortWith({ range: { ...range }, keys: [{ col: pop.col, ascending: asc }], hasHeader: true }, asc ? "Sort ascending" : "Sort descending")
      .then(() => ctl.focusGrid());
  };

  return (
    <div
      ref={ref}
      className={styles.card}
      style={{ ...style, width: 268 }}
      role="dialog"
      aria-label={`Filter ${header || "column"}`}
      data-testid="filter-popover"
      onKeyDown={(e) => {
        if (e.key === "Enter" && !nothing) {
          e.preventDefault();
          apply();
        }
      }}
    >
      <div className={styles.headerRow}>
        <div className={styles.title}>{header || "Filter"}</div>
        <button type="button" className={styles.chip} onClick={() => sort(true)} title="Sort the table by this column, A to Z">
          A→Z
        </button>
        <button type="button" className={styles.chip} onClick={() => sort(false)} title="Sort the table by this column, Z to A">
          Z→A
        </button>
      </div>
      <input
        ref={searchRef}
        className={`${styles.input} ${styles.full}`}
        placeholder="Search values"
        aria-label="Search values"
        value={search}
        onChange={(e) => setSearch(e.target.value)}
        data-testid="filter-search"
      />
      <div className={styles.list} role="group" aria-label="Values" data-testid="filter-values">
        <label className={`${styles.check} ${styles.allRow}`}>
          <input type="checkbox" checked={allShownChecked} onChange={toggleShown} data-testid="filter-all" />
          <span className={styles.checkLabel}>{q === "" ? "Select all" : "Select all matches"}</span>
          <span className={styles.count}>{shown.length}</span>
        </label>
        {shown.map((x) => (
          <label key={x.v} className={styles.check} data-value={x.v}>
            <input type="checkbox" checked={checked.has(x.v)} onChange={() => toggle(x.v)} />
            <span className={`${styles.checkLabel} ${x.v === "" ? styles.muted : ""}`}>{valueLabel(x.v)}</span>
            <span className={styles.count}>{x.n}</span>
          </label>
        ))}
        {shown.length === 0 ? <div className={`${styles.check} ${styles.muted}`}>No matches</div> : null}
      </div>
      <div className={styles.row}>
        <select
          className={styles.select}
          aria-label="Condition"
          value={op ?? ""}
          onChange={(e) => setOp(e.target.value === "" ? null : (e.target.value as FilterOp))}
          data-testid="filter-op"
        >
          <option value="">No condition</option>
          {FILTER_OPS.map((o) => (
            <option key={o.op} value={o.op}>
              {o.label}
            </option>
          ))}
        </select>
        <input
          className={styles.input}
          aria-label="Condition value"
          placeholder={op === null ? "" : "Value"}
          disabled={op === null || !needsValue}
          value={needsValue ? value : ""}
          onChange={(e) => setValue(e.target.value)}
          data-testid="filter-value"
        />
      </div>
      <div className={styles.footer}>
        <button
          type="button"
          className={styles.link}
          disabled={active === null}
          onClick={() => {
            close();
            void ctl.setColumnFilter(pop.col, null);
            ctl.focusGrid();
          }}
          data-testid="filter-clear"
        >
          Clear filter
        </button>
        <span className={styles.spacer} />
        <button type="button" className={styles.btn} onClick={close}>
          Cancel
        </button>
        <button type="button" className={styles.primary} disabled={nothing} onClick={apply} data-testid="filter-apply">
          Apply
        </button>
      </div>
    </div>
  );
}
