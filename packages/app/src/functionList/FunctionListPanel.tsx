/**
 * Function List (Insert ▸ Function List, AOO `ScFunctionDockWin`): a docked
 * panel with a category box (Last Used, All, the categories), the functions of
 * the category, the selected function's syntax and help text, and an Insert
 * button. Double-click, Enter and Insert write `NAME()` at the caret of the
 * cell edit (starting `=NAME()` when no edit is open), the caret between the
 * parentheses. Names are the document language's; the syntax uses the UI
 * argument separator.
 */
import { useEffect, useMemo, useRef, useState } from "react";
import { cx, useController } from "../context";
import { t } from "../i18n";
import { canonicalName, localizedName, useDocFormulaLang } from "../i18n/formulaNames";
import { useW56Store } from "../data/store";
import w56 from "../data/W56.module.css";
import { useEditStore } from "../stores/editStore";
import { useFunctionsStore } from "../stores/functionsStore";
import { noteFunctionUsed, useLastUsedStore } from "../stores/lastUsedStore";
import { useDocStore } from "../stores/docStore";
import type { AppController } from "../controller";
import styles from "./FunctionListPanel.module.css";
import { ALL, functionsOf, insertFunctionText, LAST_USED, listCategories, signatureWithSeparator, typeAheadIndex } from "./model";

/** The category chosen last (AOO keeps the box's selection while the window lives). */
let rememberedCategory = LAST_USED;

/**
 * Writes `NAME()` into the open edit at the caret (or starts `=NAME()` on the
 * active cell) and puts the caret between the parentheses. `false` when
 * nothing was inserted (read-only document, references being picked on another sheet).
 */
export function insertFunctionCall(ctl: AppController, name: string, canonical: string): boolean {
  if (ctl.readOnly.active || ctl.editingElsewhere) return false;
  const st = useEditStore.getState();
  if (!st.edit) {
    ctl.startEdit(ctl.selection.active, `=${name}()`, "grid");
    const started = useEditStore.getState();
    if (!started.edit) return false;
    started.replace(`=${name}()`, name.length + 2);
  } else {
    const res = insertFunctionText(st.edit.text, st.edit.caret, name);
    st.replace(res.text, res.caret);
  }
  noteFunctionUsed(canonical);
  if (useEditStore.getState().origin === "bar") document.querySelector<HTMLElement>('[data-testid="formula-input"]')?.focus();
  else ctl.focusGrid();
  return true;
}

