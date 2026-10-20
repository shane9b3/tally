/**
 * v0.3 dialogs: Conditional Formatting manager (rule list, editor, style
 * editor, priority order), the "Highlight cells" value prompt, Data
 * Validation (criteria / input help / error alert tabs), the validation
 * Stop alert, Hyperlink, Protect / Unprotect Sheet, Protect Document, Goal
 * Seek, Document Properties and Page Setup.
 */
import type {
  CellPos,
  CfRuleDto,
  CfRuleKind,
  CfStyleDto,
  CfThreshold,
  CfValueKind,
  CmpOp,
  ConditionalFormatDto,
  DocPropertiesDto,
  GoalSeekResult,
  HeaderFooter,
  PageSetupDto,
  ProtectionOptions,
  ValidationDto,
  ValidationErrorStyle,
} from "@tally/bridge";
import { drawCfIcon } from "@tally/grid";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { cx, useController } from "../context";
import { rgbaToHex } from "../palette";
import { useDocStore } from "../stores/docStore";
import type { DialogState } from "../stores/uiStore";
import {
  ALERT_STYLES,
  CF_RULE_TYPES,
  CF_STYLE_PRESETS,
  CMP_OPS,
  COLOR_SCALES,
  colorScaleStops,
  DATE_PERIODS,
  DEFAULT_PROTECTION,
  defaultKind,
  defaultValidationKind,
  emptyCfStyle,
  HF_PLACEHOLDERS,
  HIGHLIGHT_PRESETS,
  ICON_SETS,
  iconThresholds,
  insertToken,
  linkError,
  listItems,
  moveItem,
  normalizeLink,
  operandFrom,
  operandText,
  PAPERS,
  paperMm,
  parseCell,
  parseNumber,
  parseRanges,
  parseSpan,
  presetRule,
  previewHeaderFooter,
  PROTECTION_LABELS,
  rangesText,
  ruleError,
  ruleSummary,
  spanText,
  TEXT_OPS,
  usesStyle,
  validationError,
  VALIDATION_TYPES,
  type CfRuleType,
  type ValidationType,
} from "../v03";
import { ColorPicker } from "./ColorPicker";
import { close, Shell, useAutoFocus } from "./Dialogs";
import styles from "./Dialogs.module.css";
import v3 from "./V3Dialogs.module.css";

type V3Kind =
  | "cfManager"
  | "cfPreset"
  | "validation"
  | "validationAlert"
  | "hyperlink"
  | "protectSheet"
  | "unprotectSheet"
  | "protectDocument"
  | "goalSeek"
  | "properties"
  | "pageSetup";

export type V3DialogState = Extract<DialogState, { kind: V3Kind }>;

const V3_KINDS = new Set<string>(["cfManager", "cfPreset", "validation", "validationAlert", "hyperlink", "protectSheet", "unprotectSheet", "protectDocument", "goalSeek", "properties", "pageSetup"]);

export function isV3Dialog(d: DialogState): d is V3DialogState {
  return V3_KINDS.has(d.kind);
}

export function V3Dialog({ dialog }: { dialog: V3DialogState }) {
  switch (dialog.kind) {
    case "cfManager":
      return <CfManagerDialog />;
    case "cfPreset":
      return <CfPresetDialog d={dialog} />;
    case "validation":
      return <ValidationDialog d={dialog} />;
    case "validationAlert":
      return <ValidationAlertDialog d={dialog} />;
    case "hyperlink":
      return <HyperlinkDialog d={dialog} />;
    case "protectSheet":
      return <ProtectSheetDialog />;
    case "unprotectSheet":
      return <UnprotectSheetDialog />;
    case "protectDocument":
      return <ProtectDocumentDialog d={dialog} />;
    case "goalSeek":
      return <GoalSeekDialog d={dialog} />;
    case "properties":
      return <PropertiesDialog d={dialog} />;
    case "pageSetup":
      return <PageSetupDialog d={dialog} />;
  }
}

// ---------------------------------------------------------------------------
// Small controls
// ---------------------------------------------------------------------------

function Tabs<T extends string>({ tabs, value, onChange, label }: { tabs: readonly { id: T; label: string }[]; value: T; onChange: (t: T) => void; label: string }) {
  return (
    <div className={v3.tabs} role="tablist" aria-label={label}>
      {tabs.map((t) => (
        <button key={t.id} type="button" role="tab" aria-selected={value === t.id} className={value === t.id ? v3.tabOn : v3.tab} onClick={() => onChange(t.id)} data-testid={`tab-${t.id}`}>
          {t.label}
        </button>
      ))}
    </div>
  );
}

/** A colour swatch button opening the palette. */
function ColorField({ label, value, onChange, allowNone, testId }: { label: string; value: number | null; onChange: (rgba: number | null) => void; allowNone?: boolean; testId?: string }) {
  const [open, setOpen] = useState<{ x: number; y: number } | null>(null);
  const shown = value !== null && (value & 255) > 0 ? rgbaToHex(value) : null;
  return (
    <span className={v3.colorField}>
      <button
        type="button"
        className={v3.swatchBtn}
        aria-label={label}
        title={label}
        data-testid={testId}
        onClick={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          setOpen(open ? null : { x: r.left, y: r.bottom + 4 });
        }}
      >
        <span className={cx(v3.swatch, shown === null && v3.swatchNone)} style={shown ? { background: shown } : undefined} />
        <span>{label}</span>
      </button>
      {value !== null && allowNone ? (
        <button type="button" className={v3.clearBtn} aria-label={`Clear ${label}`} onClick={() => onChange(null)}>
          ×
        </button>
      ) : null}
      {open ? <ColorPicker x={open.x} y={open.y} label={label} value={value} onPick={(c) => onChange(c)} onClose={() => setOpen(null)} /> : null}
    </span>
  );
}

function Check({ label, checked, onChange, testId }: { label: string; checked: boolean; onChange: (v: boolean) => void; testId?: string }) {
  return (
    <label className={styles.checkRow}>
      <input type="checkbox" checked={checked} onChange={(e) => onChange(e.target.checked)} data-testid={testId} />
      <span>{label}</span>
    </label>
  );
}

function Field({ label, children, className }: { label: string; children: ReactNode; className?: string }) {
  return (
    <label className={cx(styles.field, className)}>
      <span>{label}</span>
      {children}
    </label>
  );
}

// ---------------------------------------------------------------------------
// Conditional formatting
// ---------------------------------------------------------------------------

/** The style preview chip of a rule ("AaBbCc" in its colours). */
export function StylePreview({ rule }: { rule: CfRuleDto }) {
  const k = rule.kind;
  if (k.type === "colorScale") {
    const stops = k.stops.map((s) => rgbaToHex(s.color));
    return <span className={v3.previewScale} style={{ background: `linear-gradient(90deg, ${stops.join(", ")})` }} aria-hidden />;
  }
  if (k.type === "dataBar") {
    return (
      <span className={v3.previewBox} aria-hidden>
        <span className={v3.previewBar} style={{ background: `linear-gradient(90deg, ${rgbaToHex(k.color)}, ${rgbaToHex(k.color)}22)` }} />
      </span>
    );
  }
  if (k.type === "iconSet") return <IconStrip set={k.set} />;
  const st = rule.style;
  const chip = st.display?.type === "chip" ? st.display : null;
  const css = {
    fontWeight: st.bold ? 650 : undefined,
    fontStyle: st.italic ? "italic" : undefined,
    textDecoration: [st.underline ? "underline" : "", st.strike ? "line-through" : ""].filter(Boolean).join(" ") || undefined,
    color: chip ? rgbaToHex(chip.fg) : st.color !== null ? rgbaToHex(st.color) : undefined,
    background: chip ? rgbaToHex(chip.bg) : st.fill !== null && (st.fill & 255) > 0 ? rgbaToHex(st.fill) : undefined,
  };
  return (
    <span className={cx(v3.previewBox, chip && v3.previewChip)} style={css} aria-hidden>
      AaBbCc
    </span>
  );
}

