/**
 * Cell styles UI (W5-6D): the Styles panel (⌘F11: the named styles, apply on
 * click, New style from selection, edit / delete, paint-can mode), the style
 * editor, the AutoFormat dialog (17 presets with previews) and the Format
 * Cells dialog (⌘1: Numbers / Font / Font Effects / Alignment / Borders /
 * Background / Cell Protection).
 */
import type { AutoFormatOptions, CellStyleDto, StyleDto } from "@tally/bridge";
import { useMemo, useState } from "react";
import dlg from "../components/Dialogs.module.css";
import { cx, useController } from "../context";
import { a1 } from "../dataOps";
import { t } from "../i18n";
import { rgbaToHex } from "../palette";
import { useW56Store, type W56Dialog } from "../data/store";
import { Check, ColorField, Field, Tabs, W56Shell, closeW56 } from "../data/W56Shell";
import w56 from "../data/W56.module.css";
import { AUTOFORMAT_OPTION_LABELS, AUTOFORMAT_PRESETS, DEFAULT_AUTOFORMAT_OPTIONS, PREVIEW_ROWS, previewCell, previewText } from "./autoformat";
import { BORDER_EDGES, BORDER_STYLES, H_ALIGNS, UNDERLINE_STYLES, V_ALIGNS, formatCellsError, patchFromState, presetEdges, shownFamily, stateFromStyle, styleFamily, toggleEdge, withFamily, withSize, withTypeface, type FormatCellsState } from "./formatCells";
import { FontListField, SimpleList, fontMissing, previewFamily, useDocumentFont, useInstalledFonts } from "../components/FontPickers";
import pick from "../components/FontPickers.module.css";
import { APP_FONT, FONT_SIZE_LIST, TYPEFACE_LABELS, familyTypefaces, findFamily, formatFontSize, parseFontSize, typefaceOf, type Typeface } from "../fonts/fontModel";
import { useFontStore } from "../fonts/fontStore";
import { arrowNav } from "../components/dialogA11y";
import { useV3Store } from "../stores/v3Store";
import { NumbersTab, numbersPreview } from "./NumbersTab";

type D<K extends W56Dialog["kind"]> = Extract<W56Dialog, { kind: K }>;

// ---------------------------------------------------------------------------
// Styles panel
// ---------------------------------------------------------------------------

function sampleStyle(s: StyleDto): React.CSSProperties {
  return {
    fontWeight: s.bold ? 700 : 400,
    fontStyle: s.italic ? "italic" : "normal",
    textDecoration: [s.underline ? "underline" : "", s.strike ? "line-through" : ""].filter(Boolean).join(" ") || undefined,
    color: s.color === null ? undefined : rgbaToHex(s.color).slice(0, 7),
    background: s.fill === null ? undefined : rgbaToHex(s.fill).slice(0, 7),
    fontFamily: s.fontFamily ?? undefined,
  };
}

const PRESET_KEYS: Readonly<Record<string, string>> = {"Default": "stylesUi.preset.default", "3D": "stylesUi.preset.threeD", "Black 1": "stylesUi.preset.black1", "Black 2": "stylesUi.preset.black2", "Blue": "stylesUi.preset.blue", "Brown": "stylesUi.preset.brown", "Currency": "stylesUi.preset.currency", "Currency 3D": "stylesUi.preset.currency3d", "Currency Gray": "stylesUi.preset.currencyGray", "Currency Lavender": "stylesUi.preset.currencyLavender", "Currency Turquoise": "stylesUi.preset.currencyTurquoise", "Gray": "stylesUi.preset.gray", "Green": "stylesUi.preset.green", "Lavender": "stylesUi.preset.lavender", "Red": "stylesUi.preset.red", "Turquoise": "stylesUi.preset.turquoise", "Yellow": "stylesUi.preset.yellow"};
const AUTOFORMAT_OPTION_KEYS: Readonly<Record<string, string>> = { numberFormat: "toolbar.numberFormat", font: "toolbar.font", alignment: "cells.alignment", borders: "cells.borders", pattern: "stylesUi.optPattern", autofit: "stylesUi.optAutofit" };

