/**
 * The W5-6B data-tool dialogs: Standard / Advanced Filter, Subtotals,
 * Consolidate, Text to Columns, Remove Duplicates, Scenarios, Define
 * Database Range, Create Names, Fill Series, Sort options, Delete Contents,
 * Multiple Operations, Insert / Delete Cells, Copy / Show Sheet, Tab
 * Colour, Show Outline Level, Solver, the merge prompt (§51), document
 * statistics, recovery, Save as Template and the ⌃D selection list.
 */
import type { AggFn, DeleteContentsFlags, NameLabelFlags, ScenarioDto, SolverConstraint, SolverOp, SolverResult, StdFilterOptions } from "@tally/bridge";
import { useEffect, useMemo, useRef, useState } from "react";
import { cx, useController } from "../context";
import { a1 } from "../dataOps";
import { t } from "../i18n";
import { rgbaToHex } from "../palette";
import { useDocStore } from "../stores/docStore";
import { useUiStore } from "../stores/uiStore";
import dlg from "../components/Dialogs.module.css";
import {
  BUILTIN_SORT_LISTS,
  DEFAULT_DELETE,
  DEFAULT_FILL_SERIES,
  DEFAULT_FILTER_OPTIONS,
  DEFAULT_SUBTOTAL_OPTIONS,
  DELETE_FLAG_LABELS,
  SCENARIO_COLORS,
  STD_FILTER_OPS,
  SUBTOTAL_FNS,
  TAB_COLORS,
  advancedFilterOp,
  consolidateOp,
  createNamesOp,
  defineDatabaseRangeOp,
  emptyFilterRows,
  emptySubtotalTabs,
  fillSeriesOp,
  filterConditions,
  filterError,
  isValidDbRangeName,
  multipleOperationsOps,
  previewSeries,
  removeDuplicatesOp,
  removeSubtotalsOp,
  sortWithOptionsOp,
  standardFilterOp,
  subtotalGroups,
  subtotalsOp,
  type ConsolidateState,
  type FillSeriesState,
  type FilterRow,
  type MultiOpsState,
  type SortOptionsState,
  type SubtotalTab,
} from "./builders";
import { COLUMN_TYPES, DEFAULT_SEPARATORS, guessSeparator, previewColumns, previewSplit, toCsvOptions, type SeparatorState } from "./textToColumns";
import { useW56Store, type W56Dialog } from "./store";
import { Check, Field, RangeField, Segmented, Tabs, W56Shell, closeW56, parseRangeText } from "./W56Shell";
import styles from "./W56.module.css";

type D<K extends W56Dialog["kind"]> = Extract<W56Dialog, { kind: K }>;

/** "Objective: {objective} after {n} iterations." with the figure in bold (word order stays translatable). */
function solverSummary(objective: string, iterations: number) {
  const parts = t("dd.solverResult", "Objective: {objective} after {n} iterations.", { objective: "\u0000", n: iterations }).split("\u0000");
  return (
    <>
      {parts[0]}
      <b>{objective}</b>
      {parts[1]}
    </>
  );
}

const colLetter = (c: number): string => a1({ r0: 0, c0: c, r1: 0, c1: c }).replace(/\d+$/, "");

// ---------------------------------------------------------------------------
// Standard / Advanced filter
// ---------------------------------------------------------------------------

export function StandardFilterDialog({ d }: { d: D<"standardFilter"> }) {
  useW56Store((s) => s.i18nRev);
  const ctl = useController();
  const [rows, setRows] = useState<FilterRow[]>(() => emptyFilterRows(d.range.c0));
  const [options, setOptions] = useState<StdFilterOptions>({ ...DEFAULT_FILTER_OPTIONS, hasHeader: d.hasHeader });
  const [copyTo, setCopyTo] = useState("");
  const [criteria, setCriteria] = useState("");
  const [advanced, setAdvanced] = useState(d.advanced);
  const cols = useMemo(() => d.labels.map((label, i) => ({ col: d.range.c0 + i, label })), [d]);
  const conds = filterConditions(rows);
  const copyCell = copyTo.trim() === "" ? null : ctl.w56.parseCellRef(copyTo);
  const criteriaRange = parseRangeText(criteria);
  const error = advanced ? (criteriaRange ? null : t("dd.chooseCriteria", "Choose the criteria range.")) : filterError(conds, options);
  const copyError = copyTo.trim() !== "" && !copyCell ? t("dd.notACell", "“{ref}” is not a cell.", { ref: copyTo }) : null;
  const setRow = (i: number, p: Partial<FilterRow>) => setRows((rs) => rs.map((r, j) => (j === i ? { ...r, ...p } : r)));
  const submit = async () => {
    const opts: StdFilterOptions = { ...options, copyTo: copyCell };
    const op = advanced && criteriaRange ? advancedFilterOp(ctl.sheet, d.range, { sheet: ctl.sheet, range: criteriaRange }, opts) : standardFilterOp(ctl.sheet, d.range, conds, opts);
    await ctl.w56.applyOps(advanced ? t("msg.undo.advancedFilter", "Advanced filter") : t("msg.undo.standardFilter", "Standard filter"), [op]);
    await ctl.refreshFilter();
  };
  return (
    <W56Shell title={advanced ? t("cmd.data.advancedFilter", "Advanced filter").replace(/…$/, "") : t("cmd.data.standardFilter", "Standard filter").replace(/…$/, "")} ok={t("dialog.ok", "OK")} size="xwide" testId="standard-filter" valid={error === null && copyError === null} onSubmit={submit}>
      <p className={dlg.lead}>
        {options.hasHeader
          ? t("dd.filtersLeadHeader", "Filters {range} (first row = labels). Rows that do not match are hidden; Data ▸ Filter ▸ Reset Filter shows them again.", { range: a1(d.range) })
          : t("dd.filtersLead", "Filters {range}. Rows that do not match are hidden; Data ▸ Filter ▸ Reset Filter shows them again.", { range: a1(d.range) })}
      </p>
      <Segmented label={t("dd.filterKind", "Filter kind")} value={advanced ? "advanced" : "standard"} onChange={(v) => setAdvanced(v === "advanced")} options={[{ id: "standard", label: t("dd.conditions", "Conditions") }, { id: "advanced", label: t("dd.criteriaRange", "Criteria range") }]} />
      {advanced ? (
        <div className={dlg.gap}>
          <RangeField label={t("dd.readCriteriaFrom", "Read filter criteria from")} value={criteria} onChange={setCriteria} testId="filter-criteria" />
          <p className={styles.muted}>{t("dd.criteriaHelp", "The first row of the criteria range holds column labels; each further row is one alternative (OR), the cells of a row combine with AND.")}</p>
        </div>
      ) : (
        <div className={dlg.gap}>
          <div className={styles.condRow}>
            <span className={styles.rowLabel}>{t("dd.operator", "Operator")}</span>
            <span className={styles.rowLabel}>{t("dd.fieldName", "Field name")}</span>
            <span className={styles.rowLabel}>{t("dd.condition", "Condition")}</span>
            <span className={styles.rowLabel}>{t("dialog.value", "Value")}</span>
          </div>
          {rows.map((r, i) => {
            const disabled = i > 0 && rows[i - 1]?.col === null;
            return (
              <div key={i} className={styles.condRow}>
                {i === 0 ? (
                  <span />
                ) : (
                  <select className={dlg.selectSmall} aria-label={t("dd.operatorN", "Operator {n}", { n: i })} value={r.connector} disabled={disabled} onChange={(e) => setRow(i, { connector: e.target.value as "and" | "or" })} data-testid={`filter-conn-${i}`}>
                    <option value="and">{t("dd.opAnd", "AND")}</option>
                    <option value="or">{t("dd.opOr", "OR")}</option>
                  </select>
                )}
                <select className={dlg.selectSmall} aria-label={t("dd.fieldN", "Field {n}", { n: i + 1 })} value={r.col === null ? "" : String(r.col)} disabled={disabled} onChange={(e) => setRow(i, { col: e.target.value === "" ? null : Number(e.target.value) })} data-testid={`filter-field-${i}`}>
                  <option value="">{t("dd.none", "- none -")}</option>
                  {cols.map((c) => (
                    <option key={c.col} value={String(c.col)}>
                      {c.label}
                    </option>
                  ))}
                </select>
                <select className={dlg.selectSmall} aria-label={t("dd.conditionN", "Condition {n}", { n: i + 1 })} value={r.op} disabled={disabled || r.col === null} onChange={(e) => setRow(i, { op: e.target.value as FilterRow["op"] })} data-testid={`filter-op-${i}`}>
                  {STD_FILTER_OPS.map((o) => (
                    <option key={o.op} value={o.op}>
                      {o.label}
                    </option>
                  ))}
                </select>
                <input className={cx(dlg.input, styles.small)} style={{ width: "100%", height: 26 }} aria-label={t("dd.valueN", "Value {n}", { n: i + 1 })} value={r.value} disabled={disabled || r.col === null} onChange={(e) => setRow(i, { value: e.target.value })} data-testid={`filter-value-${i}`} />
              </div>
            );
          })}
        </div>
      )}
      <div className={styles.section}>{t("dialog.options", "Options")}</div>
      <div className={dlg.optionGrid}>
        <Check label={t("sort.caseSensitive", "Case sensitive")} checked={options.caseSensitive} onChange={(v) => setOptions({ ...options, caseSensitive: v })} />
        <Check label={t("find.regex", "Regular expressions")} checked={options.regex} onChange={(v) => setOptions({ ...options, regex: v })} testId="filter-regex" />
        <Check label={t("sort.hasHeader", "Range contains column labels")} checked={options.hasHeader} onChange={(v) => setOptions({ ...options, hasHeader: v })} />
        <Check label={t("dd.noDuplicates", "No duplications")} checked={options.noDuplicates} onChange={(v) => setOptions({ ...options, noDuplicates: v })} testId="filter-nodup" />
        <Check label={t("dd.keepCriteria", "Keep filter criteria")} checked={options.keepCriteria} onChange={(v) => setOptions({ ...options, keepCriteria: v })} />
      </div>
      <div className={dlg.gap}>
        <RangeField label={t("dd.copyResultsOptional", "Copy results to (optional)")} value={copyTo} onChange={setCopyTo} testId="filter-copy-to" placeholder={t("dd.cellOrSheetRef", "H1 or Sheet2.A1")} allowSheet />
      </div>
      {error ? <p className={dlg.error}>{error}</p> : null}
      {copyError ? <p className={dlg.error}>{copyError}</p> : null}
    </W56Shell>
  );
}