/** The icons of a set, drawn with the grid's renderer. */
function IconStrip({ set }: { set: Extract<CfRuleKind, { type: "iconSet" }>["set"] }) {
  const ref = useRef<HTMLCanvasElement>(null);
  const n = set.startsWith("three") ? 3 : set.startsWith("four") ? 4 : 5;
  useEffect(() => {
    const c = ref.current;
    const ctx = c?.getContext?.("2d");
    if (!c || !ctx) return;
    const dpr = typeof window !== "undefined" ? window.devicePixelRatio || 1 : 1;
    c.width = n * 18 * dpr;
    c.height = 16 * dpr;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, n * 18, 16);
    for (let i = 0; i < n; i++) drawCfIcon(ctx, { set, index: n - 1 - i }, i * 18 + 1, 1, 14);
  }, [set, n]);
  return <canvas ref={ref} className={v3.iconStrip} style={{ width: n * 18, height: 16 }} aria-hidden />;
}

interface RuleRow {
  cf: ConditionalFormatDto;
  index: number;
  rule: CfRuleDto;
}

type Editing = { cf: ConditionalFormatDto | null; index: number | null; ranges: string; rule: CfRuleDto };

function CfManagerDialog() {
  const ctl = useController();
  const [list, setList] = useState<ConditionalFormatDto[] | null>(null);
  const [editing, setEditing] = useState<Editing | null>(null);
  const reload = () =>
    void ctl.v3.conditionalFormats().then(
      (l) => setList(l),
      (e: unknown) => {
        setList([]);
        ctl.notifyError(e);
      },
    );
  useEffect(reload, [ctl]);
  const rows: RuleRow[] = (list ?? []).flatMap((cf) => cf.rules.map((rule, index) => ({ cf, index, rule })));
  const after = (p: Promise<unknown>) => void p.then(reload);

  const move = (row: RuleRow, delta: -1 | 1) => {
    const cfs = list ?? [];
    const i = cfs.findIndex((c) => c.id === row.cf.id);
    const within = row.index + delta >= 0 && row.index + delta < row.cf.rules.length;
    if (within) after(ctl.v3.updateConditionalFormat(row.cf.id, row.cf.ranges, moveItem(row.cf.rules, row.index, delta)));
    else if (i + delta >= 0 && i + delta < cfs.length) after(ctl.v3.reorderConditionalFormats(moveItem(cfs, i, delta)));
  };

  if (editing) {
    return (
      <RuleEditor
        editing={editing}
        onCancel={() => setEditing(null)}
        onSave={(ranges, rule) => {
          const cf = editing.cf;
          const p =
            cf && editing.index !== null
              ? ctl.v3.updateConditionalFormat(cf.id, ranges, cf.rules.map((r, i) => (i === editing.index ? rule : r)))
              : ctl.v3.addConditionalFormat(ranges, [rule]);
          void p.then(() => {
            setEditing(null);
            reload();
          });
        }}
      />
    );
  }

  const newRule = () => setEditing({ cf: null, index: null, ranges: ctl.v3.selectionText(), rule: { kind: defaultKind("cellValue"), style: { ...CF_STYLE_PRESETS[0]!.style }, stopIfTrue: false } });
  return (
    <Shell
      title="Conditional formatting"
      wide
      noOk
      cardClass={v3.xwide}
      testId="cf-manager"
      onSubmit={() => undefined}
      cancelLabel="Done"
      extra={
        <button type="button" className={styles.primary} onClick={newRule} data-testid="cf-new">
          New rule…
        </button>
      }
    >
      <p className={styles.lead}>
        Rules for “{ctl.sheetName()}”, highest priority first. A cell takes the first matching rule's settings; ▲▼ change the order.
      </p>
      {list === null ? (
        <p className={styles.note}>Loading…</p>
      ) : rows.length === 0 ? (
        <div className={v3.empty}>
          <p>No rules on this sheet yet.</p>
          <p className={styles.note}>Select cells and choose New rule…, or use Highlight cells in the format bar.</p>
        </div>
      ) : (
        <div className={v3.ruleList} role="table" aria-label="Rules" data-testid="cf-rules">
          {rows.map((row, k) => (
            <div key={`${row.cf.id}:${row.index}`} className={v3.ruleRow} role="row" data-rule={k}>
              <span className={v3.order}>
                <button type="button" className={v3.iconBtn} aria-label="Move up" disabled={k === 0} onClick={() => move(row, -1)}>
                  ▲
                </button>
                <button type="button" className={v3.iconBtn} aria-label="Move down" disabled={k === rows.length - 1} onClick={() => move(row, 1)}>
                  ▼
                </button>
              </span>
              <StylePreview rule={row.rule} />
              <span className={v3.ruleText} role="cell">
                <span className={v3.ruleSummary}>{ruleSummary(row.rule)}</span>
                <span className={v3.ruleRanges}>{rangesText(row.cf.ranges)}</span>
              </span>
              <button type="button" className={styles.rowBtn} onClick={() => setEditing({ cf: row.cf, index: row.index, ranges: rangesText(row.cf.ranges), rule: structuredClone(row.rule) })}>
                Edit
              </button>
              <button
                type="button"
                className={cx(styles.rowBtn, styles.dangerText)}
                data-testid={`cf-delete-${k}`}
                onClick={() =>
                  after(row.cf.rules.length === 1 ? ctl.v3.deleteConditionalFormat(row.cf.id) : ctl.v3.updateConditionalFormat(row.cf.id, row.cf.ranges, row.cf.rules.filter((_, i) => i !== row.index)))
                }
              >
                Delete
              </button>
            </div>
          ))}
        </div>
      )}
    </Shell>
  );
}

function thresholdEditor(t: CfThreshold, onChange: (t: CfThreshold) => void, label: string, kinds: readonly CfValueKind[]) {
  const needsValue = t.kind === "number" || t.kind === "percent" || t.kind === "percentile" || t.kind === "formula";
  const names: Record<CfValueKind, string> = { min: "Lowest value", max: "Highest value", autoMin: "Automatic", autoMax: "Automatic", number: "Number", percent: "Percent", percentile: "Percentile", formula: "Formula" };
  return (
    <span className={v3.thresh}>
      <select className={styles.selectSmall} aria-label={`${label} type`} value={t.kind} onChange={(e) => onChange({ kind: e.target.value as CfValueKind, value: t.value })}>
        {kinds.map((k) => (
          <option key={k} value={k}>
            {names[k]}
          </option>
        ))}
      </select>
      {needsValue ? <input className={cx(styles.input, v3.small)} aria-label={`${label} value`} value={t.value} onChange={(e) => onChange({ ...t, value: e.target.value })} /> : null}
    </span>
  );
}