/** Option and preset labels live in the model files in English; they are translated when drawn. */
const presetLabel = (name: string): string => t(PRESET_KEYS[name] ?? "", name);

export function StylesPanel() {
  const ctl = useController();
  const styles = useW56Store((s) => s.cellStyles);
  const paint = useW56Store((s) => s.paintStyle);
  useW56Store((s) => s.i18nRev);
  const [selected, setSelected] = useState<string>("Default");
  const sel = styles.find((s) => s.name === selected) ?? null;
  return (
    <aside className={w56.panel} aria-label={t("styles.title", "Styles")} data-testid="styles-panel">
      <div className={w56.panelHead}>
        <span className={w56.grow}>{t("styles.cellStyles", "Cell styles")}</span>
        <button type="button" className={w56.iconBtn} aria-label={t("dialog.close", "Close")} onClick={() => useW56Store.getState().setPanel(null)}>
          ×
        </button>
      </div>
      <div className={w56.panelToolbar}>
        <button type="button" className={paint ? w56.iconBtnOn : w56.iconBtn} title={t("stylesUi.paintTip", "Fill format mode: click cells to apply the selected style")} aria-pressed={paint !== null} onClick={() => ctl.w56.togglePaintCan(selected)} data-testid="styles-paint">
          🪣
        </button>
        <button type="button" className={w56.iconBtn} onClick={() => void ctl.w56.newStyleFromSelection()} data-testid="styles-new">
          {t("styles.newStyle", "New style…")}
        </button>
        {sel ? (
          <button type="button" className={w56.iconBtn} onClick={() => ctl.w56.editStyle(sel)} data-testid="styles-edit">
            {t("dialog.edit", "Edit")}
          </button>
        ) : null}
        {sel && !sel.builtin ? (
          <button type="button" className={w56.iconBtn} onClick={() => void ctl.w56.deleteCellStyle(sel.name)} data-testid="styles-delete">
            {t("dialog.delete", "Delete")}
          </button>
        ) : null}
      </div>
      <div className={w56.panelBody} role="listbox" aria-label={t("styles.cellStyles", "Cell styles")} onKeyDown={(e) => void arrowNav(e, { select: false })}>
        {styles.map((s) => (
          <button
            key={s.name}
            type="button"
            role="option"
            aria-selected={s.name === selected}
            className={s.name === selected ? w56.styleRowOn : w56.styleRow}
            onClick={() => {
              setSelected(s.name);
              void ctl.w56.applyCellStyle(s.name);
            }}
            onDoubleClick={() => ctl.w56.editStyle(s)}
            data-testid={`style-${s.name}`}
          >
            <span className={w56.styleSample} style={sampleStyle(s.style)}>
              Aa
            </span>
            <span className={w56.grow}>{s.name}</span>
            {s.parent && s.parent !== "Default" ? <span className={w56.muted}>↳ {s.parent}</span> : null}
          </button>
        ))}
        {styles.length === 0 ? <div className={cx(w56.muted, w56.navItem)}>{t("stylesUi.none", "No styles yet (the engine lists them once W5-2 lands).")}</div> : null}
      </div>
      {paint ? <div className={w56.modeBadge}>{t("stylesUi.fillMode", "Fill format: {name} (Esc stops)", { name: paint })}</div> : null}
    </aside>
  );
}

// ---------------------------------------------------------------------------
// Style editor (new / edit)
// ---------------------------------------------------------------------------

