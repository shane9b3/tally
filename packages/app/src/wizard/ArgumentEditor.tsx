/**
 * Function Wizard argument editor (W5-6C): one field per parameter (from
 * `FunctionInfo.params` when present, else the signature), a point-mode
 * pick button per field (`grid.pickRange()`), the resulting formula text
 * and, as a stretch, a live result when the engine can evaluate the call
 * (it cannot yet: there is no `eval_formula_text_at` command, so the
 * result line shows the formula structure instead).
 */
import type { FunctionInfo } from "@tally/bridge";
import { useEffect, useMemo, useState } from "react";
import dlg from "../components/Dialogs.module.css";
import { cx, useController } from "../context";
import { a1 } from "../dataOps";
import { t } from "../i18n";
import w56 from "../data/W56.module.css";
import { useW56Store } from "../data/store";
import { argumentError, argumentSlots, buildCall, wizardParams } from "./wizardArgs";

export interface ArgumentEditorProps {
  fn: FunctionInfo;
  /** Existing argument texts (re-opening the wizard on a call). */
  initial?: readonly string[];
  /** Reports the call text (`SUMIF(A1:A5;">3")`) and whether it is complete. */
  onChange(call: string, complete: boolean): void;
}

export function ArgumentEditor({ fn, initial, onChange }: ArgumentEditorProps) {
  const ctl = useController();
  const sep = useW56Store((s) => s.prefs.argumentSeparator);
  const params = useMemo(() => wizardParams(fn), [fn]);
  const [values, setValues] = useState<string[]>(() => [...(initial ?? [])]);
  useEffect(() => {
    setValues([...(initial ?? [])]);
  }, [fn.name, initial]);
  const slots = argumentSlots(params, values);
  const error = argumentError(slots, values);
  const call = buildCall(fn.name, values, sep);
  useEffect(() => {
    onChange(call, error === null);
    // onChange is stable for the wizard's lifetime.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [call, error]);
  const set = (i: number, v: string) => {
    const next = [...values];
    while (next.length <= i) next.push("");
    next[i] = v;
    setValues(next);
  };
  const pick = async (i: number) => {
    const r = await ctl.w56.pickRange();
    if (r) set(i, a1(r));
  };
  if (params.length === 0) return <div className={w56.muted}>{t("wizard.arguments", "Arguments")}: none.</div>;
  return (
    <div data-testid="wizard-arguments">
      <div className={w56.section}>{t("wizard.arguments", "Arguments")}</div>
      {slots.map((p, i) => (
        <label key={`${p.name}-${i}`} className={dlg.field} style={{ marginBottom: 6 }}>
          <span>
            {p.name} <span className={w56.muted}>{p.optional ? t("wizard.optional", "optional") : t("wizard.required", "required")}</span>
          </span>
          <div className={w56.rangeRow}>
            <input className={cx(dlg.input, dlg.mono)} style={{ height: 28 }} value={values[i] ?? ""} placeholder={p.description ?? ""} spellCheck={false} onChange={(e) => set(i, e.target.value)} data-testid={`wizard-arg-${i}`} />
            <button type="button" className={w56.pickBtn} style={{ width: 28, height: 28 }} title={t("wizard.select", "Select")} aria-label={`${p.name}: ${t("wizard.select", "Select")}`} onClick={() => void pick(i)} data-testid={`wizard-pick-${i}`}>
              ⌖
            </button>
          </div>
        </label>
      ))}
      <div className={w56.section}>{t("wizard.formula", "Formula")}</div>
      <code className={dlg.mono} data-testid="wizard-call">
        ={call}
      </code>
      {error ? <p className={dlg.error}>{error}</p> : null}
    </div>
  );
}