function RuleEditor({ editing, onCancel, onSave }: { editing: Editing; onCancel: () => void; onSave: (ranges: NonNullable<ReturnType<typeof parseRanges>>, rule: CfRuleDto) => void }) {
  const [ranges, setRanges] = useState(editing.ranges);
  const [rule, setRule] = useState<CfRuleDto>(editing.rule);
  const parsed = parseRanges(ranges);
  const err = parsed ? ruleError(rule) : "Enter the cells, e.g. F2:F15.";
  const k = rule.kind;
  const setKind = (kind: CfRuleKind) => setRule((r) => ({ ...r, kind }));
  const setStyle = (p: Partial<CfStyleDto>) => setRule((r) => ({ ...r, style: { ...r.style, ...p } }));
  const st = rule.style;
  const chip = st.display?.type === "chip" ? st.display : null;
  const tKinds: readonly CfValueKind[] = ["min", "max", "number", "percent", "percentile", "formula"];

  return (
    <Shell
      title={editing.cf ? "Edit rule" : "New rule"}
      ok={editing.cf ? "Save" : "Add rule"}
      wide
      cardClass={v3.xwide}
      testId="cf-editor"
      keepOpen
      valid={err === null}
      onSubmit={() => parsed && onSave(parsed, rule)}
      extra={
        <button type="button" className={styles.secondary} onClick={onCancel}>
          Back
        </button>
      }
    >
      <div className={v3.grid2}>
        <Field label="Apply to">
          <input className={cx(styles.input, styles.mono)} value={ranges} spellCheck={false} onChange={(e) => setRanges(e.target.value)} data-testid="cf-ranges" />
        </Field>
        <Field label="Rule type">
          <select className={styles.select} value={k.type} onChange={(e) => setKind(defaultKind(e.target.value as CfRuleType))} data-testid="cf-type">
            {CF_RULE_TYPES.map((t) => (
              <option key={t.type} value={t.type}>
                {t.label}
              </option>
            ))}
          </select>
        </Field>
      </div>

      <div className={v3.section}>
        {k.type === "cellValue" ? (
          <div className={v3.row}>
            <span className={v3.rowLabel}>Cell value is</span>
            <select
              className={styles.selectSmall}
              value={k.op}
              aria-label="Comparison"
              data-testid="cf-op"
              onChange={(e) => {
                const op = e.target.value as CmpOp;
                const n = CMP_OPS.find((o) => o.op === op)?.operands ?? 1;
                setKind({ ...k, op, formulas: n === 2 ? [k.formulas[0] ?? "0", k.formulas[1] ?? "100"] : [k.formulas[0] ?? "0"] });
              }}
            >
              {CMP_OPS.map((o) => (
                <option key={o.op} value={o.op}>
                  {o.label}
                </option>
              ))}
            </select>
            {k.formulas.map((f, i) => (
              <input
                key={i}
                className={cx(styles.input, v3.small)}
                aria-label={i === 0 ? "Value" : "Second value"}
                value={operandText(f)}
                data-testid={`cf-value-${i}`}
                onChange={(e) => setKind({ ...k, formulas: k.formulas.map((x, j) => (j === i ? (e.target.value === "" ? "" : operandFrom(e.target.value)) : x)) })}
              />
            ))}
          </div>
        ) : null}
        {k.type === "formula" ? (
          <Field label="Format cells where this formula is true (relative to the first cell)">
            <input className={cx(styles.input, styles.mono)} value={k.formula} placeholder='$G2="Overdue"' spellCheck={false} onChange={(e) => setKind({ ...k, formula: e.target.value })} data-testid="cf-formula" />
          </Field>
        ) : null}
        {k.type === "colorScale" ? (
          <>
            <div className={v3.presetRow}>
              {COLOR_SCALES.map((p) => (
                <button key={p.id} type="button" className={v3.scaleBtn} title={p.label} aria-label={p.label} style={{ background: `linear-gradient(90deg, ${p.colors.map(rgbaToHex).join(", ")})` }} onClick={() => setKind({ ...k, stops: colorScaleStops(p.colors) })} />
              ))}
            </div>
            <div className={v3.row}>
              <span className={v3.rowLabel}>Colours</span>
              <div className={styles.segmentedInline} role="radiogroup" aria-label="Stops">
                {[2, 3].map((n) => (
                  <button
                    key={n}
                    type="button"
                    role="radio"
                    aria-checked={k.stops.length === n}
                    className={k.stops.length === n ? styles.segOn : styles.seg}
                    onClick={() => setKind({ ...k, stops: colorScaleStops(n === 3 ? [k.stops[0]!.color, 0xffeb84ff, k.stops[k.stops.length - 1]!.color] : [k.stops[0]!.color, k.stops[k.stops.length - 1]!.color]) })}
                  >
                    {n}
                  </button>
                ))}
              </div>
            </div>
            {k.stops.map((s, i) => (
              <div key={i} className={v3.row}>
                <span className={v3.rowLabel}>{i === 0 ? "Lowest" : i === k.stops.length - 1 ? "Highest" : "Middle"}</span>
                {thresholdEditor(s.threshold, (t) => setKind({ ...k, stops: k.stops.map((x, j) => (j === i ? { ...x, threshold: t } : x)) }), `Stop ${i + 1}`, tKinds)}
                <ColorField label="Colour" value={s.color} onChange={(c) => c !== null && setKind({ ...k, stops: k.stops.map((x, j) => (j === i ? { ...x, color: c } : x)) })} />
              </div>
            ))}
          </>
        ) : null}
        {k.type === "dataBar" ? (
          <>
            <div className={v3.row}>
              <span className={v3.rowLabel}>Shortest bar</span>
              {thresholdEditor(k.min, (t) => setKind({ ...k, min: t }), "Minimum", ["autoMin", "min", "number", "percent", "percentile", "formula"])}
            </div>
            <div className={v3.row}>
              <span className={v3.rowLabel}>Longest bar</span>
              {thresholdEditor(k.max, (t) => setKind({ ...k, max: t }), "Maximum", ["autoMax", "max", "number", "percent", "percentile", "formula"])}
            </div>
            <div className={v3.row}>
              <ColorField label="Bar colour" value={k.color} onChange={(c) => c !== null && setKind({ ...k, color: c })} />
              <ColorField label="Negative" value={k.negativeColor} allowNone onChange={(c) => setKind({ ...k, negativeColor: c })} />
            </div>
            <div className={v3.row}>
              <Check label="Gradient fill" checked={k.gradient} onChange={(v) => setKind({ ...k, gradient: v })} />
              <Check label="Show the value" checked={k.showValue} onChange={(v) => setKind({ ...k, showValue: v })} />
            </div>
          </>
        ) : null}
        {k.type === "iconSet" ? (
          <>
            <div className={v3.row}>
              <span className={v3.rowLabel}>Icons</span>
              <select className={styles.selectSmall} value={k.set} onChange={(e) => setKind({ ...k, set: e.target.value as typeof k.set, thresholds: iconThresholds(e.target.value as typeof k.set) })}>
                {ICON_SETS.map((s) => (
                  <option key={s.set} value={s.set}>
                    {s.label}
                  </option>
                ))}
              </select>
              <IconStrip set={k.set} />
            </div>
            {k.thresholds.slice(1).map((t, i) => (
              <div key={i} className={v3.row}>
                <span className={v3.rowLabel}>Icon {i + 2} from</span>
                {thresholdEditor(t, (nt) => setKind({ ...k, thresholds: k.thresholds.map((x, j) => (j === i + 1 ? nt : x)) }), `Threshold ${i + 2}`, ["percent", "number", "percentile", "formula"])}
              </div>
            ))}
            <div className={v3.row}>
              <Check label="Reverse order" checked={k.reverse} onChange={(v) => setKind({ ...k, reverse: v })} />
              <Check label="Show the value" checked={k.showValue} onChange={(v) => setKind({ ...k, showValue: v })} />
            </div>
          </>
        ) : null}
        {k.type === "top" ? (
          <div className={v3.row}>
            <div className={styles.segmentedInline} role="radiogroup" aria-label="Top or bottom">
              {[false, true].map((b) => (
                <button key={String(b)} type="button" role="radio" aria-checked={k.bottom === b} className={k.bottom === b ? styles.segOn : styles.seg} onClick={() => setKind({ ...k, bottom: b })}>
                  {b ? "Bottom" : "Top"}
                </button>
              ))}
            </div>
            <input className={cx(styles.input, v3.small)} aria-label="Rank" inputMode="numeric" value={String(k.rank)} onChange={(e) => setKind({ ...k, rank: Math.floor(Number(e.target.value) || 0) })} />
            <Check label="percent" checked={k.percent} onChange={(v) => setKind({ ...k, percent: v })} />
          </div>
        ) : null}
        {k.type === "average" ? (
          <div className={v3.row}>
            <div className={styles.segmentedInline} role="radiogroup" aria-label="Above or below">
              {[true, false].map((b) => (
                <button key={String(b)} type="button" role="radio" aria-checked={k.above === b} className={k.above === b ? styles.segOn : styles.seg} onClick={() => setKind({ ...k, above: b })}>
                  {b ? "Above" : "Below"}
                </button>
              ))}
            </div>
            <span className={v3.rowLabel}>average</span>
            <Check label="or equal" checked={k.equal} onChange={(v) => setKind({ ...k, equal: v })} />
          </div>
        ) : null}
        {k.type === "text" ? (
          <div className={v3.row}>
            <span className={v3.rowLabel}>Text</span>
            <select className={styles.selectSmall} value={k.op} onChange={(e) => setKind({ ...k, op: e.target.value as typeof k.op })}>
              {TEXT_OPS.map((o) => (
                <option key={o.op} value={o.op}>
                  {o.label}
                </option>
              ))}
            </select>
            <input className={styles.input} aria-label="Text" value={k.text} onChange={(e) => setKind({ ...k, text: e.target.value })} data-testid="cf-text" />
          </div>
        ) : null}
        {k.type === "dateIs" ? (
          <div className={v3.row}>
            <span className={v3.rowLabel}>Date is</span>
            <select className={styles.selectSmall} value={k.period} onChange={(e) => setKind({ ...k, period: e.target.value as typeof k.period })}>
              {DATE_PERIODS.map((p) => (
                <option key={p.period} value={p.period}>
                  {p.label}
                </option>
              ))}
            </select>
          </div>
        ) : null}
        {["duplicate", "unique", "blank", "notBlank", "error", "notError"].includes(k.type) ? <p className={styles.note}>Formats {ruleSummary(rule).toLowerCase()} in the range.</p> : null}
      </div>

      {usesStyle(k) ? (
        <div className={v3.section}>
          <div className={v3.sectionTitle}>Format with</div>
          <div className={v3.presetRow}>
            {CF_STYLE_PRESETS.map((p) => (
              <button key={p.id} type="button" className={v3.stylePreset} title={p.label} aria-label={p.label} onClick={() => setRule((r) => ({ ...r, style: { ...p.style } }))}>
                <StylePreview rule={{ kind: { type: "blank" }, style: p.style, stopIfTrue: false }} />
              </button>
            ))}
          </div>
          <div className={v3.row}>
            {(
              [
                ["bold", "B"],
                ["italic", "I"],
                ["underline", "U"],
                ["strike", "S"],
              ] as const
            ).map(([key, glyph]) => (
              <button key={key} type="button" className={st[key] ? v3.toggleOn : v3.toggle} aria-pressed={!!st[key]} aria-label={key} onClick={() => setStyle({ [key]: st[key] ? null : true })}>
                <span className={v3[`glyph_${key}`]}>{glyph}</span>
              </button>
            ))}
            <ColorField label="Text" value={chip ? null : st.color} allowNone onChange={(c) => setStyle({ color: c, display: null })} testId="cf-font-color" />
            <ColorField label="Fill" value={chip ? null : st.fill} allowNone onChange={(c) => setStyle({ fill: c, display: null })} testId="cf-fill-color" />
          </div>
          <div className={v3.row}>
            <Check label="Show as a chip" checked={chip !== null} onChange={(v) => setStyle(v ? { display: { type: "chip", bg: st.fill ?? 0xdbeafeff, fg: st.color ?? 0x1e40afff }, fill: null, color: null } : { display: null })} testId="cf-chip" />
            {chip ? (
              <>
                <ColorField label="Chip" value={chip.bg} onChange={(c) => c !== null && setStyle({ display: { ...chip, bg: c } })} />
                <ColorField label="Chip text" value={chip.fg} onChange={(c) => c !== null && setStyle({ display: { ...chip, fg: c } })} />
              </>
            ) : null}
          </div>
          <div className={v3.previewLine}>
            <span className={styles.note}>Preview</span>
            <StylePreview rule={rule} />
          </div>
        </div>
      ) : null}
      <Check label="Stop if true (skip lower rules for matching cells)" checked={rule.stopIfTrue} onChange={(v) => setRule((r) => ({ ...r, stopIfTrue: v }))} />
      {err ? <p className={styles.error}>{err}</p> : null}
    </Shell>
  );
}