export function StyleEditorDialog({ d }: { d: D<"styleEditor"> }) {
  const ctl = useController();
  // Named styles keep `fontFamily: null` for "the document font".
  const docFont = useDocumentFont();
  const styles = useW56Store((s) => s.cellStyles);
  const [name, setName] = useState(d.style.name);
  const [parent, setParent] = useState(d.style.parent ?? "Default");
  const [st, setSt] = useState<FormatCellsState>(() => stateFromStyle(d.style.style));
  const taken = d.isNew && styles.some((s) => s.name.toLowerCase() === name.trim().toLowerCase());
  const valid = name.trim() !== "" && !taken && formatCellsError(st) === null;
  const submit = async () => {
    const patch = patchFromState(st, undefined);
    const base = d.style.style;
    const style: StyleDto = {
      ...base,
      bold: patch.bold ?? base.bold,
      italic: patch.italic ?? base.italic,
      underline: patch.underline ?? base.underline,
      strike: patch.strike ?? base.strike,
      // Empty or the document font: no family of its own (drawn with the document font).
      fontFamily: styleFamily(st.fontFamily, docFont),
      fontSizePt: st.fontSizePt,
      color: st.color,
      fill: st.fill,
      hAlign: st.hAlign,
      vAlign: st.vAlign,
      wrap: st.wrap,
      indent: st.indent,
      numberFormat: st.numberFormat.trim() === "" ? "General" : st.numberFormat.trim(),
      rotationDeg: st.rotationDeg,
      shrinkToFit: st.shrinkToFit,
      underlineStyle: st.underline ? st.underlineStyle : null,
    };
    await ctl.w56.defineCellStyle({ name: name.trim(), parent: parent === "" ? null : parent, builtin: d.style.builtin && !d.isNew, style });
  };
  return (
    <W56Shell title={d.isNew ? t("styles.newStyle", "New style…").replace(/…$/, "") : `${t("dialog.edit", "Edit")}: ${d.style.name}`} ok={d.isNew ? t("dialog.add", "Add") : t("dialog.apply", "Apply")} size="xwide" testId="style-editor" valid={valid} onSubmit={submit}>
      <div className={w56.grid2}>
        <Field label={t("dialog.name", "Name")}>
          <input className={dlg.input} value={name} disabled={!d.isNew && d.style.builtin} autoFocus onChange={(e) => setName(e.target.value)} data-testid="style-name" />
        </Field>
        <Field label={t("stylesUi.inheritFrom", "Inherit from")}>
          <select className={dlg.select} value={parent} disabled={name.trim() === "Default"} onChange={(e) => setParent(e.target.value)}>
            <option value="">{t("stylesUi.inheritNone", "- none -")}</option>
            {styles.filter((s) => s.name !== d.style.name).map((s) => (
              <option key={s.name} value={s.name}>
                {s.name}
              </option>
            ))}
          </select>
        </Field>
      </div>
      {taken ? <p className={dlg.error}>{t("stylesUi.nameTaken", "A style with this name exists.")}</p> : null}
      <FormatCellsTabs st={st} setSt={setSt} showProtection={false} />
    </W56Shell>
  );
}

// ---------------------------------------------------------------------------
// AutoFormat
// ---------------------------------------------------------------------------

