/**
 * The Chart Data Table dialog (T18; AOO 4.1 View ▸ Data Table,
 * `chart2/source/controller/dialogs/dlg_DataEditor.cxx` and
 * `DataBrowser.cxx`): the own data of a chart that reads no cells, as a
 * grid with a series-name row and a role row above the data rows, and
 * AOO's toolbar (Insert Row, Insert Series, Delete Row, Delete Series,
 * Move Series Right, Move Row Down).
 *
 * As in AOO the dialog has no OK / Cancel: closing it (Close, Escape)
 * applies the edits as one "Edit chart data" undo step
 * (`ChartController::executeDispatch_EditData`). A value cell refuses
 * text that is not a number and stays in edit ("Numbers are required").
 *
 * Keyboard: the grid has one tab stop (the cursor cell); the arrow keys,
 * Home / End and Ctrl+Home / Ctrl+End move the cursor; Enter or F2 edits
 * the cell, typing replaces it; in a cell edit Enter commits and moves
 * down, Tab commits and moves right, Escape cancels; Delete clears.
 *
 * The table is read when the dialog opens (`chart_data_table`, which
 * refuses tables over a million cells) and only the rows in view (plus a
 * margin and the cursor row) are rendered.
 */
import type { ChartDataTableDto, ChartDto } from "@tally/bridge";
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { useController } from "../context";
import { t } from "../i18n";
import { useW56Store } from "../data/store";
import { useV3Store } from "../stores/v3Store";
import { arrowNav, useReturnFocus, useTitleId } from "../components/dialogA11y";
import { trapTab } from "../components/trapTab";
import dlg from "../components/Dialogs.module.css";
import styles from "./ChartDataTableDialog.module.css";
import { useChartStore } from "./chartStore";
import {
  cellValue,
  dataColumns,
  deleteRow,
  deleteSeries,
  firstColumnOf,
  formatCell,
  headerGroups,
  insertRow,
  insertSeries,
  mayDeleteRow,
  mayMoveRowDown,
  moveRowDown,
  moveSeriesRight,
  parseCell,
  rowCount,
  sameTable,
  seriesEditable,
  setCell,
  setSeriesName,
  withoutAddedColumns,
  withRoleColumns,
  type DataColumn,
  type DataCursor,
  type DataRole,
} from "./dataTable";

/** The decimal separator of a locale tag. */
function decimalSepOf(tag: string | null | undefined): string {
  try {
    return new Intl.NumberFormat(tag || "en-US").formatToParts(1.5).find((p) => p.type === "decimal")?.value ?? ".";
  } catch {
    return ".";
  }
}

function roleLabel(role: DataRole): string {
  switch (role) {
    case "categories":
      return t("chart.role.categories", "Categories");
    case "x":
      return t("chart.role.x", "X-Values");
    case "y":
      return t("chart.role.y", "Y-Values");
    case "size":
      return t("chart.role.size", "Bubble Sizes");
    case "open":
      return t("chart.role.open", "Open Values");
    case "low":
      return t("chart.role.low", "Low Values");
    case "high":
      return t("chart.role.high", "High Values");
    case "close":
      return t("chart.role.close", "Close Values");
  }
}

/** Rows rendered above and below the visible ones. */
const OVERSCAN = 12;

export function ChartDataTableDialog() {
  useW56Store((s) => s.i18nRev);
  const open = useChartStore((s) => s.dataTable);
  const chart = useChartStore((s) => (s.dataTable ? (s.charts.find((c) => c.id === s.dataTable!.id) ?? null) : null));
  if (open === null || !chart?.ownData) return null;
  return <DataTableEditor key={open.id} chart={chart} base={open.table} />;
}