// ---------------------------------------------------------------------------
// Subtotals
// ---------------------------------------------------------------------------

export function SubtotalsDialog({ d }: { d: D<"subtotals"> }) {
  useW56Store((s) => s.i18nRev);
  const ctl = useController();
  const [tabs, setTabs] = useState<SubtotalTab[]>(() => {
    const first = emptySubtotalTabs();
    const t0 = first[0];
    if (t0) {
      t0.groupBy = d.range.c0;
      t0.columns = d.labels.length > 1 ? [d.range.c1] : [];
    }
    return first;
  });
  const [tab, setTab] = useState<"g1" | "g2" | "g3" | "options">("g1");
  const [options, setOptions] = useState(DEFAULT_SUBTOTAL_OPTIONS);
  const cols = d.labels.map((label, i) => ({ col: d.range.c0 + i, label }));
  const groups = subtotalGroups(tabs);
  const gi = tab === "g1" ? 0 : tab === "g2" ? 1 : tab === "g3" ? 2 : -1;
  const cur = gi >= 0 ? tabs[gi] : undefined;
  const update = (p: Partial<SubtotalTab>) => setTabs((ts) => ts.map((x, i) => (i === gi ? { ...x, ...p } : x)));
  const toggleCol = (c: number) => cur && update({ columns: cur.columns.includes(c) ? cur.columns.filter((x) => x !== c) : [...cur.columns, c] });
  return (
    <W56Shell
      title={t("subtotals.title", "Subtotals")}
      ok={t("dialog.ok", "OK")}
      size="wide"
      testId="subtotals-dialog"
      valid={groups.length > 0}
      onSubmit={() => void ctl.w56.applyOps(t("subtotals.title", "Subtotals"), [subtotalsOp(ctl.sheet, d.range, groups, options)])}
      extra={
        <button type="button" className={dlg.secondary} onClick={() => void ctl.w56.applyOps(t("msg.undo.removeSubtotals", "Remove subtotals"), [removeSubtotalsOp(ctl.sheet, d.range)]).then(() => closeW56())} data-testid="subtotals-remove">
          {t("subtotals.removeAll", "Remove all")}
        </button>
      }
    >
      <p className={dlg.lead}>{t("dd.subtotalsLead", "Inserts SUBTOTAL rows for {range} and builds an outline.", { range: a1(d.range) })}</p>
      <Tabs label={t("dd.groups", "Groups")} value={tab} onChange={setTab} tabs={[{ id: "g1", label: t("dd.group1", "1st Group") }, { id: "g2", label: t("dd.group2", "2nd Group") }, { id: "g3", label: t("dd.group3", "3rd Group") }, { id: "options", label: t("dialog.options", "Options") }]} />
      {cur ? (
        <>
          <Field label={t("subtotals.groupBy", "Group by")}>
            <select className={dlg.select} value={cur.groupBy === null ? "" : String(cur.groupBy)} onChange={(e) => update({ groupBy: e.target.value === "" ? null : Number(e.target.value) })} data-testid={`subtotal-groupby-${gi}`}>
              <option value="">{t("dd.none", "- none -")}</option>
              {cols.map((c) => (
                <option key={c.col} value={String(c.col)}>
                  {c.label}
                </option>
              ))}
            </select>
          </Field>
          <div className={styles.section}>{t("subtotals.calculate", "Calculate subtotals for")}</div>
          <div className={styles.chips}>
            {cols.map((c) => (
              <button key={c.col} type="button" className={cur.columns.includes(c.col) ? styles.chipOn : styles.chip} aria-pressed={cur.columns.includes(c.col)} onClick={() => toggleCol(c.col)} data-testid={`subtotal-col-${c.col}`}>
                {c.label}
              </button>
            ))}
          </div>
          <Field label={t("subtotals.useFunction", "Use function")} className={dlg.gap}>
            <select className={dlg.select} value={cur.function} onChange={(e) => update({ function: e.target.value as AggFn })} data-testid={`subtotal-fn-${gi}`}>
              {SUBTOTAL_FNS.map((f) => (
                <option key={f.fn} value={f.fn}>
                  {f.label}
                </option>
              ))}
            </select>
          </Field>
        </>
      ) : (
        <div>
          <Check label={t("subtotals.pageBreak", "Page break between groups")} checked={options.pageBreaks} onChange={(v) => setOptions({ ...options, pageBreaks: v })} />
          <Check label={t("sort.caseSensitive", "Case sensitive")} checked={options.caseSensitive} onChange={(v) => setOptions({ ...options, caseSensitive: v })} />
          <Check label={t("dd.preSort", "Pre-sort area according to groups")} checked={options.presort} onChange={(v) => setOptions({ ...options, presort: v })} />
          <Check label={t("sort.ascending", "Ascending")} checked={options.ascending} disabled={!options.presort} onChange={(v) => setOptions({ ...options, ascending: v })} />
          <Check label={t("dd.includeFormats", "Include formats")} checked={options.includeFormats} disabled={!options.presort} onChange={(v) => setOptions({ ...options, includeFormats: v })} />
        </div>
      )}
    </W56Shell>
  );
}

// ---------------------------------------------------------------------------
// Consolidate
// ---------------------------------------------------------------------------

