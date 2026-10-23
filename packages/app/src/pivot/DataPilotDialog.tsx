/**
 * DataPilot layout dialog (W5-6A): the field list from `pivot_field_items`,
 * the Page / Column / Row / Data areas (drag and drop, or the ▸ buttons and
 * a "Move to…" menu for the keyboard), the function picker per data field,
 * the Field Options sub-panel (subtotals, sort, show empty, hidden items,
 * layout, AutoShow, show-as) and the result options (target, totals,
 * filter button, drill-down).
 */
import type { AggFn, PivotFieldItemsDto, PivotLayout, ShowAsKind } from "@tally/bridge";
import { useState, type DragEvent } from "react";
import { cx, useController } from "../context";
import { a1 } from "../dataOps";
import { t } from "../i18n";
import { useDocStore } from "../stores/docStore";
import dlg from "../components/Dialogs.module.css";
import { Check, Field, RangeField, W56Shell, parseRangeText } from "../data/W56Shell";
import type { W56Dialog } from "../data/store";
import w56 from "../data/W56.module.css";
import styles from "./DataPilot.module.css";
import { AGG_FNS, PIVOT_LAYOUTS, SHOW_AS, addField, availableFields, dataCaption, layoutError, moveField, removeField, updateDataField, updateField, type LayoutDataField, type LayoutField, type PivotArea, type PivotLayoutState } from "./pivotModel";

type Axis = Exclude<PivotArea, "data">;
const AXES: readonly { area: Axis; key: string; label: string }[] = [
  { area: "page", key: "pivot.pageFields", label: "Page fields" },
  { area: "col", key: "pivot.columnFields", label: "Column fields" },
  { area: "row", key: "pivot.rowFields", label: "Row fields" },
];

function dragData(e: DragEvent, data: { from: PivotArea | "list"; sourceCol: number; dataIndex?: number }) {
  e.dataTransfer.setData("text/x-tally-pivot", JSON.stringify(data));
  e.dataTransfer.effectAllowed = "move";
}

function readDrag(e: DragEvent): { from: PivotArea | "list"; sourceCol: number; dataIndex?: number } | null {
  try {
    return JSON.parse(e.dataTransfer.getData("text/x-tally-pivot")) as { from: PivotArea | "list"; sourceCol: number; dataIndex?: number };
  } catch {
    return null;
  }
}