function DataTableEditor({ chart, base }: { chart: ChartDto; base: ChartDataTableDto }) {
  const ctl = useController();
  const titleId = useTitleId();
  useReturnFocus();
  const decimalSep = decimalSepOf(useV3Store((s) => s.locale));
  const initial = useMemo(() => withRoleColumns(chart, base), [chart, base]);
  const [table, setTable] = useState(initial);
  const [cursor, setCursor] = useState<DataCursor>({ row: 0, col: 0 });
  // A series name field has the focus (AOO: the row tools are off then).
  const [headerSeries, setHeaderSeries] = useState<number | null>(null);
  const [edit, setEdit] = useState<{ text: string } | null>(null);
  const [invalid, setInvalid] = useState(false);
  const dialogRef = useRef<HTMLDivElement>(null);
  const gridRef = useRef<HTMLTableElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  // Row virtualisation: the scroll offset and view height (state, so a scroll re-renders) and a measured row height.
  const [view, setView] = useState({ top: 0, height: 480 });
  const [{ rowH, headH }, setMetrics] = useState({ rowH: 25, headH: 50 });
  const editRef = useRef<HTMLInputElement>(null);
  const cols = useMemo(() => dataColumns(chart, table), [chart, table]);
  const rows = rowCount(table);
  const row = Math.min(cursor.row, Math.max(0, rows - 1));
  const col = Math.min(cursor.col, Math.max(0, cols.length - 1));
  const curCol: DataColumn | undefined = cols[col];
  const curSeries = headerSeries ?? (curCol && curCol.series >= 0 ? curCol.series : null);
  const editable = seriesEditable(chart);

  // The rendered rows: those in view plus OVERSCAN, always including the cursor row.
  let first = Math.max(0, Math.floor((view.top - headH) / rowH) - OVERSCAN);
  let last = Math.min(rows, Math.ceil((view.top - headH + view.height) / rowH) + OVERSCAN);
  const span = Math.max(1, last - first);
  if (row < first || row >= last) {
    first = Math.max(0, Math.min(row - OVERSCAN, rows - span));
    last = Math.min(rows, first + span);
  }
  const onScroll = (): void => {
    const el = scrollRef.current;
    if (el) setView({ top: el.scrollTop, height: el.clientHeight });
  };
  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (el) setView({ top: el.scrollTop, height: el.clientHeight || 480 });
    // Measured once on open (fixed-height rows); scrolling updates the view.
    const tr = gridRef.current?.tBodies[0]?.querySelector<HTMLElement>("tr[aria-rowindex]");
    const head = gridRef.current?.tHead?.offsetHeight ?? 0;
    if (tr && tr.offsetHeight > 0) setMetrics({ rowH: tr.offsetHeight, headH: head });
  }, []);

  // The cursor cell takes the focus when it moves (not while a name field or the toolbar has it).
  const focusCell = useRef(true);
  useEffect(() => {
    if (edit) {
      editRef.current?.focus();
      return;
    }
    if (!focusCell.current) return;
    focusCell.current = false;
    const cell = gridRef.current?.querySelector<HTMLElement>('[data-cursor="true"]');
    (cell ?? dialogRef.current?.querySelector<HTMLElement>("button"))?.focus();
  }, [row, col, edit, rows, cols.length]);

  const moveTo = (r: number, c: number): void => {
    focusCell.current = true;
    setHeaderSeries(null);
    setCursor({ row: Math.max(0, Math.min(r, rows - 1)), col: Math.max(0, Math.min(c, cols.length - 1)) });
  };

  /** Commits the cell edit; false (and stays in edit) when a value cell holds no number. */
  const commitEdit = (): boolean => {
    if (!edit || !curCol) return true;
    if (curCol.series < 0) {
      setTable((tb) => setCell(tb, curCol, row, edit.text));
    } else {
      const n = parseCell(edit.text, decimalSep);
      if (n === undefined) {
        setInvalid(true);
        return false;
      }
      setTable((tb) => setCell(tb, curCol, row, n));
    }
    setInvalid(false);
    setEdit(null);
    focusCell.current = true;
    return true;
  };

  const cancelEdit = (): void => {
    setEdit(null);
    setInvalid(false);
    focusCell.current = true;
  };

  const startEdit = (text?: string): void => {
    if (!curCol || rows === 0) return;
    setEdit({ text: text ?? formatCell(cellValue(table, curCol, row), decimalSep) });
  };

  /** Close / Escape: apply the edits (AOO's dialog has no Cancel). */
  const close = (): void => {
    let final = table;
    if (edit && curCol) {
      // A pending valid edit is kept; an invalid one is dropped (AOO asks first).
      if (curCol.series < 0) final = setCell(final, curCol, row, edit.text);
      else {
        const n = parseCell(edit.text, decimalSep);
        if (n !== undefined) final = setCell(final, curCol, row, n);
      }
    }
    ctl.w5.charts.closeDataTable();
    final = withoutAddedColumns(final, base);
    if (!sameTable(final, base)) void ctl.w5.charts.setDataTable(chart.id, final);
    ctl.focusGrid();
  };

  // ----- toolbar (DataEditor::ToolboxHdl) ---------------------------------------------
  const busy = edit !== null && invalid;
  const tools = {
    insertRow: !busy && headerSeries === null,
    insertSeries: !busy && editable,
    deleteRow: headerSeries === null && mayDeleteRow(table, row),
    deleteSeries: editable && curSeries !== null,
    moveSeriesRight: !busy && curSeries !== null && curSeries < table.series.length - 1,
    moveRowDown: !busy && headerSeries === null && mayMoveRowDown(table, row),
  };
  const run = (f: () => void): void => {
    if (edit && !commitEdit()) return;
    f();
    // The focus stays on the tool (repeat it with Space); a tool that just
    // turned off hands it to the cursor cell.
    requestAnimationFrame(() => {
      const a = document.activeElement;
      if (!dialogRef.current?.contains(a) || (a instanceof HTMLButtonElement && a.disabled)) {
        gridRef.current?.querySelector<HTMLElement>('[data-cursor="true"]')?.focus();
      }
    });
  };
  const onInsertRow = () => run(() => setTable((tb) => insertRow(tb, rows === 0 ? -1 : row)));
  const onInsertSeries = () =>
    run(() => {
      // After the cursor's series; the categories put it first (DataBrowserModel::insertDataSeries).
      setTable((tb) => insertSeries(tb, chart, curSeries ?? -1));
    });
  const onDeleteRow = () => run(() => setTable((tb) => deleteRow(tb, row)));
  const onDeleteSeries = () =>
    run(() => {
      if (curSeries === null) return;
      setTable((tb) => deleteSeries(tb, curSeries));
      setHeaderSeries(null);
    });
  const onMoveSeriesRight = () =>
    run(() => {
      if (curSeries === null) return;
      // An updater: `run` has just queued the pending cell edit, which the rendered `table` does not hold yet.
      setTable((tb) => moveSeriesRight(tb, curSeries));
      const offset = col - firstColumnOf(cols, curSeries);
      // The column layout does not depend on cell values, so the rendered table gives the moved layout.
      const nextCols = dataColumns(chart, moveSeriesRight(table, curSeries));
      setHeaderSeries(null);
      // The cursor stays in the moved series (DataBrowser::SwapColumn).
      setCursor({ row, col: firstColumnOf(nextCols, curSeries + 1) + Math.max(0, offset) });
    });
  const onMoveRowDown = () =>
    run(() => {
      setTable((tb) => moveRowDown(tb, row));
      setCursor({ row: row + 1, col });
    });

  // ----- keys ------------------------------------------------------------------------
  const onCellKey = (e: KeyboardEvent<HTMLElement>): void => {
    if (edit) return;
    const k = e.key;
    const ctrl = e.ctrlKey || e.metaKey;
    let handled = true;
    if (k === "ArrowDown") moveTo(row + 1, col);
    else if (k === "ArrowUp") moveTo(row - 1, col);
    else if (k === "ArrowRight") moveTo(row, col + 1);
    else if (k === "ArrowLeft") moveTo(row, col - 1);
    else if (k === "Home") moveTo(ctrl ? 0 : row, 0);
    else if (k === "End") moveTo(ctrl ? rows - 1 : row, cols.length - 1);
    else if (k === "PageDown") moveTo(row + 10, col);
    else if (k === "PageUp") moveTo(row - 10, col);
    else if (k === "Enter" || k === "F2") startEdit();
    else if (k === "Delete" || k === "Backspace") {
      if (curCol) setTable((tb) => setCell(tb, curCol, row, curCol.series < 0 ? "" : null));
    } else if (k.length === 1 && !ctrl && !e.altKey) startEdit(k);
    else handled = false;
    if (handled) {
      e.preventDefault();
      e.stopPropagation();
    }
  };

  const onEditKey = (e: KeyboardEvent<HTMLInputElement>): void => {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      cancelEdit();
    } else if (e.key === "Enter") {
      e.preventDefault();
      e.stopPropagation();
      if (commitEdit()) setCursor({ row: Math.min(row + 1, rows - 1), col });
    } else if (e.key === "Tab") {
      e.preventDefault();
      e.stopPropagation();
      if (commitEdit()) setCursor({ row, col: Math.max(0, Math.min(col + (e.shiftKey ? -1 : 1), cols.length - 1)) });
    }
  };

  const groups = headerGroups(cols);
  const label = (b: keyof typeof tools): string =>
    ({
      insertRow: t("chart.dataTable.insertRow", "Insert Row"),
      insertSeries: t("chart.dataTable.insertSeries", "Insert Series"),
      deleteRow: t("chart.dataTable.deleteRow", "Delete Row"),
      deleteSeries: t("chart.dataTable.deleteSeries", "Delete Series"),
      moveSeriesRight: t("chart.dataTable.moveSeriesRight", "Move Series Right"),
      moveRowDown: t("chart.dataTable.moveRowDown", "Move Row Down"),
    })[b];
  const actions: [keyof typeof tools, () => void][] = [
    ["insertRow", onInsertRow],
    ["insertSeries", onInsertSeries],
    ["deleteRow", onDeleteRow],
    ["deleteSeries", onDeleteSeries],
    ["moveSeriesRight", onMoveSeriesRight],
    ["moveRowDown", onMoveRowDown],
  ];

  return (
    <div className={dlg.overlay} onMouseDown={(e) => e.target === e.currentTarget && close()}>
      <div
        ref={dialogRef}
        className={`${dlg.card} ${styles.card}`}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        data-testid="chart-data-table"
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.preventDefault();
            e.stopPropagation();
            close();
            return;
          }
          if (arrowNav(e)) return;
          if (dialogRef.current) trapTab(dialogRef.current, e, document.activeElement);
        }}
      >
        <h2 id={titleId} className={dlg.title}>
          {t("chart.dataTable.title", "Data Table")}
        </h2>
        <div className={styles.toolbar} role="toolbar" aria-label={t("chart.dataTable.tools", "Data table tools")}>
          {actions.map(([key, onClick], i) => (
            <button key={key} type="button" className={`${dlg.secondary} ${styles.tool}${i === 4 ? ` ${styles.sep}` : ""}`} disabled={!tools[key]} data-testid={`dt-${key}`} onClick={onClick}>
              {label(key)}
            </button>
          ))}
        </div>
        <div ref={scrollRef} className={styles.scroll} onScroll={onScroll}>
          <table ref={gridRef} className={styles.grid} role="grid" aria-label={t("chart.dataTable.grid", "Chart data")} aria-rowcount={rows + 2} aria-colcount={cols.length + 1}>
            <thead>
              <tr aria-rowindex={1}>
                <th className={styles.corner} aria-hidden="true" />
                {groups.map((g) =>
                  g.series < 0 ? (
                    <th key="cats" colSpan={g.span} className={styles.nameCell} aria-hidden="true" />
                  ) : (
                    <th key={`s${g.series}`} colSpan={g.span} className={styles.nameCell} scope="colgroup">
                      <input
                        className={styles.name}
                        aria-label={t("cp.seriesName", "Series name")}
                        placeholder={t("cp.seriesN", "Series {n}", { n: g.series + 1 })}
                        value={table.series[g.series]?.name ?? ""}
                        data-testid={`dt-name-${g.series}`}
                        onFocus={() => setHeaderSeries(g.series)}
                        onChange={(e) => setTable((tb) => setSeriesName(tb, g.series, e.target.value))}
                        onKeyDown={(e) => {
                          if (e.key === "Enter" || e.key === "ArrowDown") {
                            e.preventDefault();
                            moveTo(0, g.start);
                          }
                        }}
                      />
                    </th>
                  ),
                )}
              </tr>
              <tr aria-rowindex={2}>
                <th className={styles.corner} aria-hidden="true" />
                {cols.map((c, i) => (
                  <th key={i} role="columnheader" className={styles.role} aria-colindex={i + 2}>
                    {roleLabel(c.role)}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {first > 0 ? (
                <tr aria-hidden="true" className={styles.spacer}>
                  <td colSpan={cols.length + 1} style={{ height: first * rowH }} />
                </tr>
              ) : null}
              {Array.from({ length: last - first }, (_, k) => first + k).map((r) => (
                <tr key={r} aria-rowindex={r + 3}>
                  <th role="rowheader" className={styles.rowHead}>
                    {r + 1}
                  </th>
                  {cols.map((c, i) => {
                    const at = r === row && i === col;
                    const v = cellValue(table, c, r);
                    return (
                      <td
                        key={i}
                        role="gridcell"
                        aria-colindex={i + 2}
                        aria-selected={at}
                        tabIndex={at && !edit ? 0 : -1}
                        data-cursor={at ? "true" : undefined}
                        data-testid={`dt-cell-${r}-${i}`}
                        className={`${styles.cell}${c.series < 0 ? "" : ` ${styles.num}`}${at ? ` ${styles.at}` : ""}`}
                        onMouseDown={(e) => {
                          if (at && edit) return;
                          e.preventDefault();
                          if (edit && !commitEdit()) return;
                          moveTo(r, i);
                        }}
                        onDoubleClick={() => {
                          if (!edit) startEdit();
                        }}
                        onKeyDown={onCellKey}
                      >
                        {at && edit ? (
                          <input
                            ref={editRef}
                            className={styles.edit}
                            aria-label={`${roleLabel(c.role)} ${r + 1}`}
                            aria-invalid={invalid}
                            aria-describedby={invalid ? `${titleId}-err` : undefined}
                            value={edit.text}
                            data-testid="dt-edit"
                            onChange={(e) => {
                              setEdit({ text: e.target.value });
                              setInvalid(false);
                            }}
                            onKeyDown={onEditKey}
                          />
                        ) : (
                          formatCell(v, decimalSep)
                        )}
                      </td>
                    );
                  })}
                </tr>
              ))}
              {last < rows ? (
                <tr aria-hidden="true" className={styles.spacer}>
                  <td colSpan={cols.length + 1} style={{ height: (rows - last) * rowH }} />
                </tr>
              ) : null}
            </tbody>
          </table>
        </div>
        {invalid ? (
          <p id={`${titleId}-err`} className={dlg.error} role="alert">
            {t("chart.dataTable.invalidNumber", "Numbers are required. Check your input.")}
          </p>
        ) : null}
        <div className={dlg.actions}>
          <button type="button" className={dlg.primary} data-testid="dt-close" onClick={close}>
            {t("dialog.close", "Close")}
          </button>
        </div>
      </div>
    </div>
  );
}
