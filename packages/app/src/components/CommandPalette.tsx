/**
 * ⌘K palette (cmdk). Sources: assist suggestions (ANSWER: insert a formula;
 * ACTIONS: run a mapped command), every CommandRegistry command, and the
 * function list.
 */
import type { FunctionInfo, Suggestion } from "@tally/bridge";
import { Command } from "cmdk";
import { useEffect, useMemo, useState } from "react";
import { useController } from "../context";
import { pretty } from "../pretty";
import { useDocStore } from "../stores/docStore";
import { toSelectionDto, useSelectionStore } from "../stores/selectionStore";
import { useUiStore } from "../stores/uiStore";
import { SparkIcon } from "./icons";
import styles from "./CommandPalette.module.css";

export const ASSIST_DEBOUNCE_MS = 120;

export function CommandPalette() {
  const open = useUiStore((s) => s.paletteOpen);
  if (!open) return null;
  return <PaletteCard />;
}

function PaletteCard() {
  const ctl = useController();
  const [query, setQuery] = useState("");
  const [suggestions, setSuggestions] = useState<Suggestion[]>([]);
  const [functions, setFunctions] = useState<FunctionInfo[]>([]);
  const doc = useDocStore((s) => s.info?.doc);
  const close = (focus = true) => {
    useUiStore.getState().setPalette(false);
    if (focus) ctl.focusGrid();
  };

  useEffect(() => {
    ctl.client.listFunctions().then(setFunctions, () => setFunctions([]));
  }, [ctl]);

  useEffect(() => {
    const q = query.trim();
    if (doc === undefined || q.length < 2) {
      setSuggestions([]);
      return undefined;
    }
    let live = true;
    const t = setTimeout(() => {
      const sel = toSelectionDto(useSelectionStore.getState().selection, useDocStore.getState().activeSheet);
      ctl.client.assistSuggest(doc, q, sel).then(
        (s) => live && setSuggestions(s),
        () => live && setSuggestions([]),
      );
    }, ASSIST_DEBOUNCE_MS);
    return () => {
      live = false;
      clearTimeout(t);
    };
  }, [ctl, doc, query]);

  const answers = suggestions.filter((s) => s.formula !== null);
  const actionHints = suggestions.filter((s) => s.formula === null && s.action !== null && ctl.registry.has(s.action));
  const commands = useMemo(() => ctl.registry.list(), [ctl]);

  return (
    <div
      className={styles.overlay}
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) close();
      }}
    >
      <Command
        className={styles.card}
        label="Command palette"
        loop
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.preventDefault();
            close();
          }
        }}
      >
        <div className={styles.inputRow}>
          <SparkIcon />
          <Command.Input
            className={styles.input}
            autoFocus
            placeholder="Ask or do anything…"
            value={query}
            onValueChange={setQuery}
            data-testid="palette-input"
          />
          <kbd className={styles.esc}>esc</kbd>
        </div>
        <Command.List className={styles.list}>
          <Command.Empty className={styles.empty}>No matches. Try “sum of Total” or “bold”.</Command.Empty>
          {answers.length > 0 ? (
            <Command.Group heading="Answer" forceMount>
              {answers.map((s, i) => (
                <Command.Item
                  key={`a${i}`}
                  value={`answer ${i} ${s.title}`}
                  forceMount
                  className={styles.answer}
                  onSelect={() => {
                    close();
                    if (s.formula) void ctl.insertFormula(s.formula);
                  }}
                >
                  <div className={styles.answerTop}>
                    <span className={styles.value}>{s.preview ?? s.title}</span>
                    <code className={styles.formula}>{pretty(s.formula ?? "")}</code>
                    <span className={styles.insert}>Insert ↵</span>
                  </div>
                  <div className={styles.explanation}>{s.explanation || s.title}</div>
                </Command.Item>
              ))}
            </Command.Group>
          ) : null}
          <Command.Group heading="Actions">
            {actionHints.map((s, i) => {
              const cmd = s.action ? ctl.registry.get(s.action) : undefined;
              return (
                <Command.Item
                  key={`h${i}`}
                  value={`suggested ${i} ${s.title}`}
                  forceMount
                  className={styles.item}
                  onSelect={() => {
                    close();
                    if (s.action) ctl.registry.run(s.action);
                  }}
                >
                  <span className={styles.title}>{s.title}</span>
                  <span className={styles.kind}>{cmd?.kind ?? ""}</span>
                </Command.Item>
              );
            })}
            {commands.map((c) => {
              const enabled = ctl.registry.isEnabled(c.id);
              return (
                <Command.Item
                  key={c.id}
                  value={`${c.title} ${c.id}`}
                  keywords={c.keywords ?? []}
                  disabled={!enabled}
                  className={styles.item}
                  data-command={c.id}
                  onSelect={() => {
                    close();
                    ctl.registry.run(c.id);
                  }}
                >
                  <span className={styles.title}>{c.title}</span>
                  {!enabled && c.reason ? <span className={styles.reason}>{c.reason}</span> : null}
                  {c.shortcut ? <kbd className={styles.shortcut}>{c.shortcut}</kbd> : null}
                  <span className={styles.kind}>{c.kind}</span>
                </Command.Item>
              );
            })}
          </Command.Group>
          {query.trim() !== "" && functions.length > 0 ? (
            <Command.Group heading="Functions">
              {functions.map((f) => (
                <Command.Item
                  key={f.name}
                  value={`fn ${f.name}`}
                  keywords={[f.category, f.description]}
                  className={styles.item}
                  onSelect={() => {
                    close(false);
                    ctl.startEdit(useSelectionStore.getState().selection.active, `=${f.name}(`, "grid");
                    ctl.focusGrid();
                  }}
                >
                  <code className={styles.fn}>{f.name}</code>
                  <span className={styles.fnDesc}>{f.description}</span>
                  <span className={styles.kind}>{f.category}</span>
                </Command.Item>
              ))}
            </Command.Group>
          ) : null}
        </Command.List>
      </Command>
    </div>
  );
}