function CfPresetDialog({ d }: { d: Extract<DialogState, { kind: "cfPreset" }> }) {
  const ctl = useController();
  const preset = HIGHLIGHT_PRESETS.find((p) => p.id === d.preset)!;
  const [a, setA] = useState("");
  const [b, setB] = useState("");
  const [styleId, setStyleId] = useState(CF_STYLE_PRESETS[0]!.id);
  const ref = useAutoFocus();
  const valid = a.trim() !== "" && (preset.values < 2 || b.trim() !== "");
  const style = CF_STYLE_PRESETS.find((p) => p.id === styleId)?.style ?? CF_STYLE_PRESETS[0]!.style;
  const title = preset.label.replace(/…$/, "");
  return (
    <Shell
      title={`Highlight cells: ${title.toLowerCase()}`}
      ok="Apply"
      wide
      testId="cf-preset-dialog"
      valid={valid}
      onSubmit={() => void ctl.v3.addConditionalFormat(d.ranges, [presetRule(d.preset, [a, b], style)], "Highlight cells")}
    >
      <p className={styles.lead}>Format cells in {rangesText(d.ranges)} that are {title.toLowerCase()}:</p>
      <div className={v3.row}>
        <input ref={ref} className={styles.input} aria-label="Value" value={a} onChange={(e) => setA(e.target.value)} data-testid="cf-preset-value" placeholder={d.preset === "textContains" ? "text" : "value or cell"} />
        {preset.values === 2 ? (
          <>
            <span className={v3.rowLabel}>and</span>
            <input className={styles.input} aria-label="Second value" value={b} onChange={(e) => setB(e.target.value)} data-testid="cf-preset-value2" />
          </>
        ) : null}
      </div>
      <Field label="with" className={styles.gap}>
        <select className={styles.select} value={styleId} onChange={(e) => setStyleId(e.target.value)} data-testid="cf-preset-style">
          {CF_STYLE_PRESETS.map((p) => (
            <option key={p.id} value={p.id}>
              {p.label}
            </option>
          ))}
        </select>
      </Field>
    </Shell>
  );
}

// ---------------------------------------------------------------------------
// Data validation
// ---------------------------------------------------------------------------

type VTab = "criteria" | "help" | "alert";

