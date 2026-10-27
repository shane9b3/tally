/**
 * ⌘F: a compact find bar at the top-right of the grid. Enter / ⇧Enter (or
 * the arrows) go to the next / previous match; options: match case, whole
 * cell, search formulas, wildcards/regex, all sheets. "Replace…" opens the
 * Find & Replace dialog with the same text.
 */
import type { FindMode } from "@tally/bridge";
import { useEffect, useRef } from "react";
import { useController } from "../context";
import { useW56Store } from "../data/store";
import { t } from "../i18n";
import { useDataStore } from "../stores/dataStore";
import { useUiStore } from "../stores/uiStore";
import { ArrowDownIcon, ArrowUpIcon } from "./icons";
import styles from "./FindBar.module.css";
import panel from "./Panel.module.css";

export function FindBar() {
  const open = useUiStore((s) => s.findOpen);
  if (!open) return null;
  return <Bar />;
}

const MODES: { mode: FindMode; label: () => string }[] = [
  { mode: "literal", label: () => t("paste.text", "Text") },
  { mode: "wildcards", label: () => t("find.wildcards", "Wildcards") },
  { mode: "regex", label: () => t("findbar.regex", "Regex") },
];

function Bar() {
  const ctl = useController();
  useW56Store((s) => s.i18nRev);
  const find = useDataStore((s) => s.find);
  const patch = useDataStore.getState().patchFind;
  const input = useRef<HTMLInputElement>(null);
  const seq = useUiStore((s) => s.findOpen);
  useEffect(() => {
    input.current?.focus();
    input.current?.select();
  }, [seq]);
  const close = () => {
    useUiStore.getState().setFind(false);
    ctl.focusGrid();
  };
  const opts = find.options;
  const setOpt = (o: Partial<typeof opts>) => patch({ options: { ...opts, ...o }, searched: null, index: -1 });
  const total = find.hits.length;
  const status = find.searched === null ? "" : total === 0 ? t("filter.noMatches", "No matches") : t("findbar.matchOf", "{n} of {total}", { n: find.index + 1, total: `${total}${find.truncated ? "+" : ""}` });

  return (
    <div
      className={styles.bar}
      role="search"
      aria-label={t("findbar.find", "Find")}
      data-testid="find-bar"
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.preventDefault();
          e.stopPropagation();
          close();
        }
      }}
    >
      <div className={styles.top}>
        <input
          ref={input}
          className={`${panel.input} ${styles.field}`}
          placeholder={t("findbar.placeholder", "Find in sheet")}
          aria-label={t("findbar.find", "Find")}
          spellCheck={false}
          value={find.text}
          data-testid="find-input"
          onChange={(e) => patch({ text: e.target.value, searched: null, index: -1, hits: [] })}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              void ctl.findStep(e.shiftKey ? -1 : 1);
            }
          }}
        />
        <span className={styles.status} data-testid="find-status">
          {status}
        </span>
        <button type="button" className={panel.iconBtn} aria-label={t("findbar.prev", "Previous match")} title={`${t("findbar.prev", "Previous match")}  ⇧↩`} disabled={find.text === ""} onClick={() => void ctl.findStep(-1)} data-testid="find-prev">
          <ArrowUpIcon />
        </button>
        <button type="button" className={panel.iconBtn} aria-label={t("findbar.next", "Next match")} title={`${t("findbar.next", "Next match")}  ↩`} disabled={find.text === ""} onClick={() => void ctl.findStep(1)} data-testid="find-next">
          <ArrowDownIcon />
        </button>
        <button
          type="button"
          className={panel.link}
          onClick={() => useUiStore.getState().openDialog({ kind: "findReplace", text: find.text })}
          title={`${t("dlg.findReplace", "Find & replace")}  ⌥⌘F`}
        >
          {t("findbar.replaceEllipsis", "Replace…")}
        </button>
        <button type="button" className={panel.iconBtn} aria-label={t("findbar.close", "Close find")} title={`${t("dialog.close", "Close")}  Esc`} onClick={close}>
          ✕
        </button>
      </div>
      <div className={styles.options}>
        <button type="button" className={panel.chip} aria-pressed={opts.matchCase} onClick={() => setOpt({ matchCase: !opts.matchCase })} title={t("find.matchCase", "Match case")}>
          Aa
        </button>
        <button type="button" className={panel.chip} aria-pressed={opts.wholeCell} onClick={() => setOpt({ wholeCell: !opts.wholeCell })} title={t("findbar.wholeCellTip", "The whole cell must match")} data-testid="find-whole">
          {t("findbar.wholeCell", "Whole cell")}
        </button>
        <button type="button" className={panel.chip} aria-pressed={opts.inFormulas} onClick={() => setOpt({ inFormulas: !opts.inFormulas })} title={t("findbar.formulasTip", "Search formulas instead of values")}>
          {t("find.formulas", "Formulas")}
        </button>
        <button type="button" className={panel.chip} aria-pressed={opts.allSheets} onClick={() => setOpt({ allSheets: !opts.allSheets })} title={t("findbar.allSheetsTip", "Search every sheet")}>
          {t("find.allSheets", "All sheets")}
        </button>
        <select className={`${panel.select} ${styles.mode}`} aria-label={t("findbar.matchMode", "Match mode")} value={opts.mode} onChange={(e) => setOpt({ mode: e.target.value as FindMode })}>
          {MODES.map((m) => (
            <option key={m.mode} value={m.mode}>
              {m.label()}
            </option>
          ))}
        </select>
      </div>
    </div>
  );
}
