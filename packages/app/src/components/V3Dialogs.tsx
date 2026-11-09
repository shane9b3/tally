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
  HeaderFooterArea,
  PageSetupDto,
  ProtectionOptions,
  ValidationDto,
  ValidationErrorStyle,
} from "@tally/bridge";
import { drawCfIcon } from "@tally/grid";
import { useEffect, useId, useMemo, useRef, useState, type ReactNode } from "react";
import { cx, useController } from "../context";
import { useW56Store } from "../data/store";
import { t } from "../i18n";
import { sampleAndCall, useDocFormulaLang } from "../i18n/formulaNames";
import { HF_LINE_MM, hfArea, hfAreaHeightMm, pageBoxes, type HfWhich } from "../print/hfLayout";
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
  useW56Store((s) => s.i18nRev);
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
        <button key={t.id} type="button" role="tab" aria-selected={value === t.id} tabIndex={value === t.id ? 0 : -1} className={value === t.id ? v3.tabOn : v3.tab} onClick={() => onChange(t.id)} data-testid={`tab-${t.id}`}>
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
        <button type="button" className={v3.clearBtn} aria-label={t("v3.clearColor", "Clear {label}", { label })} onClick={() => onChange(null)}>
          ×
        </button>
      ) : null}
      {open ? <ColorPicker x={open.x} y={open.y} label={label} value={value} onPick={(c) => onChange(c)} onClose={() => setOpen(null)} /> : null}
    </span>
  );
}

function Check({ label, checked, onChange, testId, disabled }: { label: string; checked: boolean; onChange: (v: boolean) => void; testId?: string; disabled?: boolean }) {
  return (
    <label className={styles.checkRow}>
      <input type="checkbox" checked={checked} disabled={disabled} onChange={(e) => onChange(e.target.checked)} data-testid={testId} />
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
  // Existing rules belong to this sheet: with several sheets selected only New rule… works (it is a group edit).
  const grouped = useDocStore((s) => s.groupSheets.length > 1);
  const groupedWhy = grouped ? t("msg.reason.sheetsGrouped", "Not available with several sheets selected") : undefined;
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
      title={t("v3.cfTitle", "Conditional formatting")}
      wide
      noOk
      cardClass={v3.xwide}
      testId="cf-manager"
      onSubmit={() => undefined}
      cancelLabel={t("v3.done", "Done")}
      extra={
        <button type="button" className={styles.primary} onClick={newRule} data-testid="cf-new">
          {t("v3.newRule", "New rule…")}
        </button>
      }
    >
      {grouped ? <p className={styles.note} data-testid="cf-grouped">{groupedWhy}</p> : null}
      <p className={styles.lead}>
        {t("v3.cfLead", "Rules for “{sheet}”, highest priority first. A cell takes the first matching rule's settings; ▲▼ change the order.", { sheet: ctl.sheetName() })}
      </p>
      {list === null ? (
        <p className={styles.note}>{t("app.loading", "Loading…")}</p>
      ) : rows.length === 0 ? (
        <div className={v3.empty}>
          <p>{t("v3.noRules", "No rules on this sheet yet.")}</p>
          <p className={styles.note}>{t("v3.noRulesHint", "Select cells and choose New rule…, or use Highlight cells in the format bar.")}</p>
        </div>
      ) : (
        <div className={v3.ruleList} role="table" aria-label={t("v3.rules", "Rules")} data-testid="cf-rules">
          {rows.map((row, k) => (
            <div key={`${row.cf.id}:${row.index}`} className={v3.ruleRow} role="row" data-rule={k}>
              <span className={v3.order}>
                <button type="button" className={v3.iconBtn} aria-label={t("v3.moveUp", "Move up")} title={groupedWhy} disabled={k === 0 || grouped} onClick={() => move(row, -1)}>
                  ▲
                </button>
                <button type="button" className={v3.iconBtn} aria-label={t("v3.moveDown", "Move down")} title={groupedWhy} disabled={k === rows.length - 1 || grouped} onClick={() => move(row, 1)}>
                  ▼
                </button>
              </span>
              <StylePreview rule={row.rule} />
              <span className={v3.ruleText} role="cell">
                <span className={v3.ruleSummary}>{ruleSummary(row.rule)}</span>
                <span className={v3.ruleRanges}>{rangesText(row.cf.ranges)}</span>
              </span>
              <button type="button" className={styles.rowBtn} title={groupedWhy} disabled={grouped} data-testid={`cf-edit-${k}`} onClick={() => setEditing({ cf: row.cf, index: row.index, ranges: rangesText(row.cf.ranges), rule: structuredClone(row.rule) })}>
                {t("dialog.edit", "Edit")}
              </button>
              <button
                type="button"
                className={cx(styles.rowBtn, styles.dangerText)}
                data-testid={`cf-delete-${k}`}
                title={groupedWhy}
                disabled={grouped}
                onClick={() =>
                  after(row.cf.rules.length === 1 ? ctl.v3.deleteConditionalFormat(row.cf.id) : ctl.v3.updateConditionalFormat(row.cf.id, row.cf.ranges, row.cf.rules.filter((_, i) => i !== row.index)))
                }
              >
                {t("dialog.delete", "Delete")}
              </button>
            </div>
          ))}
        </div>
      )}
    </Shell>
  );
}

function thresholdEditor(th: CfThreshold, onChange: (th: CfThreshold) => void, label: string, kinds: readonly CfValueKind[]) {
  const needsValue = th.kind === "number" || th.kind === "percent" || th.kind === "percentile" || th.kind === "formula";
  const names: Record<CfValueKind, string> = {
    min: t("v3.lowestValue", "Lowest value"),
    max: t("v3.highestValue", "Highest value"),
    autoMin: t("v3.automatic", "Automatic"),
    autoMax: t("v3.automatic", "Automatic"),
    number: t("fmt.number", "Number"),
    percent: t("fmt.percent", "Percent"),
    percentile: t("v3.percentile", "Percentile"),
    formula: t("wizard.formula", "Formula"),
  };
  return (
    <span className={v3.thresh}>
      <select className={styles.selectSmall} aria-label={t("v3.thresholdType", "{label} type", { label })} value={th.kind} onChange={(e) => onChange({ kind: e.target.value as CfValueKind, value: th.value })}>
        {kinds.map((k) => (
          <option key={k} value={k}>
            {names[k]}
          </option>
        ))}
      </select>
      {needsValue ? <input className={cx(styles.input, v3.small)} aria-label={t("v3.thresholdValue", "{label} value", { label })} value={th.value} onChange={(e) => onChange({ ...th, value: e.target.value })} /> : null}
    </span>
  );
}

