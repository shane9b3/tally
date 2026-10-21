/**
 * Function Wizard (⌘F2 / Insert ▸ Function…).
 *
 * Category list (Last Used, All, the engine's categories), a search box,
 * the function list, its signature, description, parameters and example
 * (`FunctionInfo.params` / `example` from the engine's registry), the
 * argument editor with point mode, the live result of the call at the
 * active cell (`preview_formula`, debounced) and Insert, which writes the
 * call into the open editor (or commits it to the active cell).
 */
import type { EngineClientV4, FunctionInfo } from "@tally/bridge";
import { useEffect, useMemo, useRef, useState } from "react";
import { useController } from "../context";
import { t } from "../i18n";
import { useEditStore } from "../stores/editStore";
import { useFunctionsStore } from "../stores/functionsStore";
import { close, Shell } from "./Dialogs";
import styles from "./Dialogs.module.css";
import wiz from "./FunctionWizard.module.css";
import { ArgumentEditor } from "../wizard/ArgumentEditor";
import { parseCall } from "../wizard/wizardArgs";

/** AOO's default "Last Used" list; later inserts move to the front (session only). */
const DEFAULT_LAST_USED = ["SUM", "AVERAGE", "MIN", "MAX", "IF"];
let lastUsed: string[] = [...DEFAULT_LAST_USED];

/** The "Last Used" names, most recent first (tests). */
export function lastUsedFunctions(): readonly string[] {
  return lastUsed;
}

/** Records an insert (moves `name` to the front, keeps 10). */
export function noteFunctionUsed(name: string): void {
  lastUsed = [name, ...lastUsed.filter((n) => n !== name)].slice(0, 10);
}

/** Resets the "Last Used" list (tests). */
export function resetLastUsed(): void {
  lastUsed = [...DEFAULT_LAST_USED];
}

export const LAST_USED = "__last";
export const ALL = "__all";

/** Debounce of the live result (ms). */
export const PREVIEW_DEBOUNCE_MS = 150;

/**
 * The live result of `call` (`SUMIF(A1:A5;">3")`) at the active cell,
 * debounced; stale answers are dropped. `null` while there is nothing to
 * show; `{ error }` when the engine refuses.
 */
