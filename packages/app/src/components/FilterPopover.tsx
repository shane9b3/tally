/**
 * The AutoFilter popover of one column (opened from the ▾ in its header
 * cell): sort shortcuts, a searchable checklist of the column's distinct
 * values with counts, an optional condition (operator + value), and Clear.
 * Apply sends one `setColumnFilter`.
 */
import type { FilterOp } from "@tally/bridge";
import { useEffect, useMemo, useRef, useState } from "react";
import { useController } from "../context";
import { useW56Store } from "../data/store";
import { t } from "../i18n";
import { columnFilterFrom, FILTER_OPS } from "../dataOps";
import { useDataStore } from "../stores/dataStore";
import { useDocStore } from "../stores/docStore";
import { useUiStore, type FilterPopoverState } from "../stores/uiStore";
import { useDismiss, useFloating } from "./Popover";
import styles from "./Panel.module.css";

export function FilterPopover() {
  const pop = useUiStore((s) => s.filterPopover);
  const filter = useDataStore((s) => s.filter);
  useW56Store((s) => s.i18nRev);
  if (!pop || !filter?.range) return null;
  return <Panel key={`${pop.col}:${pop.x}:${pop.y}`} pop={pop} />;
}

/** Label for an entry of `unique` (empty cells are `""`). */
export function valueLabel(v: string): string {
  return v === "" ? t("filter.empty", "(empty)") : v;
}

/** Translated label of a condition operator (`fallback` is the English one from FILTER_OPS). */
function opLabel(op: FilterOp, fallback: string): string {
  switch (op) {
    case "eq":
      return t("filter.op.eq", "equals");
    case "ne":
      return t("filter.op.ne", "does not equal");
    case "gt":
      return t("filter.op.gt", "greater than");
    case "ge":
      return t("filter.op.ge", "at least");
    case "lt":
      return t("filter.op.lt", "less than");
    case "le":
      return t("filter.op.le", "at most");
    case "contains":
      return t("filter.op.contains", "contains");
    case "beginsWith":
      return t("filter.op.beginsWith", "begins with");
    case "endsWith":
      return t("filter.op.endsWith", "ends with");
    case "empty":
      return t("filter.op.empty", "is empty");
    case "notEmpty":
      return t("filter.op.notEmpty", "is not empty");
    default:
      return fallback;
  }
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
      .sortWith({ range: { ...range }, keys: [{ col: pop.col, ascending: asc }], hasHeader: true }, asc ? t("filter.sortAscending", "Sort ascending") : t("filter.sortDescending", "Sort descending"))
      .then(() => ctl.focusGrid());
  };

  return (
    <div
      ref={ref}
      className={styles.card}
      style={{ ...style, width: 268 }}
      role="dialog"
      aria-label={t("filter.ariaLabel", "Filter {name}", { name: header || t("filter.column", "column") })}
      data-testid="filter-popover"
      onKeyDown={(e) => {
        if (e.key === "Enter" && !nothing) {
          e.preventDefault();
          apply();
        }
      }}
    >
      <div className={styles.headerRow}>
        <div className={styles.title}>{header || t("filter.title", "Filter")}</div>
        <button type="button" className={styles.chip} onClick={() => sort(true)} title={t("filter.sortAZ", "Sort the table by this column, A to Z")}>
          A→Z
        </button>
        <button type="button" className={styles.chip} onClick={() => sort(false)} title={t("filter.sortZA", "Sort the table by this column, Z to A")}>
          Z→A
        </button>
      </div>
      <input
        ref={searchRef}
        className={`${styles.input} ${styles.full}`}
        placeholder={t("filter.searchValues", "Search values")}
        aria-label={t("filter.searchValues", "Search values")}
        value={search}
        onChange={(e) => setSearch(e.target.value)}
        data-testid="filter-search"
      />
      <div className={styles.list} role="group" aria-label={t("find.values", "Values")} data-testid="filter-values">
        <label className={`${styles.check} ${styles.allRow}`}>
          <input type="checkbox" checked={allShownChecked} onChange={toggleShown} data-testid="filter-all" />
          <span className={styles.checkLabel}>{q === "" ? t("autofilter.selectAll", "Select all") : t("filter.selectAllMatches", "Select all matches")}</span>
          <span className={styles.count}>{shown.length}</span>
        </label>
        {shown.map((x) => (
          <label key={x.v} className={styles.check} data-value={x.v}>
            <input type="checkbox" checked={checked.has(x.v)} onChange={() => toggle(x.v)} />
            <span className={`${styles.checkLabel} ${x.v === "" ? styles.muted : ""}`}>{valueLabel(x.v)}</span>
            <span className={styles.count}>{x.n}</span>
          </label>
        ))}
        {shown.length === 0 ? <div className={`${styles.check} ${styles.muted}`}>{t("filter.noMatches", "No matches")}</div> : null}
      </div>
      <div className={styles.row}>
        <select
          className={styles.select}
          aria-label={t("cf.condition", "Condition")}
          value={op ?? ""}
          onChange={(e) => setOp(e.target.value === "" ? null : (e.target.value as FilterOp))}
          data-testid="filter-op"
        >
          <option value="">{t("filter.noCondition", "No condition")}</option>
          {FILTER_OPS.map((o) => (
            <option key={o.op} value={o.op}>
              {opLabel(o.op, o.label)}
            </option>
          ))}
        </select>
        <input
          className={styles.input}
          aria-label={t("filter.conditionValue", "Condition value")}
          placeholder={op === null ? "" : t("dialog.value", "Value")}
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
          {t("filter.clear", "Clear filter")}
        </button>
        <span className={styles.spacer} />
        <button type="button" className={styles.btn} onClick={close}>
          {t("dialog.cancel", "Cancel")}
        </button>
        <button type="button" className={styles.primary} disabled={nothing} onClick={apply} data-testid="filter-apply">
          {t("dialog.apply", "Apply")}
        </button>
      </div>
    </div>
  );
}