function RuleEditor({ editing, onCancel, onSave }: { editing: Editing; onCancel: () => void; onSave: (ranges: NonNullable<ReturnType<typeof parseRanges>>, rule: CfRuleDto) => void }) {
  const lid = useId();
  const [ranges, setRanges] = useState(editing.ranges);
  const [rule, setRule] = useState<CfRuleDto>(editing.rule);
  const formulaLang = useDocFormulaLang();
  const parsed = parseRanges(ranges);
  const err = parsed ? ruleError(rule) : t("v3.enterCellsCf", "Enter the cells, e.g. F2:F15.");
  const k = rule.kind;
  const setKind = (kind: CfRuleKind) => setRule((r) => ({ ...r, kind }));
  const setStyle = (p: Partial<CfStyleDto>) => setRule((r) => ({ ...r, style: { ...r.style, ...p } }));
  const st = rule.style;
  const chip = st.display?.type === "chip" ? st.display : null;
  const styleLabels = { bold: t("v3.boldLabel", "bold"), italic: t("v3.italicLabel", "italic"), underline: t("v3.underlineLabel", "underline"), strike: t("v3.strikeLabel", "strike") } as const;
  const tKinds: readonly CfValueKind[] = ["min", "max", "number", "percent", "percentile", "formula"];

  return (
    <Shell
      title={editing.cf ? t("v3.editRule", "Edit rule") : t("v3.newRuleTitle", "New rule")}
      ok={editing.cf ? t("dialog.save", "Save") : t("v3.addRule", "Add rule")}
      wide
      cardClass={v3.xwide}
      testId="cf-editor"
      keepOpen
      valid={err === null}
      onSubmit={() => parsed && onSave(parsed, rule)}
      extra={
        <button type="button" className={styles.secondary} onClick={onCancel}>
          {t("dialog.back", "Back")}
        </button>
      }
    >
      <div className={v3.grid2}>
        <Field label={t("v3.applyTo", "Apply to")}>
          <input className={cx(styles.input, styles.mono)} value={ranges} spellCheck={false} onChange={(e) => setRanges(e.target.value)} data-testid="cf-ranges" />
        </Field>
        <Field label={t("v3.ruleType", "Rule type")}>
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
            <span className={v3.rowLabel}>{t("cf.cellValue", "Cell value is")}</span>
            <select
              className={styles.selectSmall}
              value={k.op}
              aria-label={t("v3.comparison", "Comparison")}
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
                aria-label={i === 0 ? t("dialog.value", "Value") : t("v3.secondValue", "Second value")}
                value={operandText(f)}
                data-testid={`cf-value-${i}`}
                onChange={(e) => setKind({ ...k, formulas: k.formulas.map((x, j) => (j === i ? (e.target.value === "" ? "" : operandFrom(e.target.value)) : x)) })}
              />
            ))}
          </div>
        ) : null}
        {k.type === "formula" ? (
          <Field label={t("v3.cfFormulaLabel", "Format cells where this formula is true (relative to the first cell)")}>
            <input className={cx(styles.input, styles.mono)} value={k.formula} placeholder={t("v3.phCfFormula", '$G2="Overdue"')} spellCheck={false} onChange={(e) => setKind({ ...k, formula: e.target.value })} data-testid="cf-formula" />
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
              <span className={v3.rowLabel}>{t("v3.colours", "Colours")}</span>
              <div className={styles.segmentedInline} role="radiogroup" aria-label={t("v3.stops", "Stops")}>
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
                <span className={v3.rowLabel}>{i === 0 ? t("v3.lowest", "Lowest") : i === k.stops.length - 1 ? t("v3.highest", "Highest") : t("v3.middle", "Middle")}</span>
                {thresholdEditor(s.threshold, (th) => setKind({ ...k, stops: k.stops.map((x, j) => (j === i ? { ...x, threshold: th } : x)) }), t("v3.stopN", "Stop {n}", { n: i + 1 }), tKinds)}
                <ColorField label={t("v3.colour", "Colour")} value={s.color} onChange={(c) => c !== null && setKind({ ...k, stops: k.stops.map((x, j) => (j === i ? { ...x, color: c } : x)) })} />
              </div>
            ))}
          </>
        ) : null}
        {k.type === "dataBar" ? (
          <>
            <div className={v3.row}>
              <span className={v3.rowLabel}>{t("v3.shortestBar", "Shortest bar")}</span>
              {thresholdEditor(k.min, (t) => setKind({ ...k, min: t }), t("solver.minimum", "Minimum"), ["autoMin", "min", "number", "percent", "percentile", "formula"])}
            </div>
            <div className={v3.row}>
              <span className={v3.rowLabel}>{t("v3.longestBar", "Longest bar")}</span>
              {thresholdEditor(k.max, (t) => setKind({ ...k, max: t }), t("solver.maximum", "Maximum"), ["autoMax", "max", "number", "percent", "percentile", "formula"])}
            </div>
            <div className={v3.row}>
              <ColorField label={t("v3.barColour", "Bar colour")} value={k.color} onChange={(c) => c !== null && setKind({ ...k, color: c })} />
              <ColorField label={t("v3.negative", "Negative")} value={k.negativeColor} allowNone onChange={(c) => setKind({ ...k, negativeColor: c })} />
            </div>
            <div className={v3.row}>
              <Check label={t("v3.gradientFill", "Gradient fill")} checked={k.gradient} onChange={(v) => setKind({ ...k, gradient: v })} />
              <Check label={t("v3.showValue", "Show the value")} checked={k.showValue} onChange={(v) => setKind({ ...k, showValue: v })} />
            </div>
          </>
        ) : null}
        {k.type === "iconSet" ? (
          <>
            <div className={v3.row}>
              <span id={`${lid}-icons`} className={v3.rowLabel}>{t("v3.icons", "Icons")}</span>
              <select className={styles.selectSmall} aria-labelledby={`${lid}-icons`} value={k.set} onChange={(e) => setKind({ ...k, set: e.target.value as typeof k.set, thresholds: iconThresholds(e.target.value as typeof k.set) })}>
                {ICON_SETS.map((s) => (
                  <option key={s.set} value={s.set}>
                    {s.label}
                  </option>
                ))}
              </select>
              <IconStrip set={k.set} />
            </div>
            {k.thresholds.slice(1).map((th, i) => (
              <div key={i} className={v3.row}>
                <span className={v3.rowLabel}>{t("v3.iconFrom", "Icon {n} from", { n: i + 2 })}</span>
                {thresholdEditor(th, (nt) => setKind({ ...k, thresholds: k.thresholds.map((x, j) => (j === i + 1 ? nt : x)) }), t("v3.thresholdN", "Threshold {n}", { n: i + 2 }), ["percent", "number", "percentile", "formula"])}
              </div>
            ))}
            <div className={v3.row}>
              <Check label={t("v3.reverseOrder", "Reverse order")} checked={k.reverse} onChange={(v) => setKind({ ...k, reverse: v })} />
              <Check label={t("v3.showValue", "Show the value")} checked={k.showValue} onChange={(v) => setKind({ ...k, showValue: v })} />
            </div>
          </>
        ) : null}
        {k.type === "top" ? (
          <div className={v3.row}>
            <div className={styles.segmentedInline} role="radiogroup" aria-label={t("v3.topOrBottom", "Top or bottom")}>
              {[false, true].map((b) => (
                <button key={String(b)} type="button" role="radio" aria-checked={k.bottom === b} className={k.bottom === b ? styles.segOn : styles.seg} onClick={() => setKind({ ...k, bottom: b })}>
                  {b ? t("v3.bottom", "Bottom") : t("v3.top", "Top")}
                </button>
              ))}
            </div>
            <input className={cx(styles.input, v3.small)} aria-label={t("v3.rank", "Rank")} inputMode="numeric" value={String(k.rank)} onChange={(e) => setKind({ ...k, rank: Math.floor(Number(e.target.value) || 0) })} />
            <Check label={t("v3.percentLower", "percent")} checked={k.percent} onChange={(v) => setKind({ ...k, percent: v })} />
          </div>
        ) : null}
        {k.type === "average" ? (
          <div className={v3.row}>
            <div className={styles.segmentedInline} role="radiogroup" aria-label={t("v3.aboveOrBelow", "Above or below")}>
              {[true, false].map((b) => (
                <button key={String(b)} type="button" role="radio" aria-checked={k.above === b} className={k.above === b ? styles.segOn : styles.seg} onClick={() => setKind({ ...k, above: b })}>
                  {b ? t("v3.above", "Above") : t("v3.below", "Below")}
                </button>
              ))}
            </div>
            <span className={v3.rowLabel}>{t("v3.average", "average")}</span>
            <Check label={t("v3.orEqual", "or equal")} checked={k.equal} onChange={(v) => setKind({ ...k, equal: v })} />
          </div>
        ) : null}
        {k.type === "text" ? (
          <div className={v3.row}>
            <span id={`${lid}-text`} className={v3.rowLabel}>{t("v3.text", "Text")}</span>
            <select className={styles.selectSmall} aria-labelledby={`${lid}-text`} value={k.op} onChange={(e) => setKind({ ...k, op: e.target.value as typeof k.op })}>
              {TEXT_OPS.map((o) => (
                <option key={o.op} value={o.op}>
                  {o.label}
                </option>
              ))}
            </select>
            <input className={styles.input} aria-label={t("v3.text", "Text")} value={k.text} onChange={(e) => setKind({ ...k, text: e.target.value })} data-testid="cf-text" />
          </div>
        ) : null}
        {k.type === "dateIs" ? (
          <div className={v3.row}>
            <span id={`${lid}-date`} className={v3.rowLabel}>{t("v3.dateIs", "Date is")}</span>
            <select className={styles.selectSmall} aria-labelledby={`${lid}-date`} value={k.period} onChange={(e) => setKind({ ...k, period: e.target.value as typeof k.period })}>
              {DATE_PERIODS.map((p) => (
                <option key={p.period} value={p.period}>
                  {p.label}
                </option>
              ))}
            </select>
          </div>
        ) : null}
        {["duplicate", "unique", "blank", "notBlank", "error", "notError"].includes(k.type) ? <p className={styles.note}>{formatsInRange(k.type)}</p> : null}
      </div>

      {usesStyle(k) ? (
        <div className={v3.section}>
          <div className={v3.sectionTitle}>{t("v3.formatWith", "Format with")}</div>
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
              <button key={key} type="button" className={st[key] ? v3.toggleOn : v3.toggle} aria-pressed={!!st[key]} aria-label={styleLabels[key]} onClick={() => setStyle({ [key]: st[key] ? null : true })}>
                <span className={v3[`glyph_${key}`]}>{glyph}</span>
              </button>
            ))}
            <ColorField label={t("v3.text", "Text")} value={chip ? null : st.color} allowNone onChange={(c) => setStyle({ color: c, display: null })} testId="cf-font-color" />
            <ColorField label={t("v3.fill", "Fill")} value={chip ? null : st.fill} allowNone onChange={(c) => setStyle({ fill: c, display: null })} testId="cf-fill-color" />
          </div>
          <div className={v3.row}>
            <Check label={t("v3.showChip", "Show as a chip")} checked={chip !== null} onChange={(v) => setStyle(v ? { display: { type: "chip", bg: st.fill ?? 0xdbeafeff, fg: st.color ?? 0x1e40afff }, fill: null, color: null } : { display: null })} testId="cf-chip" />
            {chip ? (
              <>
                <ColorField label={t("v3.chip", "Chip")} value={chip.bg} onChange={(c) => c !== null && setStyle({ display: { ...chip, bg: c } })} />
                <ColorField label={t("v3.chipText", "Chip text")} value={chip.fg} onChange={(c) => c !== null && setStyle({ display: { ...chip, fg: c } })} />
              </>
            ) : null}
          </div>
          <div className={v3.previewLine}>
            <span className={styles.note}>{t("v3.preview", "Preview")}</span>
            <StylePreview rule={rule} />
          </div>
        </div>
      ) : null}
      <Check label={t("v3.stopIfTrue", "Stop if true (skip lower rules for matching cells)")} checked={rule.stopIfTrue} onChange={(v) => setRule((r) => ({ ...r, stopIfTrue: v }))} />
      {err ? <p className={styles.error}>{err}</p> : null}
    </Shell>
  );
}

