/**
 * Tools ▸ Options ▸ View, laid out as AOO 4.1's page (sc/source/ui/optdlg/
 * tpview.cxx `ScTpContentOptions`, optdlg.src `RID_SCPAGE_CONTENT`): Visual
 * aids, Display, Objects and Window.
 *
 * Where each setting lives follows AOO: what `ScViewData::
 * WriteUserDataSequence` saves in the document (grid lines and colour,
 * zero values, comment indicator, headers, sheet tabs, outline symbols)
 * goes through `setViewOptions`; formulas and value highlighting are
 * per-sheet document settings in Tally; the rest (scroll bars, guides,
 * handles, anchor, text overflow, references in colour, object modes) are
 * app prefs, as AOO keeps them in its configuration. Page breaks is View ▸
 * Page breaks.
 */
import type { ViewOptionsDto } from "@tally/bridge";
import { useId, type ReactNode } from "react";
import dlg from "../components/Dialogs.module.css";
import { t } from "../i18n";
import { Check, Field } from "../data/W56Shell";
import w56 from "../data/W56.module.css";
import type { ObjectMode, ViewPrefs } from "./prefs";

/** AOO's standard colour table entries offered for the grid (light grey is "Automatic"). */
export const GRID_COLORS: readonly { rgb: number; key: string; label: string }[] = [
  { rgb: 0x000000, key: "od.color.black", label: "Black" },
  { rgb: 0x000080, key: "od.color.blue", label: "Blue" },
  { rgb: 0x008000, key: "od.color.green", label: "Green" },
  { rgb: 0x008080, key: "od.color.turquoise", label: "Turquoise" },
  { rgb: 0x800000, key: "od.color.red", label: "Red" },
  { rgb: 0x800080, key: "od.color.magenta", label: "Magenta" },
  { rgb: 0x808000, key: "od.color.brown", label: "Brown" },
  { rgb: 0x808080, key: "od.color.gray", label: "Gray" },
  { rgb: 0x0000ff, key: "od.color.lightBlue", label: "Light blue" },
  { rgb: 0x00ff00, key: "od.color.lightGreen", label: "Light green" },
  { rgb: 0xff0000, key: "od.color.lightRed", label: "Light red" },
  { rgb: 0xffff00, key: "od.color.yellow", label: "Yellow" },
];

const hex = (rgb: number) => `#${(rgb & 0xffffff).toString(16).padStart(6, "0")}`;

interface Props {
  view: ViewOptionsDto;
  setView(v: ViewOptionsDto): void;
  vp: ViewPrefs;
  setVp(v: Partial<ViewPrefs>): void;
  pageBreaks: boolean;
  setPageBreaks(v: boolean): void;
}

function Group({ title, children }: { title: string; children: ReactNode }) {
  const id = useId();
  return (
    <div role="group" aria-labelledby={id}>
      <div id={id} className={w56.section}>
        {title}
      </div>
      <div className={dlg.optionGrid}>{children}</div>
    </div>
  );
}

function ModeSelect({ label, value, onChange, testId }: { label: string; value: ObjectMode; onChange(v: ObjectMode): void; testId: string }) {
  return (
    <Field label={label}>
      <select className={dlg.select} value={value} onChange={(e) => onChange(e.target.value === "hide" ? "hide" : "show")} data-testid={testId}>
        <option value="show">{t("od.objShow", "Show")}</option>
        <option value="hide">{t("od.objHide", "Hide")}</option>
      </select>
    </Field>
  );
}