export function AutoFormatDialog({ d }: { d: D<"autoFormat"> }) {
  const ctl = useController();
  const [preset, setPreset] = useState(AUTOFORMAT_PRESETS[4]?.name ?? "Default");
  const [options, setOptions] = useState<AutoFormatOptions>(DEFAULT_AUTOFORMAT_OPTIONS);
  const p = AUTOFORMAT_PRESETS.find((x) => x.name === preset) ?? AUTOFORMAT_PRESETS[0]!;
  return (
    <W56Shell title={t("cmd.format.autoFormat", "AutoFormat").replace(/…$/, "")} ok={t("dialog.ok", "OK")} size="xwide" testId="autoformat-dialog" onSubmit={() => void ctl.w56.applyAutoFormat(d.range, preset, options)}>
      <p className={dlg.lead}>{t("stylesUi.autoFormatLead", "Applies a table format to {range}: the first row and column are headers, the last ones totals.", { range: a1(d.range) })}</p>
      <div style={{ display: "grid", gridTemplateColumns: "180px 1fr", gap: 12 }}>
        <div className={w56.list} role="listbox" aria-label={t("menu.format", "Format")} style={{ maxHeight: 300 }}>
          {AUTOFORMAT_PRESETS.map((x) => (
            <button key={x.name} type="button" role="option" aria-selected={x.name === preset} className={x.name === preset ? w56.listItemOn : w56.listItem} onClick={() => setPreset(x.name)} onDoubleClick={() => (closeW56(), void ctl.w56.applyAutoFormat(d.range, x.name, options))} data-testid={`autoformat-${x.name.replace(/\s+/g, "-").toLowerCase()}`}>
              <span className={w56.styleSample} style={{ background: x.header[0], borderColor: x.border }} />
              {presetLabel(x.name)}
            </button>
          ))}
        </div>
        <div>
          <table className={w56.previewTable} style={{ marginTop: 0 }} data-testid="autoformat-preview">
            <tbody>
              {PREVIEW_ROWS.map((row, r) => (
                <tr key={r}>
                  {row.map((_, c) => {
                    const cell = previewCell(p, r, c, options);
                    return (
                      <td key={c} style={{ background: cell.bg, color: cell.fg, fontWeight: cell.bold ? 700 : 400, border: options.borders ? `1px solid ${p.border}` : "1px solid transparent", textAlign: options.alignment && c > 0 ? "right" : "left" }}>
                        {previewText(p, r, c, options)}
                      </td>
                    );
                  })}
                </tr>
              ))}
            </tbody>
          </table>
          <div className={w56.section}>{t("stylesUi.formatting", "Formatting")}</div>
          <div className={dlg.optionGrid}>
            {AUTOFORMAT_OPTION_LABELS.map((o) => (
              <Check key={o.key} label={t(AUTOFORMAT_OPTION_KEYS[o.key] ?? "", o.label)} checked={options[o.key]} onChange={(v) => setOptions({ ...options, [o.key]: v })} testId={`autoformat-opt-${o.key}`} />
            ))}
          </div>
        </div>
      </div>
    </W56Shell>
  );
}

// ---------------------------------------------------------------------------
// Format Cells (⌘1)
// ---------------------------------------------------------------------------

type CellsTab = "numbers" | "font" | "effects" | "alignment" | "borders" | "background" | "protection";

export function FormatCellsDialog({ d }: { d: D<"formatCells"> }) {
  const ctl = useController();
  const docFont = useDocumentFont();
  const [st, setSt] = useState<FormatCellsState>(() => stateFromStyle(d.style, d.mixed));
  const [protection, setProtection] = useState<{ locked: boolean | null; hiddenFormula: boolean | null }>({ locked: null, hiddenFormula: null });
  const error = formatCellsError(st);
  return (
    <W56Shell title={t("cells.title", "Format Cells")} ok={t("dialog.ok", "OK")} size="xwide" testId="format-cells" valid={error === null} onSubmit={() => void ctl.w56.applyFormatCells(d.ranges, patchFromState(st, d.style, docFont), protection)}>
      <FormatCellsTabs st={st} setSt={setSt} showProtection protection={protection} setProtection={setProtection} sample={d.sample} sampleNum={d.sampleNum} />
      {error ? <p className={dlg.error}>{error}</p> : null}
    </W56Shell>
  );
}