function usePreview(call: string | null): { text: string | null; error: boolean } {
  const ctl = useController();
  const [state, setState] = useState<{ text: string | null; error: boolean }>({ text: null, error: false });
  const seq = useRef(0);
  useEffect(() => {
    const n = ++seq.current;
    const info = ctl.info;
    if (!call || !info) {
      setState({ text: null, error: false });
      return;
    }
    const client = ctl.client as EngineClientV4;
    if (typeof client.previewFormula !== "function") return;
    const timer = setTimeout(() => {
      const a = ctl.selection.active;
      client.previewFormula(info.doc, ctl.sheet, a.r, a.c, `=${call}`).then(
        (text) => {
          if (n === seq.current) setState({ text, error: false });
        },
        () => {
          if (n === seq.current) setState({ text: null, error: true });
        },
      );
    }, PREVIEW_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [call, ctl]);
  return state;
}

/** Categories in display order: Last Used, All, then the engine's (sorted). */
export function wizardCategories(functions: readonly FunctionInfo[]): string[] {
  const cats = [...new Set(functions.map((f) => f.category))].sort((a, b) => a.localeCompare(b));
  return [LAST_USED, ALL, ...cats];
}

/** The functions shown for a category and search text (name prefix first, then substring, then description). */
export function wizardList(functions: readonly FunctionInfo[], category: string, query: string, recent: readonly string[] = lastUsed): FunctionInfo[] {
  const q = query.trim().toUpperCase();
  let list: FunctionInfo[];
  if (category === LAST_USED) {
    const byName = new Map(functions.map((f) => [f.name, f]));
    list = recent.map((n) => byName.get(n)).filter((f): f is FunctionInfo => f !== undefined);
  } else if (category === ALL) {
    list = [...functions];
  } else {
    list = functions.filter((f) => f.category === category);
  }
  if (q === "") return list;
  const rank = (f: FunctionInfo): number =>
    f.name.startsWith(q) ? 0 : f.name.includes(q) ? 1 : f.description.toUpperCase().includes(q) ? 2 : 3;
  return list
    .map((f) => ({ f, r: rank(f) }))
    .filter((x) => x.r < 3)
    .sort((a, b) => a.r - b.r || a.f.name.localeCompare(b.f.name))
    .map((x) => x.f);
}

/** `NAME(` inserted at the caret of `text`. */
export function insertCallAt(text: string, caret: number, name: string): { text: string; caret: number } {
  const ins = `${name}(`;
  const at = Math.max(0, Math.min(caret, text.length));
  return { text: text.slice(0, at) + ins + text.slice(at), caret: at + ins.length };
}

export function FunctionWizard() {
  const ctl = useController();
  const functions = useFunctionsStore((s) => s.functions);
  const [category, setCategory] = useState<string>(LAST_USED);
  const [query, setQuery] = useState("");
  const existing = useMemo(() => {
    const e = useEditStore.getState().edit;
    return e ? parseCall(e.text) : null;
  }, []);
  const [selected, setSelected] = useState<string | null>(existing?.name ?? lastUsed[0] ?? null);
  const [call, setCall] = useState<{ text: string; complete: boolean } | null>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const categories = useMemo(() => wizardCategories(functions), [functions]);
  const list = useMemo(() => wizardList(functions, category, query), [functions, category, query]);
  const current = list.find((f) => f.name === selected) ?? list[0] ?? null;
  const callText = call && current && call.text.startsWith(`${current.name}(`) ? call.text : null;
  const preview = usePreview(callText);

  useEffect(() => {
    searchRef.current?.focus();
  }, []);

  // Keep the selection on a visible item.
  useEffect(() => {
    if (list.length > 0 && !list.some((f) => f.name === selected)) setSelected(list[0]?.name ?? null);
  }, [list, selected]);

  const insert = () => {
    if (!current) return;
    noteFunctionUsed(current.name);
    const st = useEditStore.getState();
    const full = call && call.text !== `${current.name}()` ? call.text : null;
    if (full && existing && existing.name === current.name && st.edit) {
      // Re-opened on an existing call: replace it.
      st.replace(`=${full}`, full.length + 1);
      void ctl.commitEdit("none");
      return;
    }
    if (full && !st.edit) {
      void ctl.insertFormula(`=${full}`);
      return;
    }
    if (st.edit) {
      const res = full ? { text: st.edit.text.slice(0, st.edit.caret) + full + st.edit.text.slice(st.edit.caret), caret: st.edit.caret + full.length } : insertCallAt(st.edit.text, st.edit.caret, current.name);
      st.replace(res.text, res.caret);
    } else {
      ctl.startEdit(ctl.selection.active, `=${current.name}(`, "grid");
    }
  };

  const move = (delta: number) => {
    if (list.length === 0) return;
    const i = Math.max(0, list.findIndex((f) => f.name === current?.name));
    const next = list[Math.min(list.length - 1, Math.max(0, i + delta))];
    if (next) setSelected(next.name);
  };

  const label = (c: string): string => (c === LAST_USED ? t("wizard.lastUsed", "Last Used") : c === ALL ? t("wizard.all", "All") : c);

  return (
    <Shell title={t("wizard.title", "Function Wizard")} onSubmit={insert} ok={t("wizard.insertFunction", "Insert")} valid={current !== null} cardClass={wiz.card} testId="function-wizard">
      <div className={wiz.columns}>
        <div className={wiz.left}>
          <label className={styles.field}>
            {t("wizard.category", "Category")}
            <select className={styles.select} value={category} onChange={(e) => setCategory(e.target.value)} data-testid="wizard-category">
              {categories.map((c) => (
                <option key={c} value={c}>
                  {label(c)}
                </option>
              ))}
            </select>
          </label>
          <label className={styles.field}>
            {t("wizard.search", "Search")}
            <input
              ref={searchRef}
              className={styles.input}
              value={query}
              placeholder="SUMIF, VLOOKUP…"
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "ArrowDown") {
                  e.preventDefault();
                  move(1);
                } else if (e.key === "ArrowUp") {
                  e.preventDefault();
                  move(-1);
                }
              }}
              aria-controls="function-wizard-list"
              aria-activedescendant={current ? `fw-${current.name}` : undefined}
              data-testid="wizard-search"
            />
          </label>
          <ul id="function-wizard-list" className={wiz.list} role="listbox" aria-label={t("wizard.functions", "Functions")} data-testid="wizard-list">
            {list.map((f) => (
              <li
                key={f.name}
                id={`fw-${f.name}`}
                role="option"
                aria-selected={f.name === current?.name}
                className={f.name === current?.name ? wiz.itemOn : wiz.item}
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => setSelected(f.name)}
                onDoubleClick={() => {
                  setSelected(f.name);
                  insert();
                  close();
                  ctl.focusGrid();
                }}
              >
                {f.name}
              </li>
            ))}
            {list.length === 0 ? <li className={wiz.empty}>{t("wizard.noFunction", "No function selected")}</li> : null}
          </ul>
        </div>
        <div className={wiz.right} data-testid="wizard-description">
          {current ? (
            <>
              <code className={wiz.signature}>{current.signature}</code>
              <div className={wiz.description}>{current.description}</div>
              <div className={wiz.category}>{current.category}</div>
              {current.params && current.params.length > 0 ? (
                <dl className={wiz.params} data-testid="wizard-params">
                  {current.params.map((p) => (
                    <div key={p.name}>
                      <dt>
                        {p.name}{" "}
                        <span className={wiz.muted}>
                          {p.optional ? t("wizard.optional", "optional") : t("wizard.required", "required")}
                          {p.repeat ? ", …" : ""}
                        </span>
                      </dt>
                    </div>
                  ))}
                </dl>
              ) : null}
              {current.example ? (
                <div className={wiz.example} data-testid="wizard-example">
                  {t("wizard.example", "Example")}: <code>{current.example}</code>
                </div>
              ) : null}
              <ArgumentEditor fn={current} initial={existing && existing.name === current.name ? existing.args : undefined} onChange={(text, complete) => setCall({ text, complete })} />
              <div className={wiz.result} data-testid="wizard-result" aria-live="polite">
                {t("wizard.result", "Result")}: <output>{preview.error ? t("wizard.error", "Error") : (preview.text ?? "")}</output>
              </div>
            </>
          ) : (
            <div className={wiz.muted}>{t("wizard.noFunction", "No function selected")}</div>
          )}
        </div>
      </div>
    </Shell>
  );
}