function ValidationDialog({ d }: { d: Extract<DialogState, { kind: "validation" }> }) {
  const ctl = useController();
  const [tab, setTab] = useState<VTab>("criteria");
  const [v, setV] = useState<ValidationDto>(d.validation);
  const [itemsText, setItemsText] = useState(() => (d.validation.kind.type === "list" && d.validation.kind.source.type === "values" ? d.validation.kind.source.items.join("\n") : ""));
  const [rangesInput, setRangesInput] = useState(rangesText(d.validation.ranges));
  const k = v.kind;
  const set = (p: Partial<ValidationDto>) => setV((x) => ({ ...x, ...p }));
  const parsedRanges = parseRanges(rangesInput);
  const effective: ValidationDto = { ...v, ranges: parsedRanges ?? v.ranges, kind: k.type === "list" && k.source.type === "values" ? { ...k, source: { type: "values", items: listItems(itemsText) } } : k };
  const err = parsedRanges ? validationError(effective) : "Enter the cells, e.g. G2:G15.";
  const help = v.inputHelp ?? { title: "", text: "" };
  const alert = v.error;

  return (
    <Shell
      title="Data validation"
      ok={d.existing ? "Save" : "Apply"}
      wide
      cardClass={v3.xwide}
      testId="validation-dialog"
      valid={err === null}
      onSubmit={() => {
        const out: ValidationDto = {
          ...effective,
          inputHelp: help.title === "" && help.text === "" ? null : help,
        };
        void ctl.v3.applyValidation(out, d.existing);
      }}
      extra={
        d.existing ? (
          <button type="button" className={cx(styles.secondary, styles.dangerText)} onClick={() => (close(), void ctl.v3.clearValidation(v.id))} data-testid="validation-remove">
            Remove
          </button>
        ) : undefined
      }
    >
      <Tabs
        label="Validation"
        value={tab}
        onChange={setTab}
        tabs={[
          { id: "criteria", label: "Criteria" },
          { id: "help", label: "Input help" },
          { id: "alert", label: "Error alert" },
        ]}
      />
      {tab === "criteria" ? (
        <div className={v3.section}>
          <div className={v3.grid2}>
            <Field label="Cells">
              <input className={cx(styles.input, styles.mono)} value={rangesInput} spellCheck={false} onChange={(e) => setRangesInput(e.target.value)} data-testid="validation-ranges" />
            </Field>
            <Field label="Allow">
              <select className={styles.select} value={k.type} onChange={(e) => set({ kind: defaultValidationKind(e.target.value as ValidationType) })} data-testid="validation-type">
                {VALIDATION_TYPES.map((t) => (
                  <option key={t.type} value={t.type}>
                    {t.label}
                  </option>
                ))}
              </select>
            </Field>
          </div>
          {k.type === "list" ? (
            <>
              <div className={styles.segmented} role="radiogroup" aria-label="Source">
                {(
                  [
                    ["values", "Items"],
                    ["range", "From a range"],
                  ] as const
                ).map(([t, label]) => (
                  <button
                    key={t}
                    type="button"
                    role="radio"
                    aria-checked={k.source.type === t}
                    className={k.source.type === t ? styles.segOn : styles.seg}
                    onClick={() => set({ kind: { type: "list", source: t === "values" ? { type: "values", items: [] } : { type: "range", reference: "" } } })}
                    data-testid={`validation-source-${t}`}
                  >
                    {label}
                  </button>
                ))}
              </div>
              {k.source.type === "values" ? (
                <Field label="One item per line (or separated by commas)" className={styles.gap}>
                  <textarea className={cx(styles.input, v3.textarea)} rows={4} value={itemsText} onChange={(e) => setItemsText(e.target.value)} data-testid="validation-items" placeholder={"Paid\nDue\nOverdue"} />
                </Field>
              ) : (
                <Field label="Source range" className={styles.gap}>
                  <input
                    className={cx(styles.input, styles.mono)}
                    value={k.source.reference}
                    placeholder="$Lists.$A$1:$A$10"
                    spellCheck={false}
                    onChange={(e) => set({ kind: { type: "list", source: { type: "range", reference: e.target.value } } })}
                    data-testid="validation-source"
                  />
                </Field>
              )}
              <Check label="Show a dropdown in the cell" checked={v.showDropdown} onChange={(x) => set({ showDropdown: x })} />
            </>
          ) : null}
          {"formulas" in k ? (
            <div className={v3.row}>
              <span className={v3.rowLabel}>{k.type === "textLength" ? "Length" : "Value"}</span>
              <select
                className={styles.selectSmall}
                value={k.op}
                aria-label="Comparison"
                onChange={(e) => {
                  const op = e.target.value as CmpOp;
                  const n = CMP_OPS.find((o) => o.op === op)?.operands ?? 1;
                  set({ kind: { ...k, op, formulas: n === 2 ? [k.formulas[0] ?? "", k.formulas[1] ?? ""] : [k.formulas[0] ?? ""] } });
                }}
              >
                {CMP_OPS.map((o) => (
                  <option key={o.op} value={o.op}>
                    {o.label}
                  </option>
                ))}
              </select>
              {k.formulas.map((f, i) => (
                <input key={i} className={cx(styles.input, v3.small)} aria-label={i === 0 ? "Minimum" : "Maximum"} value={f} onChange={(e) => set({ kind: { ...k, formulas: k.formulas.map((x, j) => (j === i ? e.target.value : x)) } })} />
              ))}
            </div>
          ) : null}
          {k.type === "custom" ? (
            <Field label="Valid when this formula is true (relative to the first cell)" className={styles.gap}>
              <input className={cx(styles.input, styles.mono)} value={k.formula} placeholder="AND(D2>0,D2<100)" spellCheck={false} onChange={(e) => set({ kind: { ...k, formula: e.target.value } })} />
            </Field>
          ) : null}
          {k.type === "any" ? <p className={styles.note}>Any value is allowed; use the Input help tab to show a hint.</p> : null}
          <Check label="Allow empty cells" checked={v.allowBlank} onChange={(x) => set({ allowBlank: x })} />
        </div>
      ) : null}
      {tab === "help" ? (
        <div className={v3.section}>
          <p className={styles.note}>Shown next to the cell while it is selected.</p>
          <Field label="Title">
            <input className={styles.input} value={help.title} onChange={(e) => set({ inputHelp: { ...help, title: e.target.value } })} data-testid="validation-help-title" />
          </Field>
          <Field label="Message" className={styles.gap}>
            <textarea className={cx(styles.input, v3.textarea)} rows={3} value={help.text} onChange={(e) => set({ inputHelp: { ...help, text: e.target.value } })} data-testid="validation-help-text" />
          </Field>
        </div>
      ) : null}
      {tab === "alert" ? (
        <div className={v3.section}>
          <Check label="Show an error alert for invalid values" checked={alert !== null} onChange={(x) => set({ error: x ? { style: "stop", title: "", text: "" } : null })} testId="validation-alert-on" />
          {alert ? (
            <>
              <div className={styles.segmented} role="radiogroup" aria-label="Style">
                {ALERT_STYLES.map((s) => (
                  <button key={s.style} type="button" role="radio" aria-checked={alert.style === s.style} className={alert.style === s.style ? styles.segOn : styles.seg} title={s.hint} onClick={() => set({ error: { ...alert, style: s.style as ValidationErrorStyle } })} data-testid={`validation-style-${s.style}`}>
                    {s.label}
                  </button>
                ))}
              </div>
              <p className={styles.note}>{ALERT_STYLES.find((s) => s.style === alert.style)?.hint}</p>
              <Field label="Title" className={styles.gap}>
                <input className={styles.input} value={alert.title} onChange={(e) => set({ error: { ...alert, title: e.target.value } })} data-testid="validation-alert-title" />
              </Field>
              <Field label="Message" className={styles.gap}>
                <textarea className={cx(styles.input, v3.textarea)} rows={3} value={alert.text} placeholder="The value you entered is not valid." onChange={(e) => set({ error: { ...alert, text: e.target.value } })} data-testid="validation-alert-text" />
              </Field>
            </>
          ) : (
            <p className={styles.note}>Invalid values are accepted silently.</p>
          )}
        </div>
      ) : null}
      {err ? <p className={styles.error}>{err}</p> : null}
    </Shell>
  );
}

function ValidationAlertDialog({ d }: { d: Extract<DialogState, { kind: "validationAlert" }> }) {
  const ctl = useController();
  return (
    <Shell
      title={d.title}
      ok={d.retry !== null && d.cell ? "Retry" : "OK"}
      testId="validation-alert"
      onSubmit={() => {
        if (d.retry !== null && d.cell) ctl.v3.retryEdit(d.cell, d.retry);
      }}
    >
      <div className={v3.alertBody}>
        <span className={v3.stopIcon} aria-hidden>
          ✕
        </span>
        <p className={styles.lead}>{d.message}</p>
      </div>
    </Shell>
  );
}

// ---------------------------------------------------------------------------
// Hyperlink
// ---------------------------------------------------------------------------

function HyperlinkDialog({ d }: { d: Extract<DialogState, { kind: "hyperlink" }> }) {
  const ctl = useController();
  const [url, setUrl] = useState(d.url);
  const [text, setText] = useState(d.text);
  const ref = useAutoFocus();
  const normalized = normalizeLink(url);
  const err = linkError(normalized);
  return (
    <Shell
      title={d.url ? "Edit hyperlink" : "Insert hyperlink"}
      ok={d.url ? "Save" : "Insert"}
      wide
      testId="hyperlink-dialog"
      valid={url.trim() !== "" && err === null}
      onSubmit={() => void ctl.v3.setLink(d.cell, normalized, text)}
      extra={
        d.url ? (
          <button type="button" className={cx(styles.secondary, styles.dangerText)} onClick={() => (close(), void ctl.v3.removeLink())}>
            Remove link
          </button>
        ) : undefined
      }
    >
      <Field label="Link to">
        <input ref={ref} className={styles.input} value={url} spellCheck={false} placeholder="https://example.com, name@example.com or #Sheet2.A1" onChange={(e) => setUrl(e.target.value)} data-testid="hyperlink-url" />
      </Field>
      {err ? <p className={styles.error}>{err}</p> : normalized !== url.trim() && url.trim() !== "" ? <p className={styles.note}>Saved as {normalized}</p> : null}
      <Field label="Text to display" className={styles.gap}>
        <input className={styles.input} value={text} placeholder={d.hasText ? "" : "(the link)"} onChange={(e) => setText(e.target.value)} data-testid="hyperlink-text" />
      </Field>
      <p className={styles.note}>⌘-click the cell, or click its ↗, to open the link.</p>
    </Shell>
  );
}