export function ViewTab({ view, setView, vp, setVp, pageBreaks, setPageBreaks }: Props) {
  const color = view.gridColor ?? null;
  const known = color === null || GRID_COLORS.some((c) => c.rgb === color);
  return (
    <div>
      <Group title={t("od.visualAids", "Visual aids")}>
        <Check label={t("cmd.view.gridLines", "Grid lines")} checked={view.showGridlines} onChange={(v) => setView({ ...view, showGridlines: v })} testId="opt-gridlines" />
        <Field label={t("od.gridColor", "Color")}>
          <select
            className={dlg.select}
            value={color === null ? "auto" : hex(color)}
            disabled={!view.showGridlines}
            onChange={(e) => setView({ ...view, gridColor: e.target.value === "auto" ? null : parseInt(e.target.value.slice(1), 16) })}
            data-testid="opt-grid-color"
          >
            <option value="auto">{t("od.gridColorAuto", "Automatic")}</option>
            {GRID_COLORS.map((c) => (
              <option key={c.rgb} value={hex(c.rgb)}>
                {t(c.key, c.label)}
              </option>
            ))}
            {known ? null : <option value={hex(color)}>{hex(color)}</option>}
          </select>
        </Field>
        <Check label={t("od.pageBreaks", "Page breaks")} checked={pageBreaks} onChange={setPageBreaks} testId="opt-page-breaks" />
        <Check label={t("od.helpLines", "Guides while moving")} checked={vp.helpLines} onChange={(v) => setVp({ helpLines: v })} testId="opt-help-lines" />
        <Check label={t("od.simpleHandles", "Simple handles")} checked={vp.simpleHandles} onChange={(v) => setVp({ simpleHandles: v })} testId="opt-simple-handles" />
        <Check label={t("od.bigHandles", "Large handles")} checked={vp.bigHandles} onChange={(v) => setVp({ bigHandles: v })} testId="opt-big-handles" />
      </Group>
      <Group title={t("od.display", "Display")}>
        <Check label={t("cmd.view.showFormulas", "Show formulas")} checked={view.showFormulas} onChange={(v) => setView({ ...view, showFormulas: v })} testId="opt-show-formulas" />
        <Check label={t("cmd.view.zeroValues", "Zero values")} checked={view.showZeroValues} onChange={(v) => setView({ ...view, showZeroValues: v })} />
        <Check label={t("od.commentIndicator", "Comment indicator")} checked={view.showNotes !== false} onChange={(v) => setView({ ...view, showNotes: v })} testId="opt-comment-indicator" />
        <Check label={t("cmd.view.valueHighlighting", "Value highlighting")} checked={view.valueHighlighting} onChange={(v) => setView({ ...view, valueHighlighting: v })} />
        <Check label={t("od.anchor", "Anchor")} checked={vp.anchor} onChange={(v) => setVp({ anchor: v })} testId="opt-anchor" />
        <Check label={t("od.textOverflow", "Text overflow")} checked={vp.clipMarks} onChange={(v) => setVp({ clipMarks: v })} testId="opt-clip-marks" />
        <Check label={t("od.rangeFinder", "Show references in color")} checked={vp.rangeFinder} onChange={(v) => setVp({ rangeFinder: v })} testId="opt-range-finder" />
      </Group>
      <Group title={t("od.objects", "Objects")}>
        <ModeSelect label={t("od.objGraphics", "Objects/Graphics")} value={vp.objects} onChange={(v) => setVp({ objects: v })} testId="opt-obj-graphics" />
        <ModeSelect label={t("od.objCharts", "Charts")} value={vp.charts} onChange={(v) => setVp({ charts: v })} testId="opt-obj-charts" />
        <ModeSelect label={t("od.objDrawings", "Drawing objects")} value={vp.drawings} onChange={(v) => setVp({ drawings: v })} testId="opt-obj-drawings" />
      </Group>
      <Group title={t("od.window", "Window")}>
        <Check label={t("cmd.view.headers", "Column and row headers")} checked={view.showHeaders} onChange={(v) => setView({ ...view, showHeaders: v })} />
        <Check label={t("od.hScroll", "Horizontal scroll bar")} checked={vp.hScroll} onChange={(v) => setVp({ hScroll: v })} testId="opt-hscroll" />
        <Check label={t("od.vScroll", "Vertical scroll bar")} checked={vp.vScroll} onChange={(v) => setVp({ vScroll: v })} testId="opt-vscroll" />
        <Check label={t("od.sheetTabs", "Sheet tabs")} checked={view.showSheetTabs !== false} onChange={(v) => setView({ ...view, showSheetTabs: v })} testId="opt-sheet-tabs" />
        <Check label={t("od.outlineSymbols", "Outline symbols")} checked={view.showOutlineSymbols !== false} onChange={(v) => setView({ ...view, showOutlineSymbols: v })} testId="opt-outline-symbols" />
      </Group>
      <p className={dlg.note}>
        {t(
          "od.viewNote2",
          "Formulas, value highlighting, zero values, grid lines and headers are kept per sheet; the grid colour, comment indicator, sheet tabs and outline symbols for the document. Both are saved with it. The other settings apply to every document.",
        )}
      </p>
    </div>
  );
}