export function ConsolidateDialog() {
  useW56Store((s) => s.i18nRev);
  const ctl = useController();
  const [st, setSt] = useState<ConsolidateState>({ function: "sum", sources: [], dest: "", byRowLabels: false, byColLabels: false, link: false });
  const [draft, setDraft] = useState(a1(ctl.selection.range));
  const op = consolidateOp(st, ctl.w56.sheetByName, ctl.sheet);
  const error = typeof op === "string" ? op : null;
  const addSource = () => {
    if (draft.trim() === "") return;
    setSt({ ...st, sources: [...st.sources, draft.trim()] });
    setDraft("");
  };
  return (
    <W56Shell title={t("consolidate.title", "Consolidate")} ok={t("dialog.ok", "OK")} size="wide" testId="consolidate-dialog" valid={error === null} onSubmit={() => {
        if (typeof op !== "string") void ctl.w56.applyOps(t("consolidate.title", "Consolidate"), [op]);
      }}>
      <Field label={t("consolidate.function", "Function")}>
        <select className={dlg.select} value={st.function} onChange={(e) => setSt({ ...st, function: e.target.value as AggFn })} data-testid="consolidate-fn">
          {SUBTOTAL_FNS.map((f) => (
            <option key={f.fn} value={f.fn}>
              {f.label}
            </option>
          ))}
        </select>
      </Field>
      <div className={styles.section}>{t("consolidate.sourceRanges", "Consolidation ranges")}</div>
      <div className={styles.list} data-testid="consolidate-sources">
        {st.sources.length === 0 ? <div className={cx(styles.listItem, styles.muted)}>{t("dd.noRangesYet", "No ranges yet.")}</div> : null}
        {st.sources.map((s, i) => (
          <div key={`${s}-${i}`} className={styles.listItem}>
            <span className={styles.grow}>{s}</span>
            <button type="button" className={dlg.rowBtn} onClick={() => setSt({ ...st, sources: st.sources.filter((_, j) => j !== i) })}>
              {t("dialog.remove", "Remove")}
            </button>
          </div>
        ))}
      </div>
      <div className={styles.row}>
        <div style={{ flex: 1 }}>
          <RangeField label={t("dd.sourceDataRange", "Source data range")} value={draft} onChange={setDraft} testId="consolidate-source" allowSheet />
        </div>
        <button type="button" className={dlg.secondary} style={{ marginTop: 18 }} onClick={addSource} data-testid="consolidate-add">
          {t("dialog.add", "Add")}
        </button>
      </div>
      <RangeField label={t("consolidate.target", "Copy results to")} value={st.dest} onChange={(v) => setSt({ ...st, dest: v })} testId="consolidate-dest" allowSheet placeholder={t("dd.sheetCellRef", "Sheet2.A1")} />
      <div className={styles.section}>{t("dd.consolidateBy", "Consolidate by")}</div>
      <div className={dlg.optionGrid}>
        <Check label={t("dd.rowLabels", "Row labels")} checked={st.byRowLabels} onChange={(v) => setSt({ ...st, byRowLabels: v })} />
        <Check label={t("dd.columnLabels", "Column labels")} checked={st.byColLabels} onChange={(v) => setSt({ ...st, byColLabels: v })} />
        <Check label={t("consolidate.linkToSource", "Link to source data")} checked={st.link} onChange={(v) => setSt({ ...st, link: v })} />
      </div>
      {error && st.sources.length > 0 ? <p className={dlg.error}>{error}</p> : null}
    </W56Shell>
  );
}

// ---------------------------------------------------------------------------
// Text to columns
// ---------------------------------------------------------------------------