// ---------------------------------------------------------------------------
// Protection
// ---------------------------------------------------------------------------

function ProtectSheetDialog() {
  const ctl = useController();
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [opts, setOpts] = useState<ProtectionOptions>({ ...DEFAULT_PROTECTION });
  const ref = useAutoFocus();
  const mismatch = password !== confirm;
  return (
    <Shell title={`Protect “${ctl.sheetName()}”`} ok="Protect" wide testId="protect-sheet-dialog" valid={!mismatch} onSubmit={() => void ctl.v3.protectSheet(password, opts)}>
      <p className={styles.lead}>Locked cells (all cells, unless unlocked) can't be changed while the sheet is protected.</p>
      <div className={v3.grid2}>
        <Field label="Password (optional)">
          <input ref={ref} type="password" className={styles.input} value={password} autoComplete="new-password" onChange={(e) => setPassword(e.target.value)} data-testid="protect-password" />
        </Field>
        <Field label="Confirm">
          <input type="password" className={styles.input} value={confirm} autoComplete="new-password" onChange={(e) => setConfirm(e.target.value)} data-testid="protect-confirm" />
        </Field>
      </div>
      {mismatch && confirm !== "" ? <p className={styles.error}>The passwords don't match.</p> : null}
      <div className={v3.sectionTitle}>Allow everyone to</div>
      <div className={styles.optionGrid}>
        {PROTECTION_LABELS.map((o) => (
          <Check key={o.key} label={o.label} checked={opts[o.key]} onChange={(x) => setOpts((p) => ({ ...p, [o.key]: x }))} />
        ))}
      </div>
      <p className={styles.note}>Protection keeps honest people from mistakes; the file is not encrypted.</p>
    </Shell>
  );
}

function UnprotectSheetDialog() {
  const ctl = useController();
  const [password, setPassword] = useState("");
  const [wrong, setWrong] = useState(false);
  const ref = useAutoFocus();
  return (
    <Shell
      title={`Unprotect “${ctl.sheetName()}”`}
      ok="Unprotect"
      testId="unprotect-sheet-dialog"
      keepOpen
      valid={password !== ""}
      onSubmit={() =>
        void ctl.v3.unprotectSheet(password).then((ok) => {
          if (ok) {
            close();
            ctl.focusGrid();
          } else setWrong(true);
        })
      }
    >
      <Field label="Password">
        <input ref={ref} type="password" className={styles.input} value={password} onChange={(e) => (setPassword(e.target.value), setWrong(false))} data-testid="unprotect-password" />
      </Field>
      {wrong ? <p className={styles.error}>That password is not right.</p> : null}
    </Shell>
  );
}

function ProtectDocumentDialog({ d }: { d: Extract<DialogState, { kind: "protectDocument" }> }) {
  const ctl = useController();
  const [password, setPassword] = useState("");
  const [wrong, setWrong] = useState(false);
  const ref = useAutoFocus();
  return (
    <Shell
      title={d.protected ? "Unprotect document" : "Protect document"}
      ok={d.protected ? "Unprotect" : "Protect"}
      testId="protect-document-dialog"
      keepOpen
      valid={!(d.protected && d.hasPassword && password === "")}
      onSubmit={() =>
        void ctl.v3.setDocumentProtection(!d.protected, password || null).then((ok) => {
          if (ok) {
            close();
            ctl.focusGrid();
          } else setWrong(true);
        })
      }
    >
      <p className={styles.lead}>{d.protected ? "Sheets can't be added, deleted, renamed or moved while the structure is protected." : "Protects the structure: sheets can't be added, deleted, renamed or moved."}</p>
      {!d.protected || d.hasPassword ? (
        <Field label={d.protected ? "Password" : "Password (optional)"}>
          <input ref={ref} type="password" className={styles.input} value={password} onChange={(e) => (setPassword(e.target.value), setWrong(false))} data-testid="protect-document-password" />
        </Field>
      ) : null}
      {wrong ? <p className={styles.error}>That password is not right.</p> : null}
    </Shell>
  );
}

// ---------------------------------------------------------------------------
// Goal seek
// ---------------------------------------------------------------------------

function GoalSeekDialog({ d }: { d: Extract<DialogState, { kind: "goalSeek" }> }) {
  const ctl = useController();
  const [formulaCell, setFormulaCell] = useState(d.formulaCell);
  const [target, setTarget] = useState(d.target);
  const [variableCell, setVariableCell] = useState(d.variableCell);
  const [result, setResult] = useState<{ res: GoalSeekResult; variable: CellPos } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const ref = useAutoFocus();
  const f = parseCell(formulaCell);
  const v = parseCell(variableCell);
  const t = parseNumber(target);
  const valid = f !== null && v !== null && t !== null && !(f.r === v.r && f.c === v.c);
  const fmt = (n: number) => String(Number(n.toPrecision(12)));
  const run = () => {
    if (!f || !v || t === null) return;
    setBusy(true);
    setError(null);
    ctl.v3.goalSeek(f, t, v).then(
      (res) => {
        setBusy(false);
        setResult({ res, variable: v });
      },
      (e: unknown) => {
        setBusy(false);
        setResult(null);
        setError(typeof e === "object" && e && "message" in e ? String((e as { message: unknown }).message) : "Goal seek failed.");
      },
    );
  };
  return (
    <Shell
      title="Goal seek"
      ok={result ? "Apply" : busy ? "Seeking…" : "Seek"}
      wide
      testId="goal-seek-dialog"
      keepOpen={!result}
      valid={valid && !busy}
      onSubmit={() => {
        if (result) void ctl.v3.applyGoalSeek(result.variable, result.res.value);
        else run();
      }}
    >
      <p className={styles.lead}>Changes one cell until a formula gives the value you want.</p>
      <div className={v3.grid3}>
        <Field label="Formula cell">
          <input ref={ref} className={cx(styles.input, styles.mono)} value={formulaCell} onChange={(e) => (setFormulaCell(e.target.value), setResult(null))} data-testid="goal-formula" />
        </Field>
        <Field label="Target value">
          <input className={styles.input} inputMode="decimal" value={target} onChange={(e) => (setTarget(e.target.value), setResult(null))} data-testid="goal-target" />
        </Field>
        <Field label="By changing cell">
          <input className={cx(styles.input, styles.mono)} value={variableCell} placeholder="D2" onChange={(e) => (setVariableCell(e.target.value), setResult(null))} data-testid="goal-variable" />
        </Field>
      </div>
      {error ? <p className={styles.error}>{error}</p> : null}
      {result ? (
        <div className={cx(v3.result, !result.res.found && v3.resultWarn)} data-testid="goal-result">
          <div className={v3.resultTitle}>{result.res.found ? "Found a solution" : "No exact solution"}</div>
          <div>
            {variableCell.toUpperCase()} = <b>{fmt(result.res.value)}</b> gives {formulaCell.toUpperCase()} = <b>{fmt(result.res.result)}</b>
          </div>
          <div className={styles.note}>
            {result.res.iterations} iteration{result.res.iterations === 1 ? "" : "s"}. Apply writes the value into {variableCell.toUpperCase()} (⌘Z undoes it).
          </div>
        </div>
      ) : null}
    </Shell>
  );
}

// ---------------------------------------------------------------------------
// Document properties
// ---------------------------------------------------------------------------

function PropertiesDialog({ d }: { d: Extract<DialogState, { kind: "properties" }> }) {
  const ctl = useController();
  const [p, setP] = useState<DocPropertiesDto>(d.props);
  const [keywords, setKeywords] = useState(d.props.keywords.join(", "));
  const ref = useAutoFocus();
  const set = (x: Partial<DocPropertiesDto>) => setP((o) => ({ ...o, ...x }));
  const text = (s: string) => (s.trim() === "" ? null : s);
  const date = (iso: string | null) => (iso ? new Date(iso).toLocaleString() : "—");
  return (
    <Shell
      title="Document properties"
      ok="Save"
      wide
      testId="properties-dialog"
      onSubmit={() =>
        void ctl.v3.saveProperties({
          ...p,
          keywords: keywords
            .split(",")
            .map((k) => k.trim())
            .filter((k) => k !== ""),
        })
      }
    >
      <Field label="Title">
        <input ref={ref} className={styles.input} value={p.title ?? ""} onChange={(e) => set({ title: text(e.target.value) })} data-testid="props-title" />
      </Field>
      <div className={cx(v3.grid2, styles.gap)}>
        <Field label="Subject">
          <input className={styles.input} value={p.subject ?? ""} onChange={(e) => set({ subject: text(e.target.value) })} />
        </Field>
        <Field label="Author">
          <input className={styles.input} value={p.author ?? ""} onChange={(e) => set({ author: text(e.target.value) })} data-testid="props-author" />
        </Field>
      </div>
      <Field label="Keywords (comma-separated)" className={styles.gap}>
        <input className={styles.input} value={keywords} onChange={(e) => setKeywords(e.target.value)} />
      </Field>
      <Field label="Description" className={styles.gap}>
        <textarea className={cx(styles.input, v3.textarea)} rows={3} value={p.description ?? ""} onChange={(e) => set({ description: text(e.target.value) })} />
      </Field>
      <dl className={v3.meta}>
        <dt>Created</dt>
        <dd>{date(p.created)}</dd>
        <dt>Modified</dt>
        <dd>{date(p.modified)}</dd>
        <dt>Application</dt>
        <dd>{p.generator ?? "—"}</dd>
      </dl>
    </Shell>
  );
}

// ---------------------------------------------------------------------------
// Page setup
// ---------------------------------------------------------------------------

type PTab = "page" | "margins" | "hf" | "sheet";

function PageSetupDialog({ d }: { d: Extract<DialogState, { kind: "pageSetup" }> }) {
  const ctl = useController();
  const [tab, setTab] = useState<PTab>("page");
  const [s, setS] = useState<PageSetupDto>(d.setup);
  const [printRange, setPrintRange] = useState(d.setup.printRange ? rangesText([d.setup.printRange]) : "");
  const [repeatRows, setRepeatRows] = useState(spanText(d.setup.repeatRows, "row"));
  const [repeatCols, setRepeatCols] = useState(spanText(d.setup.repeatCols, "col"));
  const set = (p: Partial<PageSetupDto>) => setS((o) => ({ ...o, ...p }));
  const pr = printRange.trim() === "" ? null : parseRanges(printRange);
  const rr = parseSpan(repeatRows, "row");
  const rc = parseSpan(repeatCols, "col");
  const err = pr === null && printRange.trim() !== "" ? "The print range is not a range (e.g. A1:G15)." : (pr?.length ?? 0) > 1 ? "Use one print range." : rr === undefined ? "Rows to repeat look like 1:2." : rc === undefined ? "Columns to repeat look like A:B." : null;
  const [w, h] = paperMm(s);
  const scaleOk = s.scale.type === "percent" ? s.scale.percent >= 10 && s.scale.percent <= 400 : s.scale.type === "fitTo" ? s.scale.width >= 0 && s.scale.height >= 0 : s.scale.pages >= 1;
  const sheetName = ctl.sheetName();
  const fileName = useDocStore((st) => st.info?.title ?? "Untitled");
  const m = s.margins;
  const setMargin = (k: keyof PageSetupDto["margins"], v: string) => {
    const n = Number(v);
    if (Number.isFinite(n) && n >= 0 && n <= 200) set({ margins: { ...m, [k]: n } });
  };
  const preview = useMemo(() => {
    const scale = 120 / Math.max(w, h);
    return { w: w * scale, h: h * scale, scale };
  }, [w, h]);
  return (
    <Shell
      title="Page setup"
      ok="Save"
      wide
      cardClass={cx(v3.xwide, v3.xxwide)}
      testId="page-setup-dialog"
      valid={err === null && scaleOk}
      onSubmit={() =>
        void ctl.v3.savePageSetup({
          ...s,
          printRange: pr?.[0] ?? null,
          repeatRows: rr ?? null,
          repeatCols: rc ?? null,
        })
      }
    >
      <Tabs
        label="Page setup"
        value={tab}
        onChange={setTab}
        tabs={[
          { id: "page", label: "Page" },
          { id: "margins", label: "Margins" },
          { id: "hf", label: "Header & footer" },
          { id: "sheet", label: "Sheet" },
        ]}
      />
      <div className={v3.pageLayout}>
        <div className={v3.pageForm}>
          {tab === "page" ? (
            <>
              <div className={v3.row}>
                <span className={v3.rowLabel}>Orientation</span>
                <div className={styles.segmentedInline} role="radiogroup" aria-label="Orientation">
                  {(["portrait", "landscape"] as const).map((o) => (
                    <button key={o} type="button" role="radio" aria-checked={s.orientation === o} className={s.orientation === o ? styles.segOn : styles.seg} onClick={() => set({ orientation: o })} data-testid={`page-${o}`}>
                      {o === "portrait" ? "Portrait" : "Landscape"}
                    </button>
                  ))}
                </div>
              </div>
              <div className={v3.row}>
                <span className={v3.rowLabel}>Paper</span>
                <select className={styles.selectSmall} value={s.paper.type} onChange={(e) => set({ paper: e.target.value === "custom" ? { type: "custom", widthMm: w, heightMm: h } : ({ type: e.target.value } as PageSetupDto["paper"]) })} data-testid="page-paper">
                  {PAPERS.map((p) => (
                    <option key={p.type} value={p.type}>
                      {p.label} ({p.mm[0]} × {p.mm[1]} mm)
                    </option>
                  ))}
                  <option value="custom">Custom…</option>
                </select>
              </div>
              {s.paper.type === "custom" ? (
                <div className={v3.row}>
                  <span className={v3.rowLabel}>Size (mm)</span>
                  <input className={cx(styles.input, v3.small)} aria-label="Width" value={s.paper.widthMm} onChange={(e) => s.paper.type === "custom" && set({ paper: { ...s.paper, widthMm: Number(e.target.value) || 0 } })} />
                  <span>×</span>
                  <input className={cx(styles.input, v3.small)} aria-label="Height" value={s.paper.heightMm} onChange={(e) => s.paper.type === "custom" && set({ paper: { ...s.paper, heightMm: Number(e.target.value) || 0 } })} />
                </div>
              ) : null}
              <div className={v3.row}>
                <span className={v3.rowLabel}>Scale</span>
                <div className={styles.segmentedInline} role="radiogroup" aria-label="Scale">
                  <button type="button" role="radio" aria-checked={s.scale.type === "percent"} className={s.scale.type === "percent" ? styles.segOn : styles.seg} onClick={() => set({ scale: { type: "percent", percent: 100 } })}>
                    Percent
                  </button>
                  <button type="button" role="radio" aria-checked={s.scale.type === "fitTo"} className={s.scale.type === "fitTo" ? styles.segOn : styles.seg} onClick={() => set({ scale: { type: "fitTo", width: 1, height: 0 } })} data-testid="page-fit">
                    Fit to pages
                  </button>
                </div>
              </div>
              <div className={v3.row}>
                <span className={v3.rowLabel} />
                {s.scale.type === "percent" ? (
                  <>
                    <input className={cx(styles.input, v3.small)} aria-label="Percent" inputMode="numeric" value={s.scale.percent} onChange={(e) => set({ scale: { type: "percent", percent: Math.round(Number(e.target.value) || 0) } })} data-testid="page-percent" />
                    <span>%</span>
                  </>
                ) : (
                  <>
                    <input className={cx(styles.input, v3.small)} aria-label="Pages wide" inputMode="numeric" value={s.scale.type === "fitTo" ? s.scale.width : 0} onChange={(e) => s.scale.type === "fitTo" && set({ scale: { ...s.scale, width: Math.max(0, Math.round(Number(e.target.value) || 0)) } })} />
                    <span>wide ×</span>
                    <input className={cx(styles.input, v3.small)} aria-label="Pages tall" inputMode="numeric" value={s.scale.type === "fitTo" ? s.scale.height : 0} onChange={(e) => s.scale.type === "fitTo" && set({ scale: { ...s.scale, height: Math.max(0, Math.round(Number(e.target.value) || 0)) } })} />
                    <span>tall (0 = any)</span>
                  </>
                )}
              </div>
              {!scaleOk ? <p className={styles.error}>Scale is 10 % to 400 %.</p> : null}
            </>
          ) : null}
          {tab === "margins" ? (
            <div className={v3.grid2}>
              {(
                [
                  ["topMm", "Top"],
                  ["bottomMm", "Bottom"],
                  ["leftMm", "Left"],
                  ["rightMm", "Right"],
                  ["headerMm", "Header"],
                  ["footerMm", "Footer"],
                ] as const
              ).map(([key, label]) => (
                <Field key={key} label={`${label} (mm)`}>
                  <input className={styles.input} inputMode="decimal" defaultValue={String(m[key])} onChange={(e) => setMargin(key, e.target.value)} data-testid={`margin-${key}`} />
                </Field>
              ))}
              <Check label="Centre horizontally" checked={s.centerH} onChange={(x) => set({ centerH: x })} />
              <Check label="Centre vertically" checked={s.centerV} onChange={(x) => set({ centerV: x })} />
            </div>
          ) : null}
          {tab === "hf" ? (
            <>
              <HeaderFooterEditor label="Header" value={s.header} onChange={(hf) => set({ header: hf })} ctx={{ sheet: sheetName, file: fileName }} testId="header" />
              <HeaderFooterEditor label="Footer" value={s.footer} onChange={(hf) => set({ footer: hf })} ctx={{ sheet: sheetName, file: fileName }} testId="footer" />
            </>
          ) : null}
          {tab === "sheet" ? (
            <>
              <Field label="Print range (empty: the used area)">
                <input className={cx(styles.input, styles.mono)} value={printRange} placeholder="A1:G15" spellCheck={false} onChange={(e) => setPrintRange(e.target.value)} data-testid="page-print-range" />
              </Field>
              <div className={cx(v3.grid2, styles.gap)}>
                <Field label="Rows to repeat">
                  <input className={cx(styles.input, styles.mono)} value={repeatRows} placeholder="1:1" onChange={(e) => setRepeatRows(e.target.value)} data-testid="page-repeat-rows" />
                </Field>
                <Field label="Columns to repeat">
                  <input className={cx(styles.input, styles.mono)} value={repeatCols} placeholder="A:A" onChange={(e) => setRepeatCols(e.target.value)} />
                </Field>
              </div>
              <div className={styles.optionGrid}>
                <Check label="Gridlines" checked={s.printGridlines} onChange={(x) => set({ printGridlines: x })} testId="page-gridlines" />
                <Check label="Row and column headers" checked={s.printHeaders} onChange={(x) => set({ printHeaders: x })} />
              </div>
              <div className={v3.row}>
                <span className={v3.rowLabel}>Page order</span>
                <div className={styles.segmentedInline} role="radiogroup" aria-label="Page order">
                  {(
                    [
                      ["topToBottom", "Down, then over"],
                      ["leftToRight", "Over, then down"],
                    ] as const
                  ).map(([o, label]) => (
                    <button key={o} type="button" role="radio" aria-checked={s.pageOrder === o} className={s.pageOrder === o ? styles.segOn : styles.seg} onClick={() => set({ pageOrder: o })}>
                      {label}
                    </button>
                  ))}
                </div>
              </div>
              <p className={styles.note}>
                Manual breaks: {s.rowBreaks.length} row, {s.colBreaks.length} column (Insert ▸ Page Break).
              </p>
            </>
          ) : null}
          {err ? <p className={styles.error}>{err}</p> : null}
        </div>
        <PagePreview setup={s} size={preview} header={s.header} footer={s.footer} ctx={{ sheet: sheetName, file: fileName }} />
      </div>
    </Shell>
  );
}

function PagePreview({ setup, size, header, footer, ctx }: { setup: PageSetupDto; size: { w: number; h: number; scale: number }; header: HeaderFooter | null; footer: HeaderFooter | null; ctx: { sheet: string; file: string } }) {
  const m = setup.margins;
  const k = size.scale;
  const hf = (x: HeaderFooter | null) => (x ? [x.left, x.center, x.right].map((t) => previewHeaderFooter(t, ctx)) : null);
  const head = hf(header);
  const foot = hf(footer);
  return (
    <div className={v3.pagePreviewWrap} aria-label="Page preview">
      <div className={v3.paper} style={{ width: size.w, height: size.h }}>
        {head ? (
          <div className={v3.hfLine} style={{ top: Math.max(2, m.headerMm * k - 4), left: m.leftMm * k, right: m.rightMm * k }}>
            {head.map((t, i) => (
              <span key={i}>{t}</span>
            ))}
          </div>
        ) : null}
        <div
          className={cx(v3.printable, setup.printGridlines && v3.printGrid)}
          style={{
            top: m.topMm * k,
            bottom: m.bottomMm * k,
            left: m.leftMm * k,
            right: m.rightMm * k,
            justifyContent: setup.centerH ? "center" : "flex-start",
            alignItems: setup.centerV ? "center" : "flex-start",
          }}
        >
          <span className={v3.printBlock} />
        </div>
        {foot ? (
          <div className={v3.hfLine} style={{ bottom: Math.max(2, m.footerMm * k - 4), left: m.leftMm * k, right: m.rightMm * k }}>
            {foot.map((t, i) => (
              <span key={i}>{t}</span>
            ))}
          </div>
        ) : null}
      </div>
      <div className={styles.note}>
        {setup.orientation === "landscape" ? "Landscape" : "Portrait"}, {setup.scale.type === "percent" ? `${setup.scale.percent} %` : setup.scale.type === "fitTo" ? `fit ${setup.scale.width || "any"} × ${setup.scale.height || "any"}` : `fit ${setup.scale.pages} pages`}
      </div>
    </div>
  );
}

function HeaderFooterEditor({ label, value, onChange, ctx, testId }: { label: string; value: HeaderFooter | null; onChange: (hf: HeaderFooter | null) => void; ctx: { sheet: string; file: string }; testId: string }) {
  const hf = value ?? { left: "", center: "", right: "" };
  const [focus, setFocus] = useState<keyof HeaderFooter>("center");
  const refs = { left: useRef<HTMLInputElement>(null), center: useRef<HTMLInputElement>(null), right: useRef<HTMLInputElement>(null) };
  const update = (k: keyof HeaderFooter, text: string) => {
    const next = { ...hf, [k]: text };
    onChange(next.left === "" && next.center === "" && next.right === "" ? null : next);
  };
  const insert = (token: string) => {
    const el = refs[focus].current;
    const cur = hf[focus];
    const res = insertToken(cur, el?.selectionStart ?? cur.length, el?.selectionEnd ?? cur.length, token);
    update(focus, res.text);
    requestAnimationFrame(() => {
      el?.focus();
      el?.setSelectionRange(res.caret, res.caret);
    });
  };
  return (
    <div className={v3.section}>
      <div className={v3.sectionTitle}>{label}</div>
      <div className={v3.grid3}>
        {(["left", "center", "right"] as const).map((k) => (
          <Field key={k} label={k === "left" ? "Left" : k === "center" ? "Centre" : "Right"}>
            <input ref={refs[k]} className={cx(styles.input, styles.mono)} value={hf[k]} onFocus={() => setFocus(k)} onChange={(e) => update(k, e.target.value)} data-testid={`${testId}-${k}`} />
          </Field>
        ))}
      </div>
      <div className={v3.presetRow}>
        {HF_PLACEHOLDERS.map((p) => (
          <button key={p.token} type="button" className={styles.preset} onMouseDown={(e) => e.preventDefault()} onClick={() => insert(p.token)} title={p.token} data-testid={`${testId}-insert-${p.label.toLowerCase()}`}>
            + {p.label}
          </button>
        ))}
      </div>
      {value ? <p className={styles.note}>Preview: {[hf.left, hf.center, hf.right].map((t) => previewHeaderFooter(t, ctx) || "—").join("  ·  ")}</p> : null}
    </div>
  );
}