/** "Formats duplicate values in the range." for the rule kinds that need no operands. */
function formatsInRange(type: string): string {
  switch (type) {
    case "duplicate": return t("v3.formatsInRange.duplicate", "Formats duplicate values in the range.");
    case "unique": return t("v3.formatsInRange.unique", "Formats unique values in the range.");
    case "blank": return t("v3.formatsInRange.blank", "Formats blank cells in the range.");
    case "notBlank": return t("v3.formatsInRange.notBlank", "Formats non-blank cells in the range.");
    case "error": return t("v3.formatsInRange.error", "Formats cells with errors in the range.");
    default: return t("v3.formatsInRange.notError", "Formats cells without errors in the range.");
  }
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
  return (
    <Shell
      title={preset.dialogTitle}
      ok={t("dialog.apply", "Apply")}
      wide
      testId="cf-preset-dialog"
      valid={valid}
      onSubmit={() => void ctl.v3.addConditionalFormat(d.ranges, [presetRule(d.preset, [a, b], style)], t("chrome.item.highlight", "Highlight cells"))}
    >
      <p className={styles.lead}>{preset.leadTemplate.replace("{ranges}", rangesText(d.ranges))}</p>
      <div className={v3.row}>
        <input ref={ref} className={styles.input} aria-label={t("dialog.value", "Value")} value={a} onChange={(e) => setA(e.target.value)} data-testid="cf-preset-value" placeholder={d.preset === "textContains" ? t("v3.phText", "text") : t("v3.phValueOrCell", "value or cell")} />
        {preset.values === 2 ? (
          <>
            <span className={v3.rowLabel}>{t("v3.and", "and")}</span>
            <input className={styles.input} aria-label={t("v3.secondValue", "Second value")} value={b} onChange={(e) => setB(e.target.value)} data-testid="cf-preset-value2" />
          </>
        ) : null}
      </div>
      <Field label={t("v3.with", "with")} className={styles.gap}>
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
  const formulaLang = useDocFormulaLang();
  const [v, setV] = useState<ValidationDto>(d.validation);
  const [itemsText, setItemsText] = useState(() => (d.validation.kind.type === "list" && d.validation.kind.source.type === "values" ? d.validation.kind.source.items.join("\n") : ""));
  const [rangesInput, setRangesInput] = useState(rangesText(d.validation.ranges));
  const k = v.kind;
  const set = (p: Partial<ValidationDto>) => setV((x) => ({ ...x, ...p }));
  const parsedRanges = parseRanges(rangesInput);
  const effective: ValidationDto = { ...v, ranges: parsedRanges ?? v.ranges, kind: k.type === "list" && k.source.type === "values" ? { ...k, source: { type: "values", items: listItems(itemsText) } } : k };
  const err = parsedRanges ? validationError(effective) : t("v3.enterCellsVal", "Enter the cells, e.g. G2:G15.");
  const help = v.inputHelp ?? { title: "", text: "" };
  const alert = v.error;
  // An existing rule belongs to this sheet: no editing or removing it with several sheets selected.
  const locked = useDocStore((s) => s.groupSheets.length > 1) && d.existing;
  const lockedWhy = locked ? t("msg.reason.sheetsGrouped", "Not available with several sheets selected") : undefined;

  return (
    <Shell
      title={t("v3.dvTitle", "Data validation")}
      ok={d.existing ? t("dialog.save", "Save") : t("dialog.apply", "Apply")}
      wide
      cardClass={v3.xwide}
      testId="validation-dialog"
      valid={err === null && !locked}
      onSubmit={() => {
        const out: ValidationDto = {
          ...effective,
          inputHelp: help.title === "" && help.text === "" ? null : help,
        };
        void ctl.v3.applyValidation(out, d.existing);
      }}
      extra={
        d.existing ? (
          <button type="button" className={cx(styles.secondary, styles.dangerText)} title={lockedWhy} disabled={locked} onClick={() => (close(), void ctl.v3.clearValidation(v.id))} data-testid="validation-remove">
            {t("dialog.remove", "Remove")}
          </button>
        ) : undefined
      }
    >
      {locked ? <p className={styles.note} data-testid="validation-grouped">{lockedWhy}</p> : null}
      <Tabs
        label={t("v3.validation", "Validation")}
        value={tab}
        onChange={setTab}
        tabs={[
          { id: "criteria", label: t("v3.criteria", "Criteria") },
          { id: "help", label: t("validation.inputHelp", "Input help") },
          { id: "alert", label: t("validation.errorAlert", "Error alert") },
        ]}
      />
      {tab === "criteria" ? (
        <div className={v3.section}>
          <div className={v3.grid2}>
            <Field label={t("v3.cells", "Cells")}>
              <input className={cx(styles.input, styles.mono)} value={rangesInput} spellCheck={false} onChange={(e) => setRangesInput(e.target.value)} data-testid="validation-ranges" />
            </Field>
            <Field label={t("validation.allow", "Allow")}>
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
              <div className={styles.segmented} role="radiogroup" aria-label={t("pivot.source", "Source")}>
                {(
                  [
                    ["values", t("v3.items", "Items")],
                    ["range", t("v3.fromRange", "From a range")],
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
                <Field label={t("v3.itemsHint", "One item per line (or separated by commas)")} className={styles.gap}>
                  <textarea className={cx(styles.input, v3.textarea)} rows={4} value={itemsText} onChange={(e) => setItemsText(e.target.value)} data-testid="validation-items" placeholder={t("v3.phListItems", "Paid\nDue\nOverdue")} />
                </Field>
              ) : (
                <Field label={t("v3.sourceRange", "Source range")} className={styles.gap}>
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
              <Check label={t("v3.showDropdown", "Show a dropdown in the cell")} checked={v.showDropdown} onChange={(x) => set({ showDropdown: x })} />
              {/* AOO CB_SORTLIST: only with the list shown (validate.cxx:680). */}
              <Check label={t("v3.sortEntries", "Sort entries ascending")} checked={v.sortAscending} disabled={!v.showDropdown} onChange={(x) => set({ sortAscending: x })} testId="validation-sort" />
            </>
          ) : null}
          {"formulas" in k ? (
            <div className={v3.row}>
              <span className={v3.rowLabel}>{k.type === "textLength" ? t("v3.length", "Length") : t("dialog.value", "Value")}</span>
              <select
                className={styles.selectSmall}
                value={k.op}
                aria-label={t("v3.comparison", "Comparison")}
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
                <input key={i} className={cx(styles.input, v3.small)} aria-label={i === 0 ? t("solver.minimum", "Minimum") : t("solver.maximum", "Maximum")} value={f} onChange={(e) => set({ kind: { ...k, formulas: k.formulas.map((x, j) => (j === i ? e.target.value : x)) } })} />
              ))}
            </div>
          ) : null}
          {k.type === "custom" ? (
            <Field label={t("v3.validFormula", "Valid when this formula is true (relative to the first cell)")} className={styles.gap}>
              <input className={cx(styles.input, styles.mono)} value={k.formula} placeholder={sampleAndCall(formulaLang)} spellCheck={false} onChange={(e) => set({ kind: { ...k, formula: e.target.value } })} />
            </Field>
          ) : null}
          {k.type === "any" ? <p className={styles.note}>{t("v3.anyValue", "Any value is allowed; use the Input help tab to show a hint.")}</p> : null}
          <Check label={t("validation.allowEmpty", "Allow empty cells")} checked={v.allowBlank} onChange={(x) => set({ allowBlank: x })} />
        </div>
      ) : null}
      {tab === "help" ? (
        <div className={v3.section}>
          <p className={styles.note}>{t("v3.helpShown", "Shown next to the cell while it is selected.")}</p>
          <Field label={t("properties.docTitle", "Title")}>
            <input className={styles.input} value={help.title} onChange={(e) => set({ inputHelp: { ...help, title: e.target.value } })} data-testid="validation-help-title" />
          </Field>
          <Field label={t("v3.message", "Message")} className={styles.gap}>
            <textarea className={cx(styles.input, v3.textarea)} rows={3} value={help.text} onChange={(e) => set({ inputHelp: { ...help, text: e.target.value } })} data-testid="validation-help-text" />
          </Field>
        </div>
      ) : null}
      {tab === "alert" ? (
        <div className={v3.section}>
          <Check label={t("v3.showAlert", "Show an error alert for invalid values")} checked={alert !== null} onChange={(x) => set({ error: x ? { style: "stop", title: "", text: "" } : null })} testId="validation-alert-on" />
          {alert ? (
            <>
              <div className={styles.segmented} role="radiogroup" aria-label={t("v3.style", "Style")}>
                {ALERT_STYLES.map((s) => (
                  <button key={s.style} type="button" role="radio" aria-checked={alert.style === s.style} className={alert.style === s.style ? styles.segOn : styles.seg} title={s.hint} onClick={() => set({ error: { ...alert, style: s.style as ValidationErrorStyle } })} data-testid={`validation-style-${s.style}`}>
                    {s.label}
                  </button>
                ))}
              </div>
              <p className={styles.note}>{ALERT_STYLES.find((s) => s.style === alert.style)?.hint}</p>
              <Field label={t("properties.docTitle", "Title")} className={styles.gap}>
                <input className={styles.input} value={alert.title} onChange={(e) => set({ error: { ...alert, title: e.target.value } })} data-testid="validation-alert-title" />
              </Field>
              <Field label={t("v3.message", "Message")} className={styles.gap}>
                <textarea className={cx(styles.input, v3.textarea)} rows={3} value={alert.text} placeholder={t("v3.defaultInvalid", "The value you entered is not valid.")} onChange={(e) => set({ error: { ...alert, text: e.target.value } })} data-testid="validation-alert-text" />
              </Field>
            </>
          ) : (
            <p className={styles.note}>{t("v3.invalidSilent", "Invalid values are accepted silently.")}</p>
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
      ok={d.retry !== null && d.cell ? t("v3.retry", "Retry") : t("dialog.ok", "OK")}
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
      title={d.url ? t("v3.editLink", "Edit hyperlink") : t("v3.insertLink", "Insert hyperlink")}
      ok={d.url ? t("dialog.save", "Save") : t("dialog.insert", "Insert")}
      wide
      testId="hyperlink-dialog"
      valid={url.trim() !== "" && err === null}
      onSubmit={() => void ctl.v3.setLink(d.cell, normalized, text)}
      extra={
        d.url ? (
          <button type="button" className={cx(styles.secondary, styles.dangerText)} onClick={() => (close(), void ctl.v3.removeLink())}>
            {t("v3.removeLink", "Remove link")}
          </button>
        ) : undefined
      }
    >
      <Field label={t("v3.linkTo", "Link to")}>
        <input ref={ref} className={styles.input} value={url} spellCheck={false} placeholder={t("v3.linkPlaceholder", "https://example.com, name@example.com or #Sheet2.A1")} onChange={(e) => setUrl(e.target.value)} data-testid="hyperlink-url" />
      </Field>
      {err ? <p className={styles.error}>{err}</p> : normalized !== url.trim() && url.trim() !== "" ? <p className={styles.note}>{t("v3.savedAs", "Saved as {link}", { link: normalized })}</p> : null}
      <Field label={t("v3.textToDisplay", "Text to display")} className={styles.gap}>
        <input className={styles.input} value={text} placeholder={d.hasText ? "" : t("v3.theLink", "(the link)")} onChange={(e) => setText(e.target.value)} data-testid="hyperlink-text" />
      </Field>
      <p className={styles.note}>{t("v3.linkHint", "⌘-click the cell, or click its ↗, to open the link.")}</p>
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
    <Shell title={t("v3.protectTitle", "Protect “{sheet}”", { sheet: ctl.sheetName() })} ok={t("v3.protect", "Protect")} wide testId="protect-sheet-dialog" valid={!mismatch} onSubmit={() => void ctl.v3.protectSheet(password, opts)}>
      <p className={styles.lead}>{t("v3.lockedCells", "Locked cells (all cells, unless unlocked) can't be changed while the sheet is protected.")}</p>
      <div className={v3.grid2}>
        <Field label={t("v3.passwordOptional", "Password (optional)")}>
          <input ref={ref} type="password" className={styles.input} value={password} autoComplete="new-password" onChange={(e) => setPassword(e.target.value)} data-testid="protect-password" />
        </Field>
        <Field label={t("v3.confirm", "Confirm")}>
          <input type="password" className={styles.input} value={confirm} autoComplete="new-password" onChange={(e) => setConfirm(e.target.value)} data-testid="protect-confirm" />
        </Field>
      </div>
      {mismatch && confirm !== "" ? <p className={styles.error}>{t("v3.pwMismatch", "The passwords don't match.")}</p> : null}
      <div className={v3.sectionTitle}>{t("v3.allowEveryone", "Allow everyone to")}</div>
      <div className={styles.optionGrid}>
        {PROTECTION_LABELS.map((o) => (
          <Check key={o.key} label={o.label} checked={opts[o.key]} onChange={(x) => setOpts((p) => ({ ...p, [o.key]: x }))} />
        ))}
      </div>
      <p className={styles.note}>{t("v3.protectNote", "Protection keeps honest people from mistakes; the file is not encrypted.")}</p>
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
      title={t("v3.unprotectTitle", "Unprotect “{sheet}”", { sheet: ctl.sheetName() })}
      ok={t("protect.unprotect", "Unprotect")}
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
      <Field label={t("dialog.password", "Password")}>
        <input ref={ref} type="password" className={styles.input} value={password} onChange={(e) => (setPassword(e.target.value), setWrong(false))} data-testid="unprotect-password" />
      </Field>
      {wrong ? <p className={styles.error}>{t("v3.wrongPassword", "That password is not right.")}</p> : null}
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
      title={d.protected ? t("v3.unprotectDoc", "Unprotect document") : t("v3.protectDoc", "Protect document")}
      ok={d.protected ? t("protect.unprotect", "Unprotect") : t("v3.protect", "Protect")}
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
      <p className={styles.lead}>{d.protected ? t("v3.docProtectedLead", "Sheets can't be added, deleted, renamed or moved while the structure is protected.") : t("v3.docProtectLead", "Protects the structure: sheets can't be added, deleted, renamed or moved.")}</p>
      {!d.protected || d.hasPassword ? (
        <Field label={d.protected ? t("dialog.password", "Password") : t("v3.passwordOptional", "Password (optional)")}>
          <input ref={ref} type="password" className={styles.input} value={password} onChange={(e) => (setPassword(e.target.value), setWrong(false))} data-testid="protect-document-password" />
        </Field>
      ) : null}
      {wrong ? <p className={styles.error}>{t("v3.wrongPassword", "That password is not right.")}</p> : null}
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
  const tv = parseNumber(target);
  const valid = f !== null && v !== null && tv !== null && !(f.r === v.r && f.c === v.c);
  const fmt = (n: number) => String(Number(n.toPrecision(12)));
  const run = () => {
    if (!f || !v || tv === null) return;
    setBusy(true);
    setError(null);
    ctl.v3.goalSeek(f, tv, v).then(
      (res) => {
        setBusy(false);
        setResult({ res, variable: v });
      },
      (e: unknown) => {
        setBusy(false);
        setResult(null);
        setError(typeof e === "object" && e && "message" in e ? String((e as { message: unknown }).message) : t("v3.goalFailed", "Goal seek failed."));
      },
    );
  };
  return (
    <Shell
      title={t("v3.goalTitle", "Goal seek")}
      ok={result ? t("dialog.apply", "Apply") : busy ? t("v3.seeking", "Seeking…") : t("v3.seek", "Seek")}
      wide
      testId="goal-seek-dialog"
      keepOpen={!result}
      valid={valid && !busy}
      onSubmit={() => {
        if (result) void ctl.v3.applyGoalSeek(result.variable, result.res.value);
        else run();
      }}
    >
      <p className={styles.lead}>{t("v3.goalLead", "Changes one cell until a formula gives the value you want.")}</p>
      <div className={v3.grid3}>
        <Field label={t("goalSeek.formulaCell", "Formula cell")}>
          <input ref={ref} className={cx(styles.input, styles.mono)} value={formulaCell} onChange={(e) => (setFormulaCell(e.target.value), setResult(null))} data-testid="goal-formula" />
        </Field>
        <Field label={t("goalSeek.targetValue", "Target value")}>
          <input className={styles.input} inputMode="decimal" value={target} onChange={(e) => (setTarget(e.target.value), setResult(null))} data-testid="goal-target" />
        </Field>
        <Field label={t("v3.byChanging", "By changing cell")}>
          <input className={cx(styles.input, styles.mono)} value={variableCell} placeholder="D2" onChange={(e) => (setVariableCell(e.target.value), setResult(null))} data-testid="goal-variable" />
        </Field>
      </div>
      {error ? <p className={styles.error}>{error}</p> : null}
      {result ? (
        <div className={cx(v3.result, !result.res.found && v3.resultWarn)} data-testid="goal-result">
          <div className={v3.resultTitle}>{result.res.found ? t("v3.found", "Found a solution") : t("v3.notFound", "No exact solution")}</div>
          <div>
            {variableCell.toUpperCase()} = <b>{fmt(result.res.value)}</b> {t("v3.gives", "gives")} {formulaCell.toUpperCase()} = <b>{fmt(result.res.result)}</b>
          </div>
          <div className={styles.note}>
            {result.res.iterations === 1 ? t("v3.goalNoteOne", "{n} iteration. Apply writes the value into {cell} (⌘Z undoes it).", { n: result.res.iterations, cell: variableCell.toUpperCase() }) : t("v3.goalNoteMany", "{n} iterations. Apply writes the value into {cell} (⌘Z undoes it).", { n: result.res.iterations, cell: variableCell.toUpperCase() })}
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
      title={t("v3.propsTitle", "Document properties")}
      ok={t("dialog.save", "Save")}
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
      <Field label={t("properties.docTitle", "Title")}>
        <input ref={ref} className={styles.input} value={p.title ?? ""} onChange={(e) => set({ title: text(e.target.value) })} data-testid="props-title" />
      </Field>
      <div className={cx(v3.grid2, styles.gap)}>
        <Field label={t("properties.subject", "Subject")}>
          <input className={styles.input} value={p.subject ?? ""} onChange={(e) => set({ subject: text(e.target.value) })} />
        </Field>
        <Field label={t("properties.author", "Author")}>
          <input className={styles.input} value={p.author ?? ""} onChange={(e) => set({ author: text(e.target.value) })} data-testid="props-author" />
        </Field>
      </div>
      <Field label={t("v3.keywordsHint", "Keywords (comma-separated)")} className={styles.gap}>
        <input className={styles.input} value={keywords} onChange={(e) => setKeywords(e.target.value)} />
      </Field>
      <Field label={t("v3.description", "Description")} className={styles.gap}>
        <textarea className={cx(styles.input, v3.textarea)} rows={3} value={p.description ?? ""} onChange={(e) => set({ description: text(e.target.value) })} />
      </Field>
      <dl className={v3.meta}>
        <dt>{t("properties.created", "Created")}</dt>
        <dd>{date(p.created)}</dd>
        <dt>{t("properties.modified", "Modified")}</dt>
        <dd>{date(p.modified)}</dd>
        <dt>{t("v3.application", "Application")}</dt>
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
  const lid = useId();
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
  const err = pr === null && printRange.trim() !== "" ? t("v3.errPrintRange", "The print range is not a range (e.g. A1:G15).") : (pr?.length ?? 0) > 1 ? t("v3.errOneRange", "Use one print range.") : rr === undefined ? t("v3.errRows", "Rows to repeat look like 1:2.") : rc === undefined ? t("v3.errCols", "Columns to repeat look like A:B.") : null;
  const [w, h] = paperMm(s);
  // The engine cuts oversized areas down to keep a 10 mm body; say so instead.
  const hfFits = pageBoxes(s, [w, h]).fits;
  const scaleOk = s.scale.type === "percent" ? s.scale.percent >= 10 && s.scale.percent <= 400 : s.scale.type === "fitTo" ? s.scale.width >= 0 && s.scale.height >= 0 : s.scale.pages >= 1;
  const sheetName = ctl.sheetName();
  const fileName = useDocStore((st) => st.info?.title ?? t("app.untitled", "Untitled"));
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
      title={t("v3.pageSetup", "Page setup")}
      ok={t("dialog.save", "Save")}
      wide
      cardClass={cx(v3.xwide, v3.xxwide)}
      testId="page-setup-dialog"
      valid={err === null && scaleOk && hfFits}
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
        label={t("v3.pageSetup", "Page setup")}
        value={tab}
        onChange={setTab}
        tabs={[
          { id: "page", label: t("print.page", "Page") },
          { id: "margins", label: t("print.margins", "Margins") },
          { id: "hf", label: t("v3.headerFooter", "Header & footer") },
          { id: "sheet", label: t("dialog.sheet", "Sheet") },
        ]}
      />
      <div className={v3.pageLayout}>
        <div className={v3.pageForm}>
          {tab === "page" ? (
            <>
              <div className={v3.row}>
                <span className={v3.rowLabel}>{t("print.orientation", "Orientation")}</span>
                <div className={styles.segmentedInline} role="radiogroup" aria-label={t("print.orientation", "Orientation")}>
                  {(["portrait", "landscape"] as const).map((o) => (
                    <button key={o} type="button" role="radio" aria-checked={s.orientation === o} className={s.orientation === o ? styles.segOn : styles.seg} onClick={() => set({ orientation: o })} data-testid={`page-${o}`}>
                      {o === "portrait" ? t("print.portrait", "Portrait") : t("print.landscape", "Landscape")}
                    </button>
                  ))}
                </div>
              </div>
              <div className={v3.row}>
                <span id={`${lid}-paper`} className={v3.rowLabel}>{t("v3.paper", "Paper")}</span>
                <select className={styles.selectSmall} aria-labelledby={`${lid}-paper`} value={s.paper.type} onChange={(e) => set({ paper: e.target.value === "custom" ? { type: "custom", widthMm: w, heightMm: h } : ({ type: e.target.value } as PageSetupDto["paper"]) })} data-testid="page-paper">
                  {PAPERS.map((p) => (
                    <option key={p.type} value={p.type}>
                      {p.label} ({p.mm[0]} × {p.mm[1]} mm)
                    </option>
                  ))}
                  <option value="custom">{t("v3.custom", "Custom…")}</option>
                </select>
              </div>
              {s.paper.type === "custom" ? (
                <div className={v3.row}>
                  <span className={v3.rowLabel}>{t("v3.sizeMm", "Size (mm)")}</span>
                  <input className={cx(styles.input, v3.small)} aria-label={t("v3.width", "Width")} value={s.paper.widthMm} onChange={(e) => s.paper.type === "custom" && set({ paper: { ...s.paper, widthMm: Number(e.target.value) || 0 } })} />
                  <span>×</span>
                  <input className={cx(styles.input, v3.small)} aria-label={t("print.hfHeight", "Height")} value={s.paper.heightMm} onChange={(e) => s.paper.type === "custom" && set({ paper: { ...s.paper, heightMm: Number(e.target.value) || 0 } })} />
                </div>
              ) : null}
              <div className={v3.row}>
                <span className={v3.rowLabel}>{t("print.scale", "Scale")}</span>
                <div className={styles.segmentedInline} role="radiogroup" aria-label={t("print.scale", "Scale")}>
                  <button type="button" role="radio" aria-checked={s.scale.type === "percent"} className={s.scale.type === "percent" ? styles.segOn : styles.seg} onClick={() => set({ scale: { type: "percent", percent: 100 } })}>
                    {t("fmt.percent", "Percent")}
                  </button>
                  <button type="button" role="radio" aria-checked={s.scale.type === "fitTo"} className={s.scale.type === "fitTo" ? styles.segOn : styles.seg} onClick={() => set({ scale: { type: "fitTo", width: 1, height: 0 } })} data-testid="page-fit">
                    {t("v3.fitToPages", "Fit to pages")}
                  </button>
                </div>
              </div>
              <div className={v3.row}>
                <span className={v3.rowLabel} />
                {s.scale.type === "percent" ? (
                  <>
                    <input className={cx(styles.input, v3.small)} aria-label={t("fmt.percent", "Percent")} inputMode="numeric" value={s.scale.percent} onChange={(e) => set({ scale: { type: "percent", percent: Math.round(Number(e.target.value) || 0) } })} data-testid="page-percent" />
                    <span>%</span>
                  </>
                ) : (
                  <>
                    <input className={cx(styles.input, v3.small)} aria-label={t("v3.pagesWide", "Pages wide")} inputMode="numeric" value={s.scale.type === "fitTo" ? s.scale.width : 0} onChange={(e) => s.scale.type === "fitTo" && set({ scale: { ...s.scale, width: Math.max(0, Math.round(Number(e.target.value) || 0)) } })} />
                    <span>{t("v3.wideTimes", "wide ×")}</span>
                    <input className={cx(styles.input, v3.small)} aria-label={t("v3.pagesTall", "Pages tall")} inputMode="numeric" value={s.scale.type === "fitTo" ? s.scale.height : 0} onChange={(e) => s.scale.type === "fitTo" && set({ scale: { ...s.scale, height: Math.max(0, Math.round(Number(e.target.value) || 0)) } })} />
                    <span>{t("v3.tallAny", "tall (0 = any)")}</span>
                  </>
                )}
              </div>
              {!scaleOk ? <p className={styles.error}>{t("v3.scaleRange", "Scale is 10 % to 400 %.")}</p> : null}
            </>
          ) : null}
          {tab === "margins" ? (
            <div className={v3.grid2}>
              {(
                [
                  ["topMm", t("print.marginTop", "Top")],
                  ["bottomMm", t("print.marginBottom", "Bottom")],
                  ["leftMm", t("print.marginLeft", "Left")],
                  ["rightMm", t("print.marginRight", "Right")],
                ] as const
              ).map(([key, label]) => (
                <Field key={key} label={t("v3.mmLabel", "{label} (mm)", { label })}>
                  <input className={styles.input} inputMode="decimal" defaultValue={String(m[key])} onChange={(e) => setMargin(key, e.target.value)} data-testid={`margin-${key}`} />
                </Field>
              ))}
              <Check label={t("print.centerHorizontally", "Centre horizontally")} checked={s.centerH} onChange={(x) => set({ centerH: x })} />
              <Check label={t("print.centerVertically", "Centre vertically")} checked={s.centerV} onChange={(x) => set({ centerV: x })} />
            </div>
          ) : null}
          {tab === "hf" ? (
            <>
              <HeaderFooterSection which="header" setup={s} onChange={set} ctx={{ sheet: sheetName, file: fileName }} />
              <HeaderFooterSection which="footer" setup={s} onChange={set} ctx={{ sheet: sheetName, file: fileName }} />
            </>
          ) : null}
          {tab === "sheet" ? (
            <>
              <Field label={t("v3.printRangeHint", "Print range (empty: the used area)")}>
                <input className={cx(styles.input, styles.mono)} value={printRange} placeholder="A1:G15" spellCheck={false} onChange={(e) => setPrintRange(e.target.value)} data-testid="page-print-range" />
              </Field>
              <div className={cx(v3.grid2, styles.gap)}>
                <Field label={t("print.repeatRows", "Rows to repeat")}>
                  <input className={cx(styles.input, styles.mono)} value={repeatRows} placeholder="1:1" onChange={(e) => setRepeatRows(e.target.value)} data-testid="page-repeat-rows" />
                </Field>
                <Field label={t("print.repeatColumns", "Columns to repeat")}>
                  <input className={cx(styles.input, styles.mono)} value={repeatCols} placeholder="A:A" onChange={(e) => setRepeatCols(e.target.value)} />
                </Field>
              </div>
              <div className={styles.optionGrid}>
                <Check label={t("v3.gridlines", "Gridlines")} checked={s.printGridlines} onChange={(x) => set({ printGridlines: x })} testId="page-gridlines" />
                <Check label={t("v3.rowColHeaders", "Row and column headers")} checked={s.printHeaders} onChange={(x) => set({ printHeaders: x })} />
              </div>
              <div className={v3.row}>
                <span className={v3.rowLabel}>{t("v3.pageOrder", "Page order")}</span>
                <div className={styles.segmentedInline} role="radiogroup" aria-label={t("v3.pageOrder", "Page order")}>
                  {(
                    [
                      ["topToBottom", t("v3.downThenOver", "Down, then over")],
                      ["leftToRight", t("v3.overThenDown", "Over, then down")],
                    ] as const
                  ).map(([o, label]) => (
                    <button key={o} type="button" role="radio" aria-checked={s.pageOrder === o} className={s.pageOrder === o ? styles.segOn : styles.seg} onClick={() => set({ pageOrder: o })}>
                      {label}
                    </button>
                  ))}
                </div>
              </div>
              <p className={styles.note}>
                {t("v3.manualBreaks", "Manual breaks: {rows} row, {cols} column (Insert ▸ Page Break).", { rows: s.rowBreaks.length, cols: s.colBreaks.length })}
              </p>
            </>
          ) : null}
          {err ? <p className={styles.error}>{err}</p> : null}
          {!hfFits ? (
            <p className={styles.error} data-testid="page-hf-too-tall">
              {t("print.hfTooTall", "The header and footer do not fit on the page with these margins. Reduce their height or spacing, or the page margins.")}
            </p>
          ) : null}
        </div>
        <PagePreview setup={s} size={preview} ctx={{ sheet: sheetName, file: fileName }} />
      </div>
    </Shell>
  );
}

function PagePreview({ setup, size, ctx }: { setup: PageSetupDto; size: { w: number; h: number; scale: number }; ctx: { sheet: string; file: string } }) {
  const m = setup.margins;
  const k = size.scale;
  // The engine's layout (print::body_of): header and footer areas inside
  // the margins, the body between them.
  const boxes = pageBoxes(setup, [size.w / k, size.h / k]);
  const lineH = HF_LINE_MM * k;
  const area = (which: HfWhich) => {
    const box = boxes[which];
    const text = which === "header" ? setup.header : setup.footer;
    if (!box || !text) return null;
    return (
      <div className={v3.hfArea} style={{ top: box.top * k, height: box.height * k, left: box.left * k, width: box.width * k, lineHeight: `${lineH}px`, fontSize: Math.max(3, lineH * 0.8) }} data-testid={`preview-${which}`}>
        {[text.left, text.center, text.right].map((x, i) => (
          <span key={i}>{previewHeaderFooter(x, ctx)}</span>
        ))}
      </div>
    );
  };
  return (
    <div className={v3.pagePreviewWrap} aria-label={t("v3.pagePreview", "Page preview")}>
      <div className={v3.paper} style={{ width: size.w, height: size.h }}>
        {area("header")}
        <div
          className={cx(v3.printable, setup.printGridlines && v3.printGrid)}
          style={{
            top: boxes.bodyTop * k,
            height: boxes.bodyHeight * k,
            left: m.leftMm * k,
            right: m.rightMm * k,
            justifyContent: setup.centerH ? "center" : "flex-start",
            alignItems: setup.centerV ? "center" : "flex-start",
          }}
          data-testid="preview-body"
        >
          <span className={v3.printBlock} />
        </div>
        {area("footer")}
      </div>
      <div className={styles.note}>
        {t("v3.previewNote", "{orientation}, {scale}", { orientation: setup.orientation === "landscape" ? t("print.landscape", "Landscape") : t("print.portrait", "Portrait"), scale: setup.scale.type === "percent" ? `${setup.scale.percent} %` : setup.scale.type === "fitTo" ? t("v3.fitWH", "fit {w} × {h}", { w: setup.scale.width || t("v3.any", "any"), h: setup.scale.height || t("v3.any", "any") }) : t("v3.fitPages", "fit {n} pages", { n: setup.scale.pages }) })}
      </div>
    </div>
  );
}

/** A length in mm, kept as typed while it parses to the same value. */
function MmInput({ value, onChange, testId }: { value: number; onChange: (mm: number) => void; testId: string }) {
  const fmt = (v: number) => String(Math.round(v * 100) / 100);
  const [text, setText] = useState(fmt(value));
  useEffect(() => {
    setText((cur) => (cur.trim() !== "" && Math.round(Number(cur) * 100) === Math.round(value * 100) ? cur : fmt(value)));
  }, [value]);
  return (
    <input
      className={styles.input}
      inputMode="decimal"
      value={text}
      onChange={(e) => {
        setText(e.target.value);
        const n = Number(e.target.value);
        if (e.target.value.trim() !== "" && Number.isFinite(n) && n >= 0 && n <= 200) onChange(n);
      }}
      data-testid={testId}
    />
  );
}

const EMPTY_HF: HeaderFooter = { left: "", center: "", right: "" };

/**
 * One tab half of AOO's Format ▸ Page ▸ Header / Footer: "Header on", the
 * texts, Left margin, Right margin, Spacing, Height and AutoFit height. The
 * Height shown is the area's without its spacing (as in AOO); the model's
 * `margins.headerMm` holds both. Switched off, the texts are kept.
 */
function HeaderFooterSection({ which, setup, onChange, ctx }: { which: HfWhich; setup: PageSetupDto; onChange: (p: Partial<PageSetupDto>) => void; ctx: { sheet: string; file: string } }) {
  const isHeader = which === "header";
  const text = isHeader ? setup.header : setup.footer;
  const area = hfArea(setup, which);
  const on = area.on && text !== null;
  const totalMm = isHeader ? setup.margins.headerMm : setup.margins.footerMm;
  const heightMm = Math.max(0, totalMm - area.spacingMm);
  const update = (patch: Partial<HeaderFooterArea>, newTotalMm?: number) => {
    const next = { ...area, ...patch };
    const p: Partial<PageSetupDto> = isHeader ? { headerArea: next } : { footerArea: next };
    // "Header on" without a text: an empty header (it still takes its room).
    if (next.on && text === null) Object.assign(p, isHeader ? { header: EMPTY_HF } : { footer: EMPTY_HF });
    if (newTotalMm !== undefined) p.margins = { ...setup.margins, [isHeader ? "headerMm" : "footerMm"]: newTotalMm };
    onChange(p);
  };
  return (
    <div className={v3.section} data-testid={`${which}-section`}>
      <div className={v3.sectionTitle}>{isHeader ? t("print.header", "Header") : t("print.footer", "Footer")}</div>
      <Check label={isHeader ? t("print.headerOn", "Header on") : t("print.footerOn", "Footer on")} checked={on} onChange={(x) => update({ on: x })} testId={`${which}-on`} />
      <fieldset className={v3.hfFields} disabled={!on} data-testid={`${which}-fields`}>
        <HeaderFooterEditor value={text} onChange={(hf) => onChange(isHeader ? { header: hf } : { footer: hf })} ctx={ctx} testId={which} />
        <div className={v3.grid4}>
          <Field label={t("v3.mmLabel", "{label} (mm)", { label: t("print.hfLeftMargin", "Left margin") })}>
            <MmInput value={area.leftMm} onChange={(v) => update({ leftMm: v })} testId={`${which}-left-margin`} />
          </Field>
          <Field label={t("v3.mmLabel", "{label} (mm)", { label: t("print.hfRightMargin", "Right margin") })}>
            <MmInput value={area.rightMm} onChange={(v) => update({ rightMm: v })} testId={`${which}-right-margin`} />
          </Field>
          <Field label={t("v3.mmLabel", "{label} (mm)", { label: t("print.hfSpacing", "Spacing") })}>
            <MmInput value={area.spacingMm} onChange={(v) => update({ spacingMm: v }, heightMm + v)} testId={`${which}-spacing`} />
          </Field>
          <Field label={t("v3.mmLabel", "{label} (mm)", { label: t("print.hfHeight", "Height") })}>
            <MmInput value={heightMm} onChange={(v) => update({}, v + area.spacingMm)} testId={`${which}-height`} />
          </Field>
        </div>
        <Check label={t("print.hfAutofit", "AutoFit height")} checked={area.autofit} onChange={(x) => update({ autofit: x })} testId={`${which}-autofit`} />
      </fieldset>
      {on && text && text.left === "" && text.center === "" && text.right === "" ? (
        // AOO: an empty area that is on still takes its room.
        <p className={styles.note} data-testid={`${which}-empty-note`}>
          {t("print.hfEmptyRoom", "This area is empty but still takes {mm} mm of every page. Uncheck it to give the room to the cells.", { mm: Math.round(hfAreaHeightMm(text, totalMm, area) * 10) / 10 })}
        </p>
      ) : null}
    </div>
  );
}

function HeaderFooterEditor({ value, onChange, ctx, testId }: { value: HeaderFooter | null; onChange: (hf: HeaderFooter) => void; ctx: { sheet: string; file: string }; testId: string }) {
  const hf = value ?? { left: "", center: "", right: "" };
  const [focus, setFocus] = useState<keyof HeaderFooter>("center");
  const refs = { left: useRef<HTMLTextAreaElement>(null), center: useRef<HTMLTextAreaElement>(null), right: useRef<HTMLTextAreaElement>(null) };
  const update = (k: keyof HeaderFooter, text: string) => {
    onChange({ ...hf, [k]: text });
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
    <>
      <div className={v3.grid3}>
        {(["left", "center", "right"] as const).map((k) => (
          <Field key={k} label={k === "left" ? t("print.marginLeft", "Left") : k === "center" ? t("v3.centre", "Centre") : t("print.marginRight", "Right")}>
            {/* A textarea: a region may hold several lines (an <input> would drop the line breaks). */}
            <textarea ref={refs[k]} className={cx(styles.input, styles.mono, v3.hfText)} rows={2} value={hf[k]} onFocus={() => setFocus(k)} onChange={(e) => update(k, e.target.value)} data-testid={`${testId}-${k}`} />
          </Field>
        ))}
      </div>
      <div className={v3.presetRow}>
        {HF_PLACEHOLDERS.map((p) => (
          <button key={p.token} type="button" className={styles.preset} onMouseDown={(e) => e.preventDefault()} onClick={() => insert(p.token)} title={p.token} data-testid={`${testId}-insert-${p.token.slice(2, -1).toLowerCase()}`}>
            + {p.label}
          </button>
        ))}
      </div>
      {value ? <p className={styles.note}>{t("v3.previewColon", "Preview: {text}", { text: [hf.left, hf.center, hf.right].map((x) => previewHeaderFooter(x, ctx) || "—").join("  ·  ") })}</p> : null}
    </>
  );
}