function FormatCellsTabs({ st, setSt, showProtection, protection, setProtection, sample, sampleNum }: { st: FormatCellsState; setSt: (s: FormatCellsState) => void; showProtection: boolean; protection?: { locked: boolean | null; hiddenFormula: boolean | null }; setProtection?: (p: { locked: boolean | null; hiddenFormula: boolean | null }) => void; sample?: string; sampleNum?: number | null }) {
  const [tab, setTab] = useState<CellsTab>("numbers");
  const docFont = useDocumentFont();
  const tabs: { id: CellsTab; label: string }[] = [
    { id: "numbers", label: t("cells.numbers", "Numbers") },
    { id: "font", label: t("cells.font", "Font") },
    { id: "effects", label: t("cells.fontEffects", "Font Effects") },
    { id: "alignment", label: t("cells.alignment", "Alignment") },
    { id: "borders", label: t("cells.borders", "Borders") },
    { id: "background", label: t("cells.background", "Background") },
    ...(showProtection ? [{ id: "protection" as const, label: t("cells.protection", "Cell Protection") }] : []),
  ];
  const docTag = useV3Store((s) => s.locale);
  const shown = useMemo(() => numbersPreview(st.numberFormat, docTag, sampleNum, sample), [st.numberFormat, docTag, sampleNum, sample]);
  const previewStyle: React.CSSProperties = {
    fontWeight: st.bold && !st.indeterminate.typeface ? 700 : 400,
    fontStyle: st.italic && !st.indeterminate.typeface ? "italic" : "normal",
    textDecoration: [st.underline ? "underline" : "", st.strike ? "line-through" : ""].filter(Boolean).join(" ") || undefined,
    textDecorationStyle: st.underline ? (st.underlineStyle === "double" || st.underlineStyle === "doubleAccounting" ? "double" : st.underlineStyle === "dotted" ? "dotted" : st.underlineStyle === "dashed" ? "dashed" : st.underlineStyle === "wave" ? "wavy" : "solid") : undefined,
    color: shown.color ?? (st.color === null ? undefined : rgbaToHex(st.color).slice(0, 7)),
    background: st.fill === null ? undefined : rgbaToHex(st.fill).slice(0, 7),
    fontFamily: st.indeterminate.fontFamily ? undefined : previewFamily(shownFamily(st.fontFamily, docFont)),
    fontSize: Math.min(28, st.fontSizePt) * 1.2,
    transform: st.rotationDeg ? `rotate(${-st.rotationDeg}deg)` : undefined,
    textAlign: st.hAlign === "general" ? "left" : (st.hAlign as "left" | "center" | "right" | "justify"),
    paddingLeft: st.indent * 4,
    display: "inline-block",
    minWidth: 120,
    padding: "6px 10px",
  };
  return (
    <>
      <Tabs label={t("menu.format", "Format")} tabs={tabs} value={tab} onChange={setTab} />
      {tab === "numbers" ? <NumbersTab code={st.numberFormat} onChange={(code) => setSt({ ...st, numberFormat: code })} /> : null}
      {tab === "font" ? <FontTab st={st} setSt={setSt} sample={sample} /> : null}
      {tab === "effects" ? (
        <div className={w56.grid2}>
          <Field label={t("toolbar.fontColor", "Font colour")}>
            <ColorField value={st.color} onChange={(c) => setSt({ ...st, color: c })} allowNone testId="cells-color" />
          </Field>
          <span />
          <Check label={t("cmd.format.underline", "Underline")} checked={st.underline} onChange={(v) => setSt({ ...st, underline: v })} testId="cells-underline" />
          <Field label={t("stylesUi.underlineStyle", "Underline style")}>
            <select className={dlg.select} value={st.underlineStyle} disabled={!st.underline} onChange={(e) => setSt({ ...st, underlineStyle: e.target.value as FormatCellsState["underlineStyle"] })} data-testid="cells-underline-style">
              {UNDERLINE_STYLES.map((u) => (
                <option key={u.style} value={u.style}>
                  {t(`stylesUi.underline.${u.style}`, u.label)}
                </option>
              ))}
            </select>
          </Field>
          <Check label={t("cmd.format.strike", "Strikethrough")} checked={st.strike} onChange={(v) => setSt({ ...st, strike: v })} />
        </div>
      ) : null}
      {tab === "alignment" ? (
        <div className={w56.grid2}>
          <Field label={t("cells.horizontal", "Horizontal")}>
            <select className={dlg.select} value={st.hAlign} onChange={(e) => setSt({ ...st, hAlign: e.target.value as FormatCellsState["hAlign"] })} data-testid="cells-halign">
              {H_ALIGNS.map((h) => (
                <option key={h.align} value={h.align}>
                  {h.align === "general" ? t("styles.default", h.label) : t(`stylesUi.halign.${h.align}`, h.label)}
                </option>
              ))}
            </select>
          </Field>
          <Field label={t("cells.vertical", "Vertical")}>
            <select className={dlg.select} value={st.vAlign} onChange={(e) => setSt({ ...st, vAlign: e.target.value as FormatCellsState["vAlign"] })}>
              {V_ALIGNS.map((v) => (
                <option key={v.align} value={v.align}>
                  {t(`stylesUi.valign.${v.align}`, v.label)}
                </option>
              ))}
            </select>
          </Field>
          <Field label={t("cells.indent", "Indent")}>
            <input className={dlg.input} type="number" min={0} max={250} value={st.indent} onChange={(e) => setSt({ ...st, indent: Number(e.target.value) })} data-testid="cells-indent" />
          </Field>
          <Field label={`${t("cells.rotation", "Text orientation")} (°)`}>
            <input className={dlg.input} type="number" min={-90} max={90} value={st.rotationDeg} onChange={(e) => setSt({ ...st, rotationDeg: Number(e.target.value) })} data-testid="cells-rotation" />
          </Field>
          <Check label={t("cmd.format.wrap", "Wrap text")} checked={st.wrap} onChange={(v) => setSt({ ...st, wrap: v })} />
          <Check label={t("cells.shrinkToFit", "Shrink to fit cell size")} checked={st.shrinkToFit} onChange={(v) => setSt({ ...st, shrinkToFit: v })} testId="cells-shrink" />
          <Check label={t("stylesUi.stacked", "Vertically stacked")} checked={st.stacked} onChange={(v) => setSt({ ...st, stacked: v })} />
        </div>
      ) : null}
      {tab === "borders" ? (
        <div>
          <div className={w56.row}>
            <button type="button" className={w56.chip} onClick={() => setSt(presetEdges(st, "none"))} data-testid="cells-borders-none">
              {t("cells.noBorder", "No borders")}
            </button>
            <button type="button" className={w56.chip} onClick={() => setSt(presetEdges(st, "outer"))} data-testid="cells-borders-outer">
              {t("cells.outerBorder", "Outer border")}
            </button>
            <button type="button" className={w56.chip} onClick={() => setSt(presetEdges(st, "all"))} data-testid="cells-borders-all">
              {t("cells.allBorders", "All borders")}
            </button>
          </div>
          <div className={w56.chips}>
            {BORDER_EDGES.map((e) => {
              const v = st.borders[e.edge];
              const on = v !== undefined && v !== null;
              const off = v === null;
              return (
                <button key={e.edge} type="button" className={on ? w56.chipOn : w56.chip} aria-pressed={on} title={off ? t("stylesUi.edgeRemoved", "Removed") : on ? t("stylesUi.edgeSet", "Set") : t("stylesUi.edgeUnchanged", "Unchanged")} style={off ? { textDecoration: "line-through" } : undefined} onClick={() => setSt(toggleEdge(st, e.edge))} data-testid={`cells-border-${e.edge}`}>
                  {t(`stylesUi.edge.${e.edge}`, e.label)}
                </button>
              );
            })}
          </div>
          <div className={w56.grid3}>
            <Field label={t("cells.lineStyle", "Line style")}>
              <select className={dlg.select} value={st.borderStyle} onChange={(e) => setSt({ ...st, borderStyle: e.target.value as FormatCellsState["borderStyle"] })}>
                {BORDER_STYLES.map((b) => (
                  <option key={b.style} value={b.style}>
                    {t(`stylesUi.lineStyle.${b.style}`, b.label)}
                  </option>
                ))}
              </select>
            </Field>
            <Field label={t("stylesUi.widthPx", "Width (px)")}>
              <input className={dlg.input} type="number" min={1} max={8} value={st.borderWidth} onChange={(e) => setSt({ ...st, borderWidth: Math.max(1, Math.min(8, Number(e.target.value) || 1)) })} />
            </Field>
            <Field label={t("cells.lineColor", "Line colour")}>
              <ColorField value={st.borderColor} onChange={(c) => setSt({ ...st, borderColor: c ?? st.borderColor })} />
            </Field>
          </div>
          <p className={dlg.note}>{t("stylesUi.edgeNote", "Edges toggle between set (with the current line), removed and unchanged. The diagonals apply per cell.")}</p>
        </div>
      ) : null}
      {tab === "background" ? (
        <Field label={t("toolbar.fillColor", "Fill colour")}>
          <ColorField value={st.fill} onChange={(c) => setSt({ ...st, fill: c })} allowNone testId="cells-fill" />
        </Field>
      ) : null}
      {tab === "protection" && protection && setProtection ? (
        <div>
          <p className={dlg.lead}>{t("stylesUi.protectionNote", "Takes effect once the sheet is protected (Tools ▸ Protect Sheet…).")}</p>
          <Check label={t("cells.protected", "Protected")} checked={protection.locked ?? st.locked} onChange={(v) => setProtection({ ...protection, locked: v })} testId="cells-locked" />
          <Check label={t("cells.hideFormula", "Hide formula")} checked={protection.hiddenFormula ?? st.hiddenFormula} onChange={(v) => setProtection({ ...protection, hiddenFormula: v })} />
        </div>
      ) : null}
      {tab === "font" ? null : (
        <>
          <div className={w56.section}>{t("font.preview", "Preview")}</div>
          <div style={{ overflow: "hidden", padding: 8, border: "1px solid var(--tally-border-subtle)", borderRadius: 8 }}>
            <span style={previewStyle} data-testid="cells-preview">
              {shown.text}
            </span>
          </div>
        </>
      )}
    </>
  );
}