export function TextToColumnsDialog({ d }: { d: D<"textToColumns"> }) {
  useW56Store((s) => s.i18nRev);
  const ctl = useController();
  const [st, setSt] = useState<SeparatorState>(() => ({ ...DEFAULT_SEPARATORS, ...guessSeparator(d.texts) }));
  const preview = useMemo(() => previewSplit(d.texts, st), [d.texts, st]);
  const nCols = previewColumns(d.texts, st);
  const typeOf = (c: number) => st.columnTypes[c] ?? "standard";
  const setType = (c: number, type: SeparatorState["columnTypes"][number]) => {
    const types = [...st.columnTypes];
    while (types.length < nCols) types.push("standard");
    types[c] = type;
    setSt({ ...st, columnTypes: types });
  };
  return (
    <W56Shell title={t("textToColumns.title", "Text to Columns")} ok={t("dialog.ok", "OK")} size="xwide" testId="text-to-columns" valid={nCols > 0} onSubmit={() => void ctl.w56.applyOps(t("msg.undo.textToColumns", "Text to columns"), [{ type: "textToColumns", sheet: ctl.sheet, range: { ...d.range }, options: toCsvOptions(st) }])}>
      <p className={dlg.lead}>{t("dd.splitsLead", "Splits the text of {range} into the columns to the right.", { range: a1(d.range) })}</p>
      <div className={styles.section}>{t("textToColumns.separator", "Separator options")}</div>
      <Segmented label={t("dd.mode", "Mode")} value={st.fixedWidth ? "fixed" : "separated"} onChange={(v) => setSt({ ...st, fixedWidth: v === "fixed" })} options={[{ id: "separated", label: t("dd.separatedBy", "Separated by") }, { id: "fixed", label: t("textToColumns.fixedWidth", "Fixed width") }]} />
      {st.fixedWidth ? (
        <Field label={t("dd.columnStarts", "Column starts (characters, comma separated)")} className={dlg.gap}>
          <input className={dlg.input} value={st.widths.join(", ")} placeholder="5, 12, 20" onChange={(e) => setSt({ ...st, widths: e.target.value.split(/[,\s]+/).map(Number).filter((n) => Number.isInteger(n) && n > 0) })} data-testid="t2c-widths" />
        </Field>
      ) : (
        <div className={dlg.optionGrid}>
          <Check label={t("textToColumns.tab", "Tab")} checked={st.tab} onChange={(v) => setSt({ ...st, tab: v })} testId="t2c-tab" />
          <Check label={t("textToColumns.comma", "Comma")} checked={st.comma} onChange={(v) => setSt({ ...st, comma: v })} testId="t2c-comma" />
          <Check label={t("textToColumns.semicolon", "Semicolon")} checked={st.semicolon} onChange={(v) => setSt({ ...st, semicolon: v })} testId="t2c-semicolon" />
          <Check label={t("textToColumns.space", "Space")} checked={st.space} onChange={(v) => setSt({ ...st, space: v })} testId="t2c-space" />
          <label className={dlg.checkRow}>
            <input type="checkbox" checked={st.other} onChange={(e) => setSt({ ...st, other: e.target.checked })} />
            <span>{t("textToColumns.other", "Other")}</span>
            <input className={dlg.input} style={{ width: 44, height: 26 }} maxLength={1} value={st.otherChar} onChange={(e) => setSt({ ...st, otherChar: e.target.value, other: e.target.value !== "" })} data-testid="t2c-other" />
          </label>
          <Check label={t("dd.mergeDelimiters", "Merge delimiters")} checked={st.mergeDelimiters} onChange={(v) => setSt({ ...st, mergeDelimiters: v })} />
        </div>
      )}
      <div className={styles.row}>
        <span className={styles.rowLabel}>{t("textToColumns.textDelimiter", "Text delimiter")}</span>
        <select className={dlg.selectSmall} aria-label={t("textToColumns.textDelimiter", "Text delimiter")} value={st.quote} onChange={(e) => setSt({ ...st, quote: e.target.value as '"' | "'" })}>
          <option value='"'>"</option>
          <option value="'">'</option>
        </select>
        <Check label={t("dd.trimSpaces", "Trim spaces")} checked={st.trim} onChange={(v) => setSt({ ...st, trim: v })} />
        <Check label={t("dd.detectNumbers", "Detect numbers and dates")} checked={st.detectNumbers} onChange={(v) => setSt({ ...st, detectNumbers: v })} />
      </div>
      <div className={styles.section}>{t("textToColumns.preview", "Preview")}</div>
      <div className={styles.previewWrap}>
        <table className={styles.previewTable} data-testid="t2c-preview">
          <thead>
            <tr>
              {Array.from({ length: nCols }, (_, c) => (
                <th key={c}>
                  <select className={dlg.selectSmall} style={{ height: 22, fontSize: 11 }} aria-label={t("dd.columnNType", "Column {n} type", { n: c + 1 })} value={typeOf(c)} onChange={(e) => setType(c, e.target.value as SeparatorState["columnTypes"][number])}>
                    {COLUMN_TYPES.map((ct) => (
                      <option key={ct.type} value={ct.type}>
                        {ct.label}
                      </option>
                    ))}
                  </select>
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {preview.map((row, r) => (
              <tr key={r}>
                {row.map((cell, c) => (
                  <td key={c}>{cell}</td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </W56Shell>
  );
}

// ---------------------------------------------------------------------------
// Remove duplicates, create names, database range
// ---------------------------------------------------------------------------

export function RemoveDuplicatesDialog({ d }: { d: D<"removeDuplicates"> }) {
  useW56Store((s) => s.i18nRev);
  const ctl = useController();
  const all = d.labels.map((_, i) => d.range.c0 + i);
  const [cols, setCols] = useState<number[]>(all);
  const [hasHeader, setHasHeader] = useState(d.hasHeader);
  return (
    <W56Shell title={t("duplicates.title", "Remove Duplicates")} ok={t("dialog.remove", "Remove")} size="wide" testId="remove-duplicates" valid={cols.length > 0} danger onSubmit={() => void ctl.w56.applyOps(t("msg.undo.removeDuplicates", "Remove duplicates"), [removeDuplicatesOp(ctl.sheet, d.range, cols.length === all.length ? [] : cols, hasHeader)])}>
      <p className={dlg.lead}>{t("dd.dupLead", "Keeps the first of each set of identical rows in {range} (compared on the chosen columns).", { range: a1(d.range) })}</p>
      <div className={styles.section}>{t("duplicates.columns", "Compare columns")}</div>
      <div className={styles.chips}>
        {d.labels.map((label, i) => {
          const c = d.range.c0 + i;
          const on = cols.includes(c);
          return (
            <button key={c} type="button" className={on ? styles.chipOn : styles.chip} aria-pressed={on} onClick={() => setCols(on ? cols.filter((x) => x !== c) : [...cols, c])} data-testid={`dup-col-${c}`}>
              {label}
            </button>
          );
        })}
      </div>
      <Check label={t("sort.hasHeader", "Range contains column labels")} checked={hasHeader} onChange={setHasHeader} />
    </W56Shell>
  );
}

export function CreateNamesDialog({ d }: { d: D<"createNamesFromLabels"> }) {
  useW56Store((s) => s.i18nRev);
  const ctl = useController();
  const [from, setFrom] = useState<NameLabelFlags>({ top: true, left: false, bottom: false, right: false });
  const any = from.top || from.left || from.bottom || from.right;
  return (
    <W56Shell title={t("cmd.data.createNamesFromLabels", "Create names from labels").replace(/…$/, "")} ok={t("dialog.ok", "OK")} testId="create-names" valid={any} onSubmit={() => void ctl.w56.applyOps(t("msg.undo.createNames", "Create names"), [createNamesOp(ctl.sheet, d.range, from)]).then(() => ctl.refreshNames())}>
      <p className={dlg.lead}>{t("dd.namesLead", "Names the rows and columns of {range} after the labels in its", { range: a1(d.range) })}</p>
      <div className={dlg.optionGrid}>
        <Check label={t("dd.topRow", "Top row")} checked={from.top} onChange={(v) => setFrom({ ...from, top: v })} testId="names-top" />
        <Check label={t("dd.leftColumn", "Left column")} checked={from.left} onChange={(v) => setFrom({ ...from, left: v })} testId="names-left" />
        <Check label={t("dd.bottomRow", "Bottom row")} checked={from.bottom} onChange={(v) => setFrom({ ...from, bottom: v })} />
        <Check label={t("dd.rightColumn", "Right column")} checked={from.right} onChange={(v) => setFrom({ ...from, right: v })} />
      </div>
    </W56Shell>
  );
}

export function DefineDatabaseRangeDialog({ d }: { d: D<"defineDatabaseRange"> }) {
  useW56Store((s) => s.i18nRev);
  const ctl = useController();
  const [name, setName] = useState("");
  const [range, setRange] = useState(a1(d.range));
  const [hasHeader, setHasHeader] = useState(true);
  const parsed = parseRangeText(range);
  const valid = isValidDbRangeName(name.trim()) && parsed !== null;
  return (
    <W56Shell title={t("cmd.data.defineDatabaseRange", "Define database range").replace(/…$/, "")} ok={t("dialog.add", "Add")} size="wide" testId="define-db-range" valid={valid} onSubmit={() => {
        if (parsed) void ctl.w56.applyOps(t("msg.undo.defineDatabaseRange", "Define database range"), [defineDatabaseRangeOp(name.trim(), ctl.sheet, parsed, hasHeader)]);
      }}>
      <Field label={t("dialog.name", "Name")}>
        <input className={dlg.input} value={name} autoFocus onChange={(e) => setName(e.target.value)} data-testid="dbrange-name" />
      </Field>
      <RangeField label={t("dialog.range", "Range")} value={range} onChange={setRange} testId="dbrange-range" />
      <Check label={t("sort.hasHeader", "Range contains column labels")} checked={hasHeader} onChange={setHasHeader} />
      {d.existing.length > 0 ? (
        <>
          <div className={styles.section}>{t("dd.existingRanges", "Existing ranges")}</div>
          <div className={styles.list}>
            {d.existing.map((r) => (
              <div key={r.name} className={styles.listItem}>
                <span className={styles.grow}>
                  {r.name} <span className={styles.muted}>{a1(r.range)}</span>
                </span>
                <button type="button" className={cx(dlg.rowBtn, dlg.dangerText)} onClick={() => void ctl.w56.deleteDatabaseRange(r.name).then(() => closeW56())}>
                  {t("dialog.delete", "Delete")}
                </button>
              </div>
            ))}
          </div>
        </>
      ) : null}
      {name.trim() !== "" && !isValidDbRangeName(name.trim()) ? <p className={dlg.error}>{t("dd.invalidName", "Invalid name.")}</p> : null}
    </W56Shell>
  );
}

// ---------------------------------------------------------------------------
// Fill series, sort options
// ---------------------------------------------------------------------------

export function FillSeriesDialog({ d }: { d: D<"fillSeries"> }) {
  useW56Store((s) => s.i18nRev);
  const ctl = useController();
  const first = useDocStore.getState().source?.getCell(d.range.r0, d.range.c0)?.text ?? "";
  const [st, setSt] = useState<FillSeriesState>({ ...DEFAULT_FILL_SERIES, direction: d.range.r1 > d.range.r0 ? "down" : "right", start: first });
  const n = st.direction === "down" || st.direction === "up" ? d.range.r1 - d.range.r0 + 1 : d.range.c1 - d.range.c0 + 1;
  const preview = st.kind === "autoFill" ? [] : previewSeries(st, n, first);
  return (
    <W56Shell title={t("cmd.edit.fillSeries", "Fill series").replace(/…$/, "")} ok={t("dialog.ok", "OK")} size="wide" testId="fill-series" onSubmit={() => void ctl.w56.applyOps(t("msg.undo.fillSeries", "Fill series"), [fillSeriesOp(ctl.sheet, d.range, st)])}>
      <div className={styles.grid2}>
        <Field label={t("sort.direction", "Direction")}>
          <select className={dlg.select} value={st.direction} onChange={(e) => setSt({ ...st, direction: e.target.value as FillSeriesState["direction"] })} data-testid="series-direction">
            <option value="down">{t("dd.dirDown", "Down")}</option>
            <option value="right">{t("dd.dirRight", "Right")}</option>
            <option value="up">{t("dd.dirUp", "Up")}</option>
            <option value="left">{t("dd.dirLeft", "Left")}</option>
          </select>
        </Field>
        <Field label={t("dd.seriesType", "Series type")}>
          <select className={dlg.select} value={st.kind} onChange={(e) => setSt({ ...st, kind: e.target.value as FillSeriesState["kind"] })} data-testid="series-kind">
            <option value="linear">{t("dd.serLinear", "Linear")}</option>
            <option value="growth">{t("dd.serGrowth", "Growth")}</option>
            <option value="date">{t("fmt.date", "Date")}</option>
            <option value="autoFill">{t("dd.serAutoFill", "AutoFill")}</option>
          </select>
        </Field>
        {st.kind === "date" ? (
          <Field label={t("dd.timeUnit", "Time unit")}>
            <select className={dlg.select} value={st.unit} onChange={(e) => setSt({ ...st, unit: e.target.value as FillSeriesState["unit"] })}>
              <option value="day">{t("dd.unitDay", "Day")}</option>
              <option value="weekday">{t("dd.unitWeekday", "Weekday")}</option>
              <option value="month">{t("dd.unitMonth", "Month")}</option>
              <option value="year">{t("dd.unitYear", "Year")}</option>
            </select>
          </Field>
        ) : (
          <span />
        )}
        <Field label={t("dd.startValue", "Start value")}>
          <input className={dlg.input} value={st.start} onChange={(e) => setSt({ ...st, start: e.target.value })} data-testid="series-start" />
        </Field>
        <Field label={t("dd.endValueOptional", "End value (optional)")}>
          <input className={dlg.input} value={st.end} onChange={(e) => setSt({ ...st, end: e.target.value })} data-testid="series-end" />
        </Field>
        <Field label={t("dd.increment", "Increment")}>
          <input className={dlg.input} value={st.increment} disabled={st.kind === "autoFill"} onChange={(e) => setSt({ ...st, increment: e.target.value })} data-testid="series-increment" />
        </Field>
      </div>
      {preview.length > 0 ? (
        <p className={dlg.note} data-testid="series-preview">
          {a1(d.range)}: {preview.join(", ")}
          {preview.length < n ? ", …" : ""}
        </p>
      ) : null}
    </W56Shell>
  );
}

export function SortOptionsDialog({ d }: { d: D<"sortOptions"> }) {
  useW56Store((s) => s.i18nRev);
  const ctl = useController();
  const [st, setSt] = useState<SortOptionsState>({ keys: [{ col: Math.min(d.range.c1, Math.max(d.range.c0, ctl.selection.active.c)), ascending: true }], hasHeader: d.hasHeader, caseSensitive: false, leftToRight: false, copyTo: "", customList: null, includeFormats: true, natural: false });
  const lists = [...BUILTIN_SORT_LISTS, ...d.lists];
  const keys = st.leftToRight ? Array.from({ length: d.range.r1 - d.range.r0 + 1 }, (_, i) => ({ col: d.range.r0 + i, label: t("a11y.rowHeader", "Row {n}", { n: d.range.r0 + i + 1 }) })) : d.labels.map((label, i) => ({ col: d.range.c0 + i, label: st.hasHeader ? label : t("a11y.columnHeader", "Column {n}", { n: colLetter(d.range.c0 + i) }) }));
  const op = sortWithOptionsOp(ctl.sheet, d.range, st, lists, (s) => ctl.w56.parseCellRef(s));
  const key0 = st.keys[0];
  return (
    <W56Shell title={`${t("sort.title", "Sort")} (${t("dialog.options", "Options")})`} ok={t("sort.title", "Sort")} size="wide" testId="sort-options" valid={typeof op !== "string"} onSubmit={() => {
        if (typeof op !== "string") void ctl.w56.applyOps(t("sort.title", "Sort"), [op]);
      }}>
      <Field label={st.leftToRight ? t("dd.sortByRow", "Sort by row") : t("sort.key", "Sort key")}>
        <select className={dlg.select} value={String(key0?.col ?? "")} onChange={(e) => setSt({ ...st, keys: [{ col: Number(e.target.value), ascending: key0?.ascending ?? true }] })} data-testid="sortopt-key">
          {keys.map((k) => (
            <option key={k.col} value={String(k.col)}>
              {k.label}
            </option>
          ))}
        </select>
      </Field>
      <div className={styles.row}>
        <Segmented label={t("dd.order", "Order")} value={key0?.ascending ? "asc" : "desc"} onChange={(v) => setSt({ ...st, keys: [{ col: key0?.col ?? d.range.c0, ascending: v === "asc" }] })} options={[{ id: "asc", label: t("sort.ascending", "Ascending") }, { id: "desc", label: t("sort.descending", "Descending") }]} />
      </div>
      <div className={styles.section}>{t("sort.direction", "Direction")}</div>
      <Segmented label={t("sort.direction", "Direction")} value={st.leftToRight ? "ltr" : "ttb"} onChange={(v) => setSt({ ...st, leftToRight: v === "ltr", keys: [{ col: v === "ltr" ? d.range.r0 : d.range.c0, ascending: true }] })} options={[{ id: "ttb", label: t("sort.topToBottom", "Top to bottom (sort rows)") }, { id: "ltr", label: t("sort.leftToRight", "Left to right (sort columns)") }]} />
      <div className={styles.section}>{t("dialog.options", "Options")}</div>
      <div className={dlg.optionGrid}>
        <Check label={t("sort.hasHeader", "Range contains column labels")} checked={st.hasHeader} onChange={(v) => setSt({ ...st, hasHeader: v })} />
        <Check label={t("sort.caseSensitive", "Case sensitive")} checked={st.caseSensitive} onChange={(v) => setSt({ ...st, caseSensitive: v })} />
        <Check label={t("dd.includeFormats", "Include formats")} checked={st.includeFormats} onChange={(v) => setSt({ ...st, includeFormats: v })} />
        <Check label={t("sort.naturalSort", "Natural sort")} checked={st.natural} onChange={(v) => setSt({ ...st, natural: v })} />
      </div>
      <Field label={t("dd.customSortOrder", "Custom sort order (first key)")} className={dlg.gap}>
        <select className={dlg.select} value={st.customList === null ? "" : String(st.customList)} onChange={(e) => setSt({ ...st, customList: e.target.value === "" ? null : Number(e.target.value) })} data-testid="sortopt-list">
          <option value="">{t("dd.none", "- none -")}</option>
          {lists.map((l, i) => (
            <option key={i} value={String(i)}>
              {l.slice(0, 4).join(", ")}
              {l.length > 4 ? ", …" : ""}
            </option>
          ))}
        </select>
      </Field>
      <RangeField label={t("dd.copySortOptional", "Copy sort results to (optional)")} value={st.copyTo} onChange={(v) => setSt({ ...st, copyTo: v })} testId="sortopt-copy-to" allowSheet placeholder={t("dd.sheetCellRef", "Sheet2.A1")} />
      {typeof op === "string" ? <p className={dlg.error}>{op}</p> : null}
    </W56Shell>
  );
}

// ---------------------------------------------------------------------------
// Delete contents, multiple operations, insert / delete cells
// ---------------------------------------------------------------------------

export function DeleteContentsDialog({ d }: { d: D<"deleteContents"> }) {
  useW56Store((s) => s.i18nRev);
  const ctl = useController();
  const [what, setWhat] = useState<DeleteContentsFlags>(DEFAULT_DELETE);
  const [all, setAll] = useState(false);
  const flags = all ? { text: true, numbers: true, dates: true, formulas: true, notes: true, formats: true, objects: true } : what;
  const any = Object.values(flags).some(Boolean);
  return (
    <W56Shell title={t("cmd.edit.deleteContents", "Delete contents").replace(/…$/, "")} ok={t("dialog.ok", "OK")} testId="delete-contents" valid={any} onSubmit={() => void ctl.w56.deleteContents(d.ranges, flags)}>
      <p className={dlg.lead}>{t("dd.removesFrom", "Removes from {ranges}:", { ranges: d.ranges.map(a1).join(", ") })}</p>
      <Check label={t("dd.deleteAll", "Delete all")} checked={all} onChange={setAll} testId="delete-all" />
      <div className={dlg.optionGrid}>
        {DELETE_FLAG_LABELS.map((f) => (
          <Check key={f.key} label={t(`paste.${f.key}`, f.label)} checked={flags[f.key]} disabled={all} onChange={(v) => setWhat({ ...what, [f.key]: v })} testId={`delete-${f.key}`} />
        ))}
      </div>
    </W56Shell>
  );
}

export function MultipleOperationsDialog({ d }: { d: D<"multipleOperations"> }) {
  useW56Store((s) => s.i18nRev);
  const ctl = useController();
  const [st, setSt] = useState<MultiOpsState>({ formulas: "", rowInput: "", colInput: "" });
  const parseCell = (s: string) => {
    const r = ctl.w56.parseCellRef(s);
    return r ? { r: r.r, c: r.c } : null;
  };
  const ops = multipleOperationsOps(ctl.sheet, d.range, st, parseCell);
  return (
    <W56Shell title={t("multiOps.title", "Multiple Operations")} ok={t("dialog.ok", "OK")} size="wide" testId="multiple-operations" valid={typeof ops !== "string"} onSubmit={() => {
        if (typeof ops !== "string") void ctl.w56.applyOps(t("msg.undo.multipleOperations", "Multiple operations"), ops);
      }}>
      <p className={dlg.lead}>{t("dd.multiLead", "Fills {range} with MULTIPLE.OPERATIONS formulas: the first column (and/or row) holds the values to try.", { range: a1(d.range) })}</p>
      <RangeField label={t("multiOps.formulas", "Formulas")} value={st.formulas} onChange={(v) => setSt({ ...st, formulas: v })} testId="mops-formulas" placeholder="B4" />
      <RangeField label={t("multiOps.rowInputCell", "Row input cell")} value={st.rowInput} onChange={(v) => setSt({ ...st, rowInput: v })} testId="mops-row" placeholder="B1" />
      <RangeField label={t("multiOps.columnInputCell", "Column input cell")} value={st.colInput} onChange={(v) => setSt({ ...st, colInput: v })} testId="mops-col" placeholder="B2" />
      {typeof ops === "string" && (st.formulas || st.rowInput || st.colInput) ? <p className={dlg.error}>{ops}</p> : null}
    </W56Shell>
  );
}

export function InsertCellsDialog({ d }: { d: D<"insertCells"> }) {
  useW56Store((s) => s.i18nRev);
  const ctl = useController();
  const [mode, setMode] = useState<"down" | "right" | "rows" | "cols">("down");
  return (
    <W56Shell
      title={t("cmd.sheet.insertCells", "Insert cells").replace(/…$/, "")}
      ok={t("dialog.insert", "Insert")}
      testId="insert-cells"
      onSubmit={() => {
        if (mode === "rows") void ctl.insertRows("before");
        else if (mode === "cols") void ctl.insertCols("before");
        else void ctl.w56.insertCells(d.range, mode);
      }}
    >
      <p className={dlg.lead}>{t("dd.insertAt", "Insert at {range}:", { range: a1(d.range) })}</p>
      <div role="radiogroup" aria-label={t("dd.selection", "Selection")}>
        {([
          ["down", t("dd.shiftDown", "Shift cells down")],
          ["right", t("dd.shiftRight", "Shift cells right")],
          ["rows", t("dd.entireRow", "Entire row")],
          ["cols", t("dd.entireColumn", "Entire column")],
        ] as const).map(([id, label]) => (
          <label key={id} className={dlg.checkRow}>
            <input type="radio" name="insert-mode" checked={mode === id} onChange={() => setMode(id)} data-testid={`insert-${id}`} />
            <span>{label}</span>
          </label>
        ))}
      </div>
    </W56Shell>
  );
}

export function DeleteCellsDialog({ d }: { d: D<"deleteCells"> }) {
  useW56Store((s) => s.i18nRev);
  const ctl = useController();
  const [mode, setMode] = useState<"up" | "left" | "rows" | "cols">("up");
  return (
    <W56Shell
      title={t("cmd.sheet.deleteCells", "Delete cells").replace(/…$/, "")}
      ok={t("dialog.delete", "Delete")}
      danger
      testId="delete-cells"
      onSubmit={() => {
        if (mode === "rows") void ctl.deleteRows();
        else if (mode === "cols") void ctl.deleteCols();
        else void ctl.w56.deleteCells(d.range, mode);
      }}
    >
      <p className={dlg.lead}>{t("dd.deleteRangeLead", "Delete {range}:", { range: a1(d.range) })}</p>
      <div role="radiogroup" aria-label={t("dd.selection", "Selection")}>
        {([
          ["up", t("dd.shiftUp", "Shift cells up")],
          ["left", t("dd.shiftLeft", "Shift cells left")],
          ["rows", t("dd.deleteRows", "Delete entire row(s)")],
          ["cols", t("dd.deleteCols", "Delete entire column(s)")],
        ] as const).map(([id, label]) => (
          <label key={id} className={dlg.checkRow}>
            <input type="radio" name="delete-mode" checked={mode === id} onChange={() => setMode(id)} data-testid={`deletecells-${id}`} />
            <span>{label}</span>
          </label>
        ))}
      </div>
    </W56Shell>
  );
}

// ---------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------

export function ScenariosDialog({ d }: { d: D<"scenarios"> }) {
  useW56Store((s) => s.i18nRev);
  const ctl = useController();
  const [list, setList] = useState<ScenarioDto[]>(d.list);
  const [name, setName] = useState(t("dd.scenarioN", "Scenario {n}", { n: d.list.length + 1 }));
  const [comment, setComment] = useState("");
  const [color, setColor] = useState<number>(SCENARIO_COLORS[0]?.rgba ?? 0xbfdbfeff);
  const [copyBack, setCopyBack] = useState(true);
  const [displayBorder, setDisplayBorder] = useState(true);
  const [preventChanges, setPrevent] = useState(false);
  const ranges = ctl.ranges;
  const create = async () => {
    const s: ScenarioDto = { id: 0, name: name.trim(), comment, ranges: ranges.map((r) => ({ ...r })), borderColor: color, displayBorder, copyBack, copyStyles: false, preventChanges, active: false };
    if (await ctl.w56.createScenario(s)) {
      closeW56();
      ctl.focusGrid();
    }
  };
  return (
    <W56Shell title={t("scenario.title", "Create Scenario")} ok={t("dialog.add", "Add")} size="wide" testId="scenarios-dialog" valid={name.trim() !== ""} keepOpen onSubmit={create}>
      <p className={dlg.lead}>{t("dd.scenarioLead", "A scenario stores the values of {ranges} under a name; show one from the Navigator or here.", { ranges: ranges.map(a1).join(", ") })}</p>
      {list.length > 0 ? (
        <div className={styles.list} data-testid="scenario-list">
          {list.map((s) => (
            <div key={s.id} className={s.active ? styles.listItemOn : styles.listItem}>
              <span className={styles.grow} title={s.comment}>
                {s.name}
              </span>
              <button type="button" className={dlg.rowBtn} onClick={() => void ctl.w56.showScenario(s.id).then(() => setList(list.map((x) => ({ ...x, active: x.id === s.id }))))}>
                {t("dd.show", "Show")}
              </button>
              <button type="button" className={cx(dlg.rowBtn, dlg.dangerText)} onClick={() => void ctl.w56.deleteScenario(s.id).then(() => setList(list.filter((x) => x.id !== s.id)))}>
                {t("dialog.delete", "Delete")}
              </button>
            </div>
          ))}
        </div>
      ) : null}
      <Field label={t("scenario.name", "Name of scenario")} className={dlg.gap}>
        <input className={dlg.input} value={name} autoFocus onChange={(e) => setName(e.target.value)} data-testid="scenario-name" />
      </Field>
      <Field label={t("scenario.comment", "Comment")}>
        <textarea className={styles.textarea} value={comment} onChange={(e) => setComment(e.target.value)} />
      </Field>
      <div className={styles.section}>{t("dd.settings", "Settings")}</div>
      <div className={dlg.optionGrid}>
        <Check label={t("dd.displayBorder", "Display border")} checked={displayBorder} onChange={setDisplayBorder} />
        <Check label={t("scenario.copyBack", "Copy back")} checked={copyBack} onChange={setCopyBack} />
        <Check label={t("dd.preventChanges", "Prevent changes")} checked={preventChanges} onChange={setPrevent} />
      </div>
      <div className={styles.swatches} role="radiogroup" aria-label={t("dd.borderColour", "Border colour")}>
        {SCENARIO_COLORS.map((c) => (
          <button key={c.rgba} type="button" role="radio" aria-checked={color === c.rgba} aria-label={c.label} title={c.label} className={color === c.rgba ? styles.swatchOn : styles.swatch} style={{ background: rgbaToHex(c.rgba).slice(0, 7) }} onClick={() => setColor(c.rgba)} />
        ))}
      </div>
    </W56Shell>
  );
}

// ---------------------------------------------------------------------------
// Sheets: copy, show, tab colour; outline level
// ---------------------------------------------------------------------------

export function CopySheetDialog({ d }: { d: D<"copySheet"> }) {
  useW56Store((s) => s.i18nRev);
  const ctl = useController();
  const sheets = useDocStore((s) => s.info?.sheets ?? []);
  const [name, setName] = useState(d.name);
  const [index, setIndex] = useState<string>("after");
  const taken = sheets.some((s) => s.name.toLowerCase() === name.trim().toLowerCase());
  return (
    <W56Shell title={t("cmd.sheet.duplicate", "Copy sheet").replace(/…$/, "")} ok={t("dialog.ok", "OK")} testId="copy-sheet" valid={name.trim() !== "" && !taken} onSubmit={() => void ctl.w56.copySheet(d.sheet, name, index === "after" ? null : Number(index))}>
      <Field label={t("dd.newName", "New name")}>
        <input className={dlg.input} value={name} autoFocus onChange={(e) => setName(e.target.value)} data-testid="copy-sheet-name" />
      </Field>
      <Field label={t("dd.insertBefore", "Insert before")} className={dlg.gap}>
        <select className={dlg.select} value={index} onChange={(e) => setIndex(e.target.value)}>
          <option value="after">{t("dd.afterCurrentSheet", "After the current sheet")}</option>
          {sheets.map((s, i) => (
            <option key={s.id} value={String(i)}>
              {s.name}
            </option>
          ))}
          <option value={String(sheets.length)}>{t("dd.moveToEnd", "- move to end position -")}</option>
        </select>
      </Field>
      {taken ? <p className={dlg.error}>{t("sheet.nameExists", "A sheet with this name already exists.")}</p> : null}
    </W56Shell>
  );
}

export function ShowSheetDialog({ d }: { d: D<"showSheet"> }) {
  useW56Store((s) => s.i18nRev);
  const ctl = useController();
  const [picked, setPicked] = useState<number[]>(d.hidden[0] ? [d.hidden[0].id] : []);
  return (
    <W56Shell title={t("cmd.sheet.show", "Show sheet").replace(/…$/, "")} ok={t("dialog.ok", "OK")} testId="show-sheet" valid={picked.length > 0} onSubmit={() => void ctl.w56.showSheets(picked)}>
      <div className={styles.list} role="listbox" aria-multiselectable="true" aria-label={t("dd.hiddenSheets", "Hidden sheets")}>
        {d.hidden.map((s) => {
          const on = picked.includes(s.id);
          return (
            <button key={s.id} type="button" role="option" aria-selected={on} className={on ? styles.listItemOn : styles.listItem} onClick={() => setPicked(on ? picked.filter((x) => x !== s.id) : [...picked, s.id])} data-testid={`show-sheet-${s.id}`}>
              {s.name}
            </button>
          );
        })}
      </div>
    </W56Shell>
  );
}

export function TabColorDialog({ d }: { d: D<"tabColor"> }) {
  useW56Store((s) => s.i18nRev);
  const ctl = useController();
  const [color, setColor] = useState<number | null>(d.color);
  return (
    <W56Shell title={t("cmd.sheet.tabColor", "Tab colour").replace(/…$/, "")} ok={t("dialog.ok", "OK")} testId="tab-color" onSubmit={() => void ctl.w56.setTabColor(d.sheet, color)}>
      <div className={styles.swatches} role="radiogroup" aria-label={t("dd.tabColour", "Tab colour")}>
        <button type="button" role="radio" aria-checked={color === null} aria-label={t("dd.noColour", "No colour")} title={t("dd.noColour", "No colour")} className={color === null ? styles.swatchOn : styles.swatch} style={{ background: "transparent", border: color === null ? undefined : "1px dashed var(--tally-border)" }} onClick={() => setColor(null)} data-testid="tab-color-none">
          ✕
        </button>
        {TAB_COLORS.map((c) => (
          <button key={c.rgba} type="button" role="radio" aria-checked={color === c.rgba} aria-label={c.label} title={c.label} className={color === c.rgba ? styles.swatchOn : styles.swatch} style={{ background: rgbaToHex(c.rgba).slice(0, 7) }} onClick={() => setColor(c.rgba)} data-testid={`tab-color-${c.id}`} />
        ))}
      </div>
    </W56Shell>
  );
}

export function ShowOutlineLevelDialog({ d }: { d: D<"showOutlineLevel"> }) {
  useW56Store((s) => s.i18nRev);
  const ctl = useController();
  const [axis, setAxis] = useState(d.axis);
  const [level, setLevel] = useState(1);
  return (
    <W56Shell title={t("cmd.data.showOutlineLevel", "Show outline level").replace(/…$/, "")} ok={t("dialog.ok", "OK")} testId="outline-level" onSubmit={() => ctl.w56.onOutlineLevel(axis, level)}>
      <Segmented label={t("dd.axis", "Axis")} value={axis} onChange={setAxis} options={[{ id: "rows", label: t("menu.format.rows", "Rows") }, { id: "cols", label: t("menu.format.columns", "Columns") }]} />
      <Field label={t("outline.level", "Level {n}", { n: level })} className={dlg.gap}>
        <input className={dlg.input} type="number" min={1} max={d.max + 1} value={level} onChange={(e) => setLevel(Math.max(1, Math.min(d.max + 1, Number(e.target.value) || 1)))} data-testid="outline-level-input" />
      </Field>
      <p className={dlg.note}>{t("dd.levelNote", "Level {n} shows everything; level 1 only the outermost groups.", { n: d.max + 1 })}</p>
    </W56Shell>
  );
}

// ---------------------------------------------------------------------------
// Solver
// ---------------------------------------------------------------------------

interface ConstraintRow {
  lhs: string;
  op: SolverOp;
  rhs: string;
}

export function SolverDialog({ d }: { d: D<"solver"> }) {
  useW56Store((s) => s.i18nRev);
  const ctl = useController();
  const [objective, setObjective] = useState(d.objective);
  const [goal, setGoal] = useState<"max" | "min" | "value">("max");
  const [value, setValue] = useState("0");
  const [variables, setVariables] = useState("");
  const [rows, setRows] = useState<ConstraintRow[]>([{ lhs: "", op: "le", rhs: "" }]);
  const [nonNeg, setNonNeg] = useState(true);
  const [integer, setInteger] = useState(false);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<SolverResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const obj = ctl.w56.parseCellRef(objective);
  const vars = variables.split(/[,;]/).map((s) => s.trim()).filter((s) => s !== "").map((s) => {
    const r = parseRangeText(s);
    return r ? { sheet: ctl.sheet, range: r } : null;
  });
  const constraints: SolverConstraint[] = [];
  for (const r of rows) {
    if (r.lhs.trim() === "") continue;
    const lhs = parseRangeText(r.lhs);
    if (lhs) constraints.push({ lhs: { sheet: ctl.sheet, range: lhs }, op: r.op, rhs: r.rhs.trim() });
  }
  const valid = obj !== null && vars.length > 0 && vars.every((v) => v !== null) && (goal !== "value" || Number.isFinite(Number(value)));
  const solve = () => {
    if (!obj) return;
    setBusy(true);
    setError(null);
    ctl.w56
      .solve({
        objective: obj,
        goal: goal === "max" ? { type: "maximize" } : goal === "min" ? { type: "minimize" } : { type: "value", value: Number(value) },
        variables: vars.filter((v): v is NonNullable<typeof v> => v !== null),
        constraints,
        options: { assumeNonNegative: nonNeg, integer, epsilon: 1e-6, maxIterations: 10_000, timeoutMs: 10_000 },
      })
      .then(
        (res) => {
          setBusy(false);
          setResult(res);
        },
        (e: unknown) => {
          setBusy(false);
          setError(typeof e === "object" && e && "message" in e ? String((e as { message: unknown }).message) : t("dd.solverFailedMsg", "The solver failed."));
        },
      );
  };
  const setRow = (i: number, p: Partial<ConstraintRow>) => setRows((rs) => rs.map((r, j) => (j === i ? { ...r, ...p } : r)));
  return (
    <W56Shell
      title={t("solver.title", "Solver")}
      ok={result && result.status === "optimal" ? t("dialog.apply", "Apply") : busy ? t("dd.solving", "Solving…") : t("solver.solve", "Solve")}
      size="xwide"
      testId="solver-dialog"
      keepOpen={!(result && result.status === "optimal")}
      valid={valid && !busy}
      onSubmit={() => {
        if (result && result.status === "optimal") void ctl.w56.applySolverResult(result);
        else solve();
      }}
    >
      <div className={styles.grid3}>
        <RangeField label={t("solver.targetCell", "Target cell")} value={objective} onChange={(v) => (setObjective(v), setResult(null))} testId="solver-objective" />
        <Field label={t("solver.optimize", "Optimize result to")}>
          <select className={dlg.select} value={goal} onChange={(e) => (setGoal(e.target.value as "max" | "min" | "value"), setResult(null))} data-testid="solver-goal">
            <option value="max">{t("solver.maximum", "Maximum")}</option>
            <option value="min">{t("solver.minimum", "Minimum")}</option>
            <option value="value">{t("solver.valueOf", "Value of")}</option>
          </select>
        </Field>
        <Field label={t("dialog.value", "Value")}>
          <input className={dlg.input} value={value} disabled={goal !== "value"} onChange={(e) => setValue(e.target.value)} />
        </Field>
      </div>
      <RangeField label={t("solver.byChanging", "By changing cells")} value={variables} onChange={(v) => (setVariables(v), setResult(null))} testId="solver-variables" placeholder="B2:B4, D2" />
      <div className={styles.section}>{t("solver.constraints", "Limiting conditions")}</div>
      {rows.map((r, i) => (
        <div key={i} className={styles.condRow} style={{ gridTemplateColumns: "1.4fr 0.8fr 1.4fr auto" }}>
          <input className={cx(dlg.input, dlg.mono)} style={{ height: 28 }} aria-label={t("dd.cellReferenceN", "Cell reference {n}", { n: i + 1 })} value={r.lhs} placeholder="B2:B4" onChange={(e) => setRow(i, { lhs: e.target.value })} data-testid={`solver-lhs-${i}`} />
          <select className={dlg.selectSmall} aria-label={t("dd.operatorN", "Operator {n}", { n: i + 1 })} value={r.op} onChange={(e) => setRow(i, { op: e.target.value as SolverOp })}>
            <option value="le">{"<="}</option>
            <option value="ge">{">="}</option>
            <option value="eq">=</option>
            <option value="integer">{t("dd.integer", "Integer")}</option>
            <option value="binary">{t("dd.binary", "Binary")}</option>
          </select>
          <input className={dlg.input} style={{ height: 28 }} aria-label={t("dd.valueN", "Value {n}", { n: i + 1 })} value={r.rhs} disabled={r.op === "integer" || r.op === "binary"} placeholder={t("dd.valueOrCell", "100 or C1")} onChange={(e) => setRow(i, { rhs: e.target.value })} data-testid={`solver-rhs-${i}`} />
          <button type="button" className={dlg.rowBtn} onClick={() => setRows(rows.length > 1 ? rows.filter((_, j) => j !== i) : [{ lhs: "", op: "le", rhs: "" }])}>
            ×
          </button>
        </div>
      ))}
      <button type="button" className={dlg.rowBtn} onClick={() => setRows([...rows, { lhs: "", op: "le", rhs: "" }])} data-testid="solver-add-constraint">
        {t("dd.addCondition", "+ Add condition")}
      </button>
      <div className={dlg.optionGrid}>
        <Check label={t("dd.assumeNonNegative", "Assume variables as non-negative")} checked={nonNeg} onChange={setNonNeg} />
        <Check label={t("dd.assumeInteger", "Assume variables as integer")} checked={integer} onChange={setInteger} />
      </div>
      {error ? <p className={dlg.error}>{error}</p> : null}
      {result ? (
        <div className={cx(styles.result, result.status !== "optimal" && styles.resultWarn)} data-testid="solver-result">
          <div className={styles.resultTitle}>{result.status === "optimal" ? t("solver.solved", "Solving successfully finished.") : result.status === "notLinear" ? t("solver.notLinear", "The model is not linear. The solver handles linear models only.") : `${t("solver.failed", "No solution was found.")} (${result.status})`}</div>
          <div>
            {solverSummary(String(Number(result.objective.toPrecision(10))), result.iterations)}
            {result.values.length > 0 ? ` ${result.values.map((v) => `${colLetter(v.cell.c)}${v.cell.r + 1} = ${Number(v.value.toPrecision(10))}`).join(", ")}` : ""}
          </div>
        </div>
      ) : null}
    </W56Shell>
  );
}

// ---------------------------------------------------------------------------
// Merge prompt (§51), document statistics, recovery, Save as Template, selection list
// ---------------------------------------------------------------------------

export function MergePromptDialog({ d }: { d: D<"mergePrompt"> }) {
  useW56Store((s) => s.i18nRev);
  return (
    <W56Shell
      title={t("cmd.format.merge", "Merge cells")}
      ok={t("dialog.yes", "Yes")}
      cancelLabel={t("dialog.no", "No")}
      testId="merge-prompt"
      onSubmit={d.onMove}
      extra={
        <button type="button" className={dlg.secondary} onClick={() => closeW56()} data-testid="merge-cancel">
          {t("dialog.cancel", "Cancel")}
        </button>
      }
      onClose={() => undefined}
    >
      <p className={dlg.lead}>
        {d.hidden === 1
          ? t("dd.mergeLeadOne", "Should the contents of the hidden cells be moved into the first cell? ({n} cell holds content in {range}; “No” keeps only the first cell's content.)", { n: d.hidden, range: a1(d.range) })
          : t("dd.mergeLeadMany", "Should the contents of the hidden cells be moved into the first cell? ({n} cells hold content in {range}; “No” keeps only the first cell's content.)", { n: d.hidden, range: a1(d.range) })}
      </p>
      <MergeNoButton onKeep={d.onKeep} />
    </W56Shell>
  );
}

/** "No" is the Cancel-styled button of the shell: wire it to keep-and-merge. */
function MergeNoButton({ onKeep }: { onKeep: () => void }) {
  useEffect(() => {
    const el = document.querySelector<HTMLButtonElement>('[data-testid="merge-prompt"] [data-testid="dialog-cancel"]');
    if (!el) return undefined;
    const handler = (e: MouseEvent) => {
      e.stopPropagation();
      e.preventDefault();
      closeW56();
      onKeep();
    };
    el.addEventListener("click", handler, true);
    return () => el.removeEventListener("click", handler, true);
  }, [onKeep]);
  return null;
}

export function DocumentStatsDialog({ d }: { d: D<"documentStats"> }) {
  useW56Store((s) => s.i18nRev);
  const rows: [string, string, string][] = [
    ["sheets", t("navigator.sheets", "Sheets"), String(d.stats.sheets)],
    ["cells-with-content", t("dd.statCells", "Cells with content"), String(d.stats.cells)],
    ["formula-cells", t("dd.statFormulas", "Formula cells"), String(d.stats.formulas)],
    ["pages", t("print.pages", "Pages"), d.stats.pages === null ? "—" : String(d.stats.pages)],
    ["charts", t("navigator.charts", "Charts"), String(d.stats.charts)],
    ["notes", t("navigator.notes", "Notes"), String(d.stats.notes)],
    ["defined-names", t("dd.statNames", "Defined names"), String(d.stats.names)],
  ];
  return (
    <W56Shell title={t("cmd.file.documentStats", "Document statistics…").replace(/…$/, "")} ok={t("dialog.close", "Close")} testId="document-stats" onSubmit={() => undefined} cancelLabel={t("dialog.close", "Close")} noOk>
      <div className={dlg.preview}>
        {rows.map(([id, k, v]) => (
          <div key={id} className={dlg.previewRow}>
            <span className={dlg.previewIn}>{k}</span>
            <span className={dlg.previewOut} data-testid={`stat-${id}`}>
              {v}
            </span>
          </div>
        ))}
      </div>
    </W56Shell>
  );
}

export function RecoveryDialog({ d }: { d: D<"recovery"> }) {
  useW56Store((s) => s.i18nRev);
  const ctl = useController();
  const [files, setFiles] = useState(d.files);
  useEffect(() => {
    if (files.length === 0) closeW56();
  }, [files]);
  return (
    <W56Shell title={t("dd.recoverTitle", "Recover unsaved work")} size="wide" testId="recovery-dialog" noOk cancelLabel={t("dd.later", "Later")} onSubmit={() => undefined}>
      <p className={dlg.lead}>{t("dd.recoveryLead", "Tally kept autosaved copies of these workbooks when it last closed. Recover opens the copy; Discard deletes it.")}</p>
      <div className={styles.list}>
        {files.map((f) => (
          <div key={f.path} className={styles.listItem}>
            <span className={styles.grow} title={f.path}>
              {f.title} <span className={styles.muted}>{f.savedAt}</span>
            </span>
            <button type="button" className={dlg.rowBtn} onClick={() => void ctl.w56.recover(f, files.filter((x) => x !== f)).then((ok) => {
              if (ok) setFiles(files.filter((x) => x !== f));
            })} data-testid="recover">
              {t("dd.recover", "Recover")}
            </button>
            <button type="button" className={cx(dlg.rowBtn, dlg.dangerText)} onClick={() => void ctl.w56.discardRecovery(f).then(() => setFiles(files.filter((x) => x !== f)))}>
              {t("dd.discard", "Discard")}
            </button>
          </div>
        ))}
      </div>
    </W56Shell>
  );
}

export function SaveAsTemplateDialog({ d }: { d: D<"saveAsTemplate"> }) {
  useW56Store((s) => s.i18nRev);
  const ctl = useController();
  const [name, setName] = useState(d.name);
  const valid = /^[^\\/:*?"<>|]{1,80}$/.test(name.trim());
  return (
    <W56Shell title={t("cmd.file.saveAsTemplate", "Save as template").replace(/…$/, "")} ok={t("dialog.save", "Save")} testId="save-as-template" valid={valid} onSubmit={() => void ctl.w56.saveAsTemplate(name.trim())}>
      <Field label={t("dd.templateName", "Template name")}>
        <input className={dlg.input} value={name} autoFocus onChange={(e) => setName(e.target.value)} data-testid="template-name" />
      </Field>
      <p className={dlg.note}>{t("dd.templateNote", "Saved as an OpenDocument template (.ots) in your templates folder; New from Template lists it.")}</p>
    </W56Shell>
  );
}

/** ⌃D: a small list anchored at the cell (not a modal). */
export function SelectionListPopover({ d }: { d: D<"selectionList"> }) {
  useW56Store((s) => s.i18nRev);
  const ctl = useController();
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    ref.current?.querySelector("button")?.focus();
    const down = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) closeW56();
    };
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        closeW56();
        ctl.focusGrid();
      }
    };
    window.addEventListener("mousedown", down, true);
    window.addEventListener("keydown", key, true);
    return () => {
      window.removeEventListener("mousedown", down, true);
      window.removeEventListener("keydown", key, true);
    };
  }, [ctl]);
  const top = Math.min(d.rect.bottom + 2, window.innerHeight - 230);
  return (
    <div ref={ref} className={styles.selectionList} style={{ left: d.rect.left, top }} role="listbox" aria-label={t("dd.selectionList", "Selection list")} data-testid="selection-list">
      {d.items.map((item) => (
        <button
          key={item}
          type="button"
          role="option"
          aria-selected={false}
          onClick={() => {
            closeW56();
            void ctl.w56.pickFromList(d.cell, item);
          }}
          onKeyDown={(e) => {
            const el = e.currentTarget;
            if (e.key === "ArrowDown") (el.nextElementSibling as HTMLElement | null)?.focus();
            if (e.key === "ArrowUp") (el.previousElementSibling as HTMLElement | null)?.focus();
          }}
        >
          {item}
        </button>
      ))}
    </div>
  );
}

/** Hook for the generic confirm used by a few flows (keeps the ui store import local). */
export function confirmW56(title: string, message: string, ok: string, onConfirm: () => void): void {
  useUiStore.getState().openDialog({ kind: "confirm", title, message, ok, onConfirm });
}

/** Re-exported for the chrome. */
export { useW56Store };