export function FunctionListPanel() {
  const ctl = useController();
  const functions = useFunctionsStore((s) => s.functions);
  const recent = useLastUsedStore((s) => s.names);
  const lang = useDocFormulaLang();
  const sep = useW56Store((s) => s.prefs.argumentSeparator);
  useW56Store((s) => s.i18nRev);
  const readOnly = useDocStore((s) => s.info?.readOnly ?? false);
  const editing = useEditStore((s) => s.edit !== null && s.sheet !== null && s.sheet !== ctl.sheet);
  const [category, setCategory] = useState(rememberedCategory);
  const [selected, setSelected] = useState<string | null>(null);
  const buffer = useRef({ text: "", at: 0 });
  const listRef = useRef<HTMLUListElement>(null);

  const categories = useMemo(() => listCategories(functions), [functions]);
  const list = useMemo(
    () => functionsOf(functions, categories.includes(category) ? category : ALL, recent, (n) => (lang ? localizedName(lang, n) : n)),
    [functions, categories, category, recent, lang],
  );
  const current = list.find((f) => f.name === selected) ?? list[0] ?? null;
  const canInsert = current !== null && !readOnly && !editing;

  const label = (c: string): string => (c === LAST_USED ? t("wizard.lastUsed", "Last Used") : c === ALL ? t("wizard.all", "All") : c);

  const insert = (name: string | undefined = current?.name) => {
    if (!name || readOnly || editing) return;
    const canonical = (lang ? canonicalName(lang, name) : null) ?? name;
    insertFunctionCall(ctl, name, canonical);
  };

  const select = (i: number) => {
    const f = list[Math.max(0, Math.min(list.length - 1, i))];
    if (!f) return;
    setSelected(f.name);
    document.getElementById(`fl-${f.name}`)?.scrollIntoView?.({ block: "nearest" });
  };

  useEffect(() => {
    if (selected !== null && !list.some((f) => f.name === selected)) setSelected(null);
  }, [list, selected]);

  const onKey = (e: React.KeyboardEvent) => {
    const idx = Math.max(0, list.findIndex((f) => f.name === current?.name));
    if (e.key === "ArrowDown") select(idx + 1);
    else if (e.key === "ArrowUp") select(idx - 1);
    else if (e.key === "Home") select(0);
    else if (e.key === "End") select(list.length - 1);
    else if (e.key === "PageDown") select(idx + 8);
    else if (e.key === "PageUp") select(idx - 8);
    else if (e.key === "Enter") insert();
    else if (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey && e.key !== " ") {
      const now = Date.now();
      const b = buffer.current;
      b.text = now - b.at > 800 ? e.key : b.text + e.key;
      b.at = now;
      const names = list.map((f) => f.name);
      // A repeated single letter cycles through the matches.
      const cycle = b.text.length > 1 && [...b.text].every((c) => c === b.text[0]);
      const from = cycle ? idx + 1 : b.text.length === 1 ? idx + 1 : idx;
      const hit = typeAheadIndex(names, cycle ? b.text[0]! : b.text, from);
      if (hit >= 0) select(hit);
    } else return;
    e.preventDefault();
  };

  return (
    <aside className={w56.panel} aria-label={t("cmd.insert.functionList", "Function list")} data-testid="function-list-panel">
      <div className={w56.panelHead}>
        <span className={w56.grow}>{t("cmd.insert.functionList", "Function list")}</span>
        <button type="button" className={w56.iconBtn} aria-label={t("dialog.close", "Close")} onClick={() => useW56Store.getState().setPanel(null)}>
          ×
        </button>
      </div>
      <div className={styles.body}>
        <label className={styles.field}>
          {t("wizard.category", "Category")}
          <select
            className={styles.select}
            value={categories.includes(category) ? category : ALL}
            onChange={(e) => {
              rememberedCategory = e.target.value;
              setCategory(e.target.value);
              setSelected(null);
            }}
            data-testid="function-list-category"
          >
            {categories.map((c) => (
              <option key={c} value={c}>
                {label(c)}
              </option>
            ))}
          </select>
        </label>
        <ul
          ref={listRef}
          className={styles.list}
          role="listbox"
          tabIndex={0}
          aria-label={t("wizard.functions", "Functions")}
          aria-activedescendant={current ? `fl-${current.name}` : undefined}
          onKeyDown={onKey}
          data-testid="function-list"
        >
          {list.map((f) => (
            <li
              key={f.name}
              id={`fl-${f.name}`}
              role="option"
              aria-selected={f.name === current?.name}
              className={cx(styles.item, f.name === current?.name && styles.itemOn)}
              onClick={() => setSelected(f.name)}
              onDoubleClick={() => {
                setSelected(f.name);
                insert(f.name);
              }}
            >
              {f.name}
            </li>
          ))}
        </ul>
        <div className={styles.description} data-testid="function-list-description" aria-live="polite">
          {current ? (
            <>
              <div className={styles.name}>{current.name}:</div>
              <code className={styles.syntax}>{signatureWithSeparator(current.signature, sep)}</code>
              <div>{current.description}</div>
            </>
          ) : null}
        </div>
        <button type="button" className={styles.insert} disabled={!canInsert} onClick={() => insert()} data-testid="function-list-insert">
          <span aria-hidden="true">fx</span> {t("dialog.insert", "Insert")}
        </button>
      </div>
    </aside>
  );
}