export function DataPilotDialog({ d }: { d: Extract<W56Dialog, { kind: "dataPilot" }> }) {
  const ctl = useController();
  const sheets = useDocStore((s) => s.info?.sheets ?? []);
  const [st, setSt] = useState<PivotLayoutState>(d.layout);
  const [selected, setSelected] = useState<{ area: PivotArea; sourceCol: number; dataIndex?: number } | null>(null);
  const [moveMenu, setMoveMenu] = useState<{ area: PivotArea | "list"; sourceCol: number; dataIndex?: number } | null>(null);
  const [target, setTarget] = useState(a1({ r0: st.target.r, c0: st.target.c, r1: st.target.r, c1: st.target.c }));
  const [more, setMore] = useState(false);
  const fieldOf = (col: number) => d.fields.find((f) => f.sourceCol === col);
  const available = availableFields(st, d.fields);
  const error = layoutError(st);
  const targetCell = parseRangeText(target);
  const targetError = st.targetSheet !== null && !targetCell ? "Choose a target cell." : null;
  const src = st.source;
  const sourceText = src.type === "range" ? `${sheets.find((s) => s.id === src.sheet)?.name ?? "Sheet"}.${a1(src.range)}` : src.name;

  const drop = (area: PivotArea | "list") => (e: DragEvent) => {
    e.preventDefault();
    const data = readDrag(e);
    if (!data) return;
    moveTo(data, area);
  };
  const moveTo = (data: { from: PivotArea | "list"; sourceCol: number; dataIndex?: number }, area: PivotArea | "list") => {
    const f = fieldOf(data.sourceCol);
    if (!f) return;
    if (area === "list") {
      setSt((s) => removeField(s, data.sourceCol, data.from === "data" ? (data.dataIndex ?? null) : null));
    } else if (area === "data") {
      setSt((s) => addField(data.from === "data" ? s : removeField(s, data.sourceCol, null), f, "data"));
    } else if (data.from === "list" || data.from === "data") {
      const axis = area;
      const fromData = data.from === "data";
      setSt((s) => addField(fromData ? removeField(s, data.sourceCol, data.dataIndex ?? null) : s, f, axis));
    } else {
      const from = data.from;
      const axis = area;
      setSt((s) => moveField(s, data.sourceCol, from, axis));
    }
    setMoveMenu(null);
    setSelected(null);
  };

  const submit = async () => {
    const layout = st.targetSheet === null ? st : { ...st, target: targetCell ? { r: targetCell.r0, c: targetCell.c0 } : st.target };
    await ctl.w56.savePivot(layout, d.existing);
  };

  const selField = selected && selected.area !== "data" ? st[selected.area].find((f) => f.sourceCol === selected.sourceCol) : undefined;
  const selData = selected?.area === "data" && selected.dataIndex !== undefined ? st.data[selected.dataIndex] : undefined;

  return (
    <W56Shell title={`${t("pivot.title", "DataPilot")} — ${t("pivot.layout", "Layout")}`} ok={d.existing ? t("dialog.apply", "Apply") : t("dialog.ok", "OK")} size="xwide" testId="datapilot-dialog" valid={error === null && targetError === null} onSubmit={submit}>
      <p className={dlg.lead}>
        {t("pivot.source", "Source")}: <b>{sourceText}</b>. Drag fields into the areas, or select one and use Move to.
      </p>
      <div className={styles.layout}>
        <div className={styles.areas}>
          {AXES.map((ax) => (
            <div key={ax.area} className={cx(styles.area, styles[ax.area])} onDragOver={(e) => e.preventDefault()} onDrop={drop(ax.area)} data-testid={`pivot-area-${ax.area}`}>
              <div className={styles.areaTitle}>{t(ax.key, ax.label)}</div>
              {st[ax.area].map((f) => (
                <FieldChip key={f.sourceCol} label={f.name} selected={selected?.area === ax.area && selected.sourceCol === f.sourceCol} onDragStart={(e) => dragData(e, { from: ax.area, sourceCol: f.sourceCol })} onClick={() => setSelected({ area: ax.area, sourceCol: f.sourceCol })} onMove={() => setMoveMenu({ area: ax.area, sourceCol: f.sourceCol })} onRemove={() => moveTo({ from: ax.area, sourceCol: f.sourceCol }, "list")} />
              ))}
              {st[ax.area].length === 0 ? <div className={styles.areaEmpty}>Drop a field here</div> : null}
            </div>
          ))}
          <div className={cx(styles.area, styles.data)} onDragOver={(e) => e.preventDefault()} onDrop={drop("data")} data-testid="pivot-area-data">
            <div className={styles.areaTitle}>{t("pivot.dataFields", "Data fields")}</div>
            {st.data.map((f, i) => (
              <FieldChip key={`${f.sourceCol}-${i}`} label={dataCaption(f)} selected={selected?.area === "data" && selected.dataIndex === i} onDragStart={(e) => dragData(e, { from: "data", sourceCol: f.sourceCol, dataIndex: i })} onClick={() => setSelected({ area: "data", sourceCol: f.sourceCol, dataIndex: i })} onMove={() => setMoveMenu({ area: "data", sourceCol: f.sourceCol, dataIndex: i })} onRemove={() => moveTo({ from: "data", sourceCol: f.sourceCol, dataIndex: i }, "list")} />
            ))}
            {st.data.length === 0 ? <div className={styles.areaEmpty}>Drop the values to summarise here</div> : null}
          </div>
        </div>
        <div className={styles.fieldList} onDragOver={(e) => e.preventDefault()} onDrop={drop("list")} data-testid="pivot-field-list">
          <div className={styles.areaTitle}>{t("pivot.availableFields", "Available fields")}</div>
          {available.map((f) => (
            <FieldChip key={f.sourceCol} label={f.name} hint={f.numeric ? "123" : f.date ? "📅" : "abc"} onDragStart={(e) => dragData(e, { from: "list", sourceCol: f.sourceCol })} onClick={() => setSelected(null)} onMove={() => setMoveMenu({ area: "list", sourceCol: f.sourceCol })} onAdd={{ row: () => moveTo({ from: "list", sourceCol: f.sourceCol }, "row"), col: () => moveTo({ from: "list", sourceCol: f.sourceCol }, "col"), data: () => moveTo({ from: "list", sourceCol: f.sourceCol }, "data") }} />
          ))}
          {available.length === 0 ? <div className={styles.areaEmpty}>Every field is placed.</div> : null}
        </div>
      </div>
      {moveMenu ? (
        <div className={styles.moveMenu} role="menu" aria-label={t("pivot.moveTo", "Move to…")} data-testid="pivot-move-menu">
          <span className={w56.muted}>{t("pivot.moveTo", "Move to…")}</span>
          {(["page", "col", "row", "data", "list"] as const).map((area) => (
            <button key={area} type="button" role="menuitem" className={w56.chip} disabled={moveMenu.area === area} onClick={() => moveTo({ from: moveMenu.area, sourceCol: moveMenu.sourceCol, ...(moveMenu.dataIndex !== undefined ? { dataIndex: moveMenu.dataIndex } : {}) }, area)} data-testid={`pivot-move-${area}`}>
              {area === "list" ? "Field list" : AXES.find((a) => a.area === area)?.label ?? "Data fields"}
            </button>
          ))}
          <button type="button" className={dlg.rowBtn} onClick={() => setMoveMenu(null)}>
            ×
          </button>
        </div>
      ) : null}
      {selData ? <DataFieldOptions f={selData} index={selected?.dataIndex ?? 0} st={st} fields={d.fields} onChange={(p) => setSt((s) => updateDataField(s, selected?.dataIndex ?? 0, p))} /> : null}
      {selField && selected && selected.area !== "data" ? <FieldOptions f={selField} area={selected.area} fields={d.fields} st={st} onChange={(p) => setSt((s) => updateField(s, selected.area as Axis, selected.sourceCol, p))} /> : null}
      <button type="button" className={dlg.rowBtn} onClick={() => setMore(!more)} aria-expanded={more} data-testid="pivot-more">
        {more ? "▾" : "▸"} {t("dialog.more", "More")}
      </button>
      {more ? (
        <div className={styles.options}>
          <Field label={t("pivot.resultsTo", "Results to")}>
            <select className={dlg.select} value={st.targetSheet === null ? "new" : String(st.targetSheet)} onChange={(e) => setSt({ ...st, targetSheet: e.target.value === "new" ? null : Number(e.target.value) })} data-testid="pivot-target-sheet">
              <option value="new">{t("pivot.newSheet", "New sheet")}</option>
              {sheets.map((s) => (
                <option key={s.id} value={String(s.id)}>
                  {s.name}
                </option>
              ))}
            </select>
          </Field>
          {st.targetSheet !== null ? <RangeField label="Target cell" value={target} onChange={setTarget} testId="pivot-target" placeholder="H1" /> : null}
          <div className={dlg.optionGrid}>
            <Check label={t("pivot.ignoreEmpty", "Ignore empty rows")} checked={st.ignoreEmptyRows} onChange={(v) => setSt({ ...st, ignoreEmptyRows: v })} />
            <Check label={t("pivot.identifyCategories", "Identify categories")} checked={st.identifyCategories} onChange={(v) => setSt({ ...st, identifyCategories: v })} />
            <Check label={t("pivot.totalColumns", "Total columns")} checked={st.colTotals} onChange={(v) => setSt({ ...st, colTotals: v })} testId="pivot-total-cols" />
            <Check label={t("pivot.totalRows", "Total rows")} checked={st.rowTotals} onChange={(v) => setSt({ ...st, rowTotals: v })} testId="pivot-total-rows" />
            <Check label={t("pivot.addFilter", "Add filter")} checked={st.showFilterButton} onChange={(v) => setSt({ ...st, showFilterButton: v })} />
            <Check label={t("pivot.enableDrillDown", "Enable drill to details")} checked={st.drillDown} onChange={(v) => setSt({ ...st, drillDown: v })} />
          </div>
          <Field label="Data pseudo-field orientation">
            <select className={dlg.select} value={st.dataLayout} onChange={(e) => setSt({ ...st, dataLayout: e.target.value as "row" | "column" })}>
              <option value="column">Column</option>
              <option value="row">Row</option>
            </select>
          </Field>
        </div>
      ) : null}
      {error ? <p className={dlg.error}>{error}</p> : null}
      {targetError ? <p className={dlg.error}>{targetError}</p> : null}
    </W56Shell>
  );
}

