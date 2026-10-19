/**
 * ⌘F: a compact find bar at the top-right of the grid. Enter / ⇧Enter (or
 * the arrows) go to the next / previous match; options: match case, whole
 * cell, search formulas, wildcards/regex, all sheets. "Replace…" opens the
 * Find & Replace dialog with the same text.
 */
import type { FindMode } from "@tally/bridge";
import { useEffect, useRef } from "react";
import { useController } from "../context";
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

const MODES: { mode: FindMode; label: string }[] = [
  { mode: "literal", label: "Text" },
  { mode: "wildcards", label: "Wildcards" },
  { mode: "regex", label: "Regex" },
];

function Bar() {
  const ctl = useController();
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
  const status = find.searched === null ? "" : total === 0 ? "No matches" : `${find.index + 1} of ${total}${find.truncated ? "+" : ""}`;

  return (
    <div
      className={styles.bar}
      role="search"
      aria-label="Find"
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
          placeholder="Find in sheet"
          aria-label="Find"
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
        <button type="button" className={panel.iconBtn} aria-label="Previous match" title="Previous match  ⇧↩" disabled={find.text === ""} onClick={() => void ctl.findStep(-1)} data-testid="find-prev">
          <ArrowUpIcon />
        </button>
        <button type="button" className={panel.iconBtn} aria-label="Next match" title="Next match  ↩" disabled={find.text === ""} onClick={() => void ctl.findStep(1)} data-testid="find-next">
          <ArrowDownIcon />
        </button>
        <button
          type="button"
          className={panel.link}
          onClick={() => useUiStore.getState().openDialog({ kind: "findReplace", text: find.text })}
          title="Find & replace  ⌥⌘F"
        >
          Replace…
        </button>
        <button type="button" className={panel.iconBtn} aria-label="Close find" title="Close  Esc" onClick={close}>
          ✕
        </button>
      </div>
      <div className={styles.options}>
        <button type="button" className={panel.chip} aria-pressed={opts.matchCase} onClick={() => setOpt({ matchCase: !opts.matchCase })} title="Match case">
          Aa
        </button>
        <button type="button" className={panel.chip} aria-pressed={opts.wholeCell} onClick={() => setOpt({ wholeCell: !opts.wholeCell })} title="The whole cell must match" data-testid="find-whole">
          Whole cell
        </button>
        <button type="button" className={panel.chip} aria-pressed={opts.inFormulas} onClick={() => setOpt({ inFormulas: !opts.inFormulas })} title="Search formulas instead of values">
          Formulas
        </button>
        <button type="button" className={panel.chip} aria-pressed={opts.allSheets} onClick={() => setOpt({ allSheets: !opts.allSheets })} title="Search every sheet">
          All sheets
        </button>
        <select className={`${panel.select} ${styles.mode}`} aria-label="Match mode" value={opts.mode} onChange={(e) => setOpt({ mode: e.target.value as FindMode })}>
          {MODES.map((m) => (
            <option key={m.mode} value={m.mode}>
              {m.label}
            </option>
          ))}
        </select>
      </div>
    </div>
  );
}