// ---------------------------------------------------------------------------
// Format Cells ▸ Font (AOO: Font, Typeface, Size, preview)
// ---------------------------------------------------------------------------

const TYPEFACE_KEYS: Readonly<Record<Typeface, string>> = { regular: "font.regular", italic: "font.italic", bold: "font.bold", boldItalic: "font.boldItalic" };

function FontTab({ st, setSt, sample }: { st: FormatCellsState; setSt: (s: FormatCellsState) => void; sample?: string }) {
  const families = useInstalledFonts();
  const loaded = useFontStore((s) => s.loaded);
  const ctl = useController();
  const docFont = useDocumentFont();
  // The family meant ("" = the document font); the field shows what is typed.
  const family = st.indeterminate.fontFamily ? "" : shownFamily(st.fontFamily, docFont);
  const info = family === "" ? undefined : findFamily(family, families);
  const faces = familyTypefaces(info);
  const typeface = st.indeterminate.typeface ? null : typefaceOf(st.bold, st.italic);
  const [sizeText, setSizeText] = useState<string>(st.indeterminate.fontSizePt ? "" : formatFontSize(st.fontSizePt));
  const missing = loaded && family !== "" && fontMissing(family, families);
  return (
    <div>
      <div className={pick.fontGrid}>
        <div className={pick.column}>
          <span>{t("toolbar.font", "Font")}</span>
          <FontListField value={family} testId="cells-font" onChange={(name) => setSt(withFamily(st, name))} />
        </div>
        <div className={pick.column}>
          <span>{t("font.typeface", "Typeface")}</span>
          <div className={pick.field}>
            <input className={pick.fieldInput} readOnly tabIndex={-1} aria-hidden="true" value={typeface ? t(TYPEFACE_KEYS[typeface], TYPEFACE_LABELS[typeface]) : ""} />
            <SimpleList
              label={t("font.typeface", "Typeface")}
              testId="cells-typeface"
              value={typeface}
              items={faces.faces.map((f) => ({
                value: f.typeface,
                label: t(TYPEFACE_KEYS[f.typeface], TYPEFACE_LABELS[f.typeface]),
                hint: f.real ? undefined : t("font.synthetic", "synthesised"),
                style: { fontFamily: family ? previewFamily(family) : undefined, fontWeight: f.typeface.startsWith("bold") ? 700 : 400, fontStyle: f.typeface === "italic" || f.typeface === "boldItalic" ? "italic" : "normal" },
              }))}
              onPick={(tf) => setSt(withTypeface(st, tf))}
            />
          </div>
        </div>
        <div className={pick.column}>
          <span>{t("toolbar.fontSize", "Font size")}</span>
          <div className={pick.field}>
            <input
              className={pick.fieldInput}
              inputMode="decimal"
              aria-label={t("toolbar.fontSize", "Font size")}
              value={sizeText}
              data-testid="cells-font-size"
              onChange={(e) => {
                setSizeText(e.target.value);
                const pt = parseFontSize(e.target.value);
                setSt(withSize(st, pt ?? Number.NaN));
              }}
            />
            <SimpleList
              label={t("toolbar.fontSize", "Font size")}
              testId="cells-font-sizes"
              value={st.indeterminate.fontSizePt ? null : (FONT_SIZE_LIST.find((x) => x === st.fontSizePt) ?? null)}
              items={FONT_SIZE_LIST.map((pt) => ({ value: pt, label: formatFontSize(pt) }))}
              onPick={(pt) => {
                setSizeText(formatFontSize(pt));
                setSt(withSize(st, pt));
              }}
            />
          </div>
        </div>
      </div>
      {faces.extra ? <p className={pick.note}>{t("font.extraWeights", "This family has other weights or widths; Tally stores Regular, Bold, Italic and Bold Italic only.")}</p> : null}
      <div className={pick.preview} data-testid="cells-font-preview">
        <span style={fontPreviewStyle(st, family)}>{(sample ?? "").trim() !== "" ? (sample ?? "").slice(0, 80) : "Tally"}</span>
      </div>
      <p className={cx(pick.note, missing && pick.warn)} data-testid="cells-font-note">
        {missing ? t("font.notInstalled", "This font is not installed; a substitute will be used.") : st.indeterminate.fontFamily ? t("font.mixed", "The selection uses several fonts; pick one to change them all.") : "\u00a0"}
        {" "}
        <button type="button" className={pick.linkBtn} onClick={() => void useFontStore.getState().load(ctl.shell as never, true)} data-testid="cells-font-refresh">
          {t("font.refresh", "Rescan installed fonts")}
        </button>
      </p>
    </div>
  );
}

/** The Font tab's preview: family, size (capped), typeface, colour and effects. */
function fontPreviewStyle(st: FormatCellsState, family: string): React.CSSProperties {
  const tf = !st.indeterminate.typeface;
  return {
    fontFamily: previewFamily(family || APP_FONT),
    fontSize: Math.min(40, Math.max(6, st.indeterminate.fontSizePt || !Number.isFinite(st.fontSizePt) ? 11 : st.fontSizePt)) * (4 / 3),
    fontWeight: tf && st.bold ? 700 : 400,
    fontStyle: tf && st.italic ? "italic" : "normal",
    textDecoration: [st.underline ? "underline" : "", st.strike ? "line-through" : ""].filter(Boolean).join(" ") || undefined,
    color: st.color === null ? undefined : rgbaToHex(st.color).slice(0, 7),
    background: st.fill === null ? undefined : rgbaToHex(st.fill).slice(0, 7),
    padding: "2px 6px",
  };
}

/** Keeps the CellStyleDto import referenced for the panel's types. */
export type { CellStyleDto };