function FieldChip({ label, hint, selected, onDragStart, onClick, onMove, onRemove, onAdd }: { label: string; hint?: string; selected?: boolean; onDragStart: (e: DragEvent) => void; onClick: () => void; onMove: () => void; onRemove?: () => void; onAdd?: { row: () => void; col: () => void; data: () => void } }) {
  return (
    <div className={cx(styles.chip, selected && styles.chipOn)} draggable onDragStart={onDragStart} onClick={onClick} onDoubleClick={onAdd?.row} role="group" aria-label={label} data-testid={`pivot-field-${label}`}>
      {/* The chip's own buttons sit side by side (no nested controls): the field name selects it. */}
      <button type="button" className={cx(styles.chipPick, styles.chipLabel)} aria-pressed={selected} onClick={(e) => (e.stopPropagation(), onClick())}>
        {label}
      </button>
      {hint ? <span className={styles.chipHint}>{hint}</span> : null}
      {onAdd ? (
        <span className={styles.chipBtns}>
          <button type="button" title="Add to row fields" aria-label={`${label}: add to rows`} onClick={(e) => (e.stopPropagation(), onAdd.row())} data-testid={`pivot-add-row-${label}`}>
            R
          </button>
          <button type="button" title="Add to column fields" aria-label={`${label}: add to columns`} onClick={(e) => (e.stopPropagation(), onAdd.col())} data-testid={`pivot-add-col-${label}`}>
            C
          </button>
          <button type="button" title="Add to data fields" aria-label={`${label}: add to data`} onClick={(e) => (e.stopPropagation(), onAdd.data())} data-testid={`pivot-add-data-${label}`}>
            Σ
          </button>
        </span>
      ) : null}
      <button type="button" className={styles.chipMenu} title={t("pivot.moveTo", "Move to…")} aria-label={`${label}: move to`} onClick={(e) => (e.stopPropagation(), onMove())}>
        ⋯
      </button>
      {onRemove ? (
        <button type="button" className={styles.chipMenu} title={t("dialog.remove", "Remove")} aria-label={`${label}: remove`} onClick={(e) => (e.stopPropagation(), onRemove())}>
          ×
        </button>
      ) : null}
    </div>
  );
}

