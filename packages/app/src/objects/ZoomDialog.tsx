/**
 * View ▸ Zoom… (AOO's "Zoom & View Layout" dialog, cui/source/dialogs/zoom.cxx
 * as Calc opens it in sc/source/ui/view/tabvwsh3.cxx `FID_SCALE`): Optimal
 * (only for a marked range), Fit width and height, Fit width, 100 % and
 * Variable, limited to 20–400 % (`MINZOOM` / `MAXZOOM`). Calc leaves the
 * dialog's "View layout" part disabled; it is not shown.
 */
import { useState } from "react";
import dlg from "../components/Dialogs.module.css";
import { useController } from "../context";
import { useW56Store, type W56Dialog } from "../data/store";
import { W56Shell } from "../data/W56Shell";
import { t } from "../i18n";
import { useUiStore } from "../stores/uiStore";

type Mode = "optimal" | "wholePage" | "pageWidth" | "100" | "variable";

/** The zoom percent a Variable entry stands for: clamped to 20–400 % (AOO clamps too); `null` if it is not a number. */
export function variableZoom(text: string): number | null {
  const n = Number(text.trim().replace(/%$/, "").trim());
  if (text.trim() === "" || !Number.isFinite(n)) return null;
  return Math.min(400, Math.max(20, Math.round(n)));
}

export function ZoomDialog({ d }: { d: Extract<W56Dialog, { kind: "zoom" }> }) {
  useW56Store((s) => s.i18nRev);
  const ctl = useController();
  const current = Math.round(useUiStore.getState().zoom * 100);
  const [mode, setMode] = useState<Mode>(current === 100 ? "100" : "variable");
  const [value, setValue] = useState(String(current));
  const pct = variableZoom(value);
  const valid = mode !== "variable" || pct !== null;
  const submit = () => {
    const view = ctl.w5.view;
    switch (mode) {
      case "optimal":
        view.zoomOptimal();
        break;
      case "wholePage":
        void view.zoomToPage("both");
        break;
      case "pageWidth":
        void view.zoomToPage("width");
        break;
      case "100":
        useUiStore.getState().setZoom(1);
        break;
      case "variable":
        if (pct !== null) useUiStore.getState().setZoom(pct / 100);
        break;
    }
  };
  const options: readonly (readonly [Mode, string, boolean])[] = [
    ["optimal", t("zoomDlg.optimal", "Optimal"), d.optimal],
    ["wholePage", t("zoomDlg.wholePage", "Fit width and height"), true],
    ["pageWidth", t("zoomDlg.pageWidth", "Fit width"), true],
    ["100", "100 %", true],
  ];
  return (
    <W56Shell title={t("zoomDlg.title", "Zoom & View Layout")} ok={t("dialog.ok", "OK")} testId="zoom-dialog" valid={valid} onSubmit={submit}>
      <div role="radiogroup" aria-label={t("zoomDlg.factor", "Zoom factor")}>
        <p className={dlg.lead}>{t("zoomDlg.factor", "Zoom factor")}</p>
        {options.map(([id, label, enabled]) => (
          <label key={id} className={dlg.checkRow}>
            <input type="radio" name="zoom-mode" checked={mode === id} disabled={!enabled} onChange={() => setMode(id)} data-testid={`zoom-${id}`} />
            <span>{label}</span>
          </label>
        ))}
        <label className={dlg.checkRow}>
          <input type="radio" name="zoom-mode" checked={mode === "variable"} onChange={() => setMode("variable")} data-testid="zoom-variable" />
          <span>{t("zoomDlg.variable", "Variable")}</span>
          <input
            className={dlg.input}
            style={{ width: 80, marginLeft: 8 }}
            type="number"
            min={20}
            max={400}
            step={1}
            aria-label={t("zoomDlg.variable", "Variable")}
            value={value}
            onFocus={() => setMode("variable")}
            onChange={(e) => (setValue(e.target.value), setMode("variable"))}
            onBlur={() => pct !== null && setValue(String(pct))}
            data-testid="zoom-value"
          />
          <span>%</span>
        </label>
      </div>
    </W56Shell>
  );
}