function DataFieldOptions({ f, index, st, fields, onChange }: { f: LayoutDataField; index: number; st: PivotLayoutState; fields: PivotFieldItemsDto[]; onChange: (p: Partial<LayoutDataField>) => void }) {
  const axisFields = [...st.row, ...st.col];
  const items = f.showAs?.baseField !== null && f.showAs?.baseField !== undefined ? (fields.find((x) => x.sourceCol === f.showAs?.baseField)?.items ?? []) : [];
  return (
    <div className={styles.options} data-testid="pivot-data-options">
      <div className={w56.section}>
        {t("pivot.fieldOptions", "Data Field")}: {f.name} (#{index + 1})
      </div>
      <div className={w56.grid3}>
        <Field label={t("pivot.function", "Function")}>
          <select className={dlg.select} value={f.function} onChange={(e) => onChange({ function: e.target.value as AggFn })} data-testid="pivot-data-fn">
            {AGG_FNS.map((a) => (
              <option key={a.fn} value={a.fn}>
                {t(a.key, a.label)}
              </option>
            ))}
          </select>
        </Field>
        <Field label="Caption (optional)">
          <input className={dlg.input} value={f.caption ?? ""} placeholder={dataCaption({ ...f, caption: null })} onChange={(e) => onChange({ caption: e.target.value === "" ? null : e.target.value })} />
        </Field>
        <Field label="Displayed value">
          <select className={dlg.select} value={f.showAs?.kind ?? ""} onChange={(e) => onChange({ showAs: e.target.value === "" ? null : { kind: e.target.value as ShowAsKind, baseField: f.showAs?.baseField ?? axisFields[0]?.sourceCol ?? null, baseItem: f.showAs?.baseItem ?? null } })}>
            <option value="">Normal</option>
            {SHOW_AS.map((s) => (
              <option key={s.kind} value={s.kind}>
                {s.label}
              </option>
            ))}
          </select>
        </Field>
        {f.showAs && (f.showAs.kind === "difference" || f.showAs.kind === "percentOf" || f.showAs.kind === "percentDifference" || f.showAs.kind === "runningTotal") ? (
          <>
            <Field label="Base field">
              <select className={dlg.select} value={f.showAs.baseField === null ? "" : String(f.showAs.baseField)} onChange={(e) => onChange({ showAs: { ...f.showAs!, baseField: e.target.value === "" ? null : Number(e.target.value) } })}>
                {axisFields.map((a) => (
                  <option key={a.sourceCol} value={String(a.sourceCol)}>
                    {a.name}
                  </option>
                ))}
              </select>
            </Field>
            {f.showAs.kind !== "runningTotal" ? (
              <Field label="Base item">
                <select className={dlg.select} value={f.showAs.baseItem ?? "__prev"} onChange={(e) => onChange({ showAs: { ...f.showAs!, baseItem: e.target.value } })}>
                  <option value="__prev">- previous item -</option>
                  <option value="__next">- next item -</option>
                  {items.map((it) => (
                    <option key={it.text} value={it.text}>
                      {it.text}
                    </option>
                  ))}
                </select>
              </Field>
            ) : null}
          </>
        ) : null}
      </div>
    </div>
  );
}

function FieldOptions({ f, area, fields, st, onChange }: { f: LayoutField; area: Axis; fields: PivotFieldItemsDto[]; st: PivotLayoutState; onChange: (p: Partial<LayoutField>) => void }) {
  const items = fields.find((x) => x.sourceCol === f.sourceCol)?.items ?? [];
  const subtotalType = f.subtotals.type;
  const customFns = f.subtotals.type === "custom" ? f.subtotals.fns : [];
  const toggleHidden = (text: string) => onChange({ hiddenItems: f.hiddenItems.includes(text) ? f.hiddenItems.filter((x) => x !== text) : [...f.hiddenItems, text] });
  return (
    <div className={styles.options} data-testid="pivot-field-options">
      <div className={w56.section}>
        Field: {f.name} ({AXES.find((a) => a.area === area)?.label})
      </div>
      {area === "page" ? (
        <Field label="Selected item">
          <select className={dlg.select} value={f.pageItem ?? ""} onChange={(e) => onChange({ pageItem: e.target.value === "" ? null : e.target.value })}>
            <option value="">- all -</option>
            {items.map((it) => (
              <option key={it.text} value={it.text}>
                {it.text}
              </option>
            ))}
          </select>
        </Field>
      ) : (
        <div className={w56.grid3}>
          <Field label="Subtotals">
            <select className={dlg.select} value={subtotalType} onChange={(e) => onChange({ subtotals: e.target.value === "auto" ? { type: "auto" } : e.target.value === "none" ? { type: "none" } : { type: "custom", fns: ["sum"] } })} data-testid="pivot-subtotals">
              <option value="auto">Automatic</option>
              <option value="none">None</option>
              <option value="custom">User-defined</option>
            </select>
          </Field>
          <Field label="Sort by">
            <select className={dlg.select} value={f.sort.type === "name" ? (f.sort.ascending ? "nameAsc" : "nameDesc") : f.sort.type === "data" ? (f.sort.ascending ? "dataAsc" : "dataDesc") : f.sort.type} onChange={(e) => onChange({ sort: e.target.value === "none" ? { type: "none" } : e.target.value === "manual" ? { type: "manual", order: items.map((i) => i.text) } : e.target.value.startsWith("name") ? { type: "name", ascending: e.target.value === "nameAsc" } : { type: "data", dataField: 0, ascending: e.target.value === "dataAsc" } })}>
              <option value="nameAsc">Name, ascending</option>
              <option value="nameDesc">Name, descending</option>
              <option value="dataAsc">Data field, ascending</option>
              <option value="dataDesc">Data field, descending</option>
              <option value="manual">Manual</option>
              <option value="none">None (source order)</option>
            </select>
          </Field>
          <Field label="Layout">
            <select className={dlg.select} value={f.layout} onChange={(e) => onChange({ layout: e.target.value as PivotLayout })}>
              {PIVOT_LAYOUTS.map((l) => (
                <option key={l.layout} value={l.layout}>
                  {l.label}
                </option>
              ))}
            </select>
          </Field>
        </div>
      )}
      {subtotalType === "custom" ? (
        <div className={w56.chips}>
          {AGG_FNS.map((a) => {
            const on = customFns.includes(a.fn);
            return (
              <button key={a.fn} type="button" className={on ? w56.chipOn : w56.chip} aria-pressed={on} onClick={() => onChange({ subtotals: { type: "custom", fns: on ? customFns.filter((x) => x !== a.fn) : [...customFns, a.fn] } })}>
                {t(a.key, a.label)}
              </button>
            );
          })}
        </div>
      ) : null}
      {area !== "page" ? (
        <div className={dlg.optionGrid}>
          <Check label="Show items without data" checked={f.showEmpty} onChange={(v) => onChange({ showEmpty: v })} />
          <Check label="Empty line after each item" checked={f.emptyLineAfter} onChange={(v) => onChange({ emptyLineAfter: v })} />
          <Check label="Show automatically (top N)" checked={f.autoShow !== null} onChange={(v) => onChange({ autoShow: v ? { top: true, count: 5, dataField: 0 } : null })} />
          {f.autoShow ? (
            <span className={w56.row}>
              <select className={dlg.selectSmall} value={f.autoShow.top ? "top" : "bottom"} onChange={(e) => onChange({ autoShow: { ...f.autoShow!, top: e.target.value === "top" } })}>
                <option value="top">Top</option>
                <option value="bottom">Bottom</option>
              </select>
              <input className={dlg.input} style={{ width: 60, height: 26 }} type="number" min={1} value={f.autoShow.count} onChange={(e) => onChange({ autoShow: { ...f.autoShow!, count: Math.max(1, Number(e.target.value) || 1) } })} />
              <select className={dlg.selectSmall} value={String(f.autoShow.dataField)} onChange={(e) => onChange({ autoShow: { ...f.autoShow!, dataField: Number(e.target.value) } })}>
                {st.data.map((df, i) => (
                  <option key={i} value={String(i)}>
                    {dataCaption(df)}
                  </option>
                ))}
              </select>
            </span>
          ) : null}
        </div>
      ) : null}
      {items.length > 0 && items.length <= 200 ? (
        <>
          <div className={w56.section}>Hide items</div>
          <div className={w56.chips}>
            {items.map((it) => {
              const hidden = f.hiddenItems.includes(it.text);
              return (
                <button key={it.text} type="button" className={hidden ? w56.chip : w56.chipOn} aria-pressed={!hidden} title={hidden ? "Hidden" : "Shown"} onClick={() => toggleHidden(it.text)}>
                  {it.text === "" ? "(empty)" : it.text} <span className={w56.muted}>{it.count}</span>
                </button>
              );
            })}
          </div>
        </>
      ) : null}
    </div>
  );
}
