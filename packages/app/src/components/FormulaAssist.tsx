/**
 * Function autocomplete and the signature tooltip, for whichever editor
 * holds the EditSession (in-cell or formula bar). Keyboard handling lives
 * in `AppController.handleEditorKey` (↑/↓, Tab/Enter, Esc); this renders.
 */
import { useEffect, useState } from "react";
import { useController } from "../context";
import { assistFor, signatureAt, splitSignature } from "../formulaEdit";
import { t } from "../i18n";
import { useEditStore } from "../stores/editStore";
import { useFunctionsStore } from "../stores/functionsStore";
import styles from "./FormulaAssist.module.css";

interface Anchor {
  left: number;
  top: number;
  bottom: number;
}

export function FormulaAssist() {
  const ctl = useController();
  const edit = useEditStore((s) => s.edit);
  const origin = useEditStore((s) => s.origin);
  const index = useEditStore((s) => s.assistIndex);
  const dismissed = useEditStore((s) => s.assistDismissed);
  const functions = useFunctionsStore((s) => s.functions);
  const [anchor, setAnchor] = useState<Anchor | null>(null);

  // Measure the editor after the grid has laid it out (it updates in an effect).
  useEffect(() => {
    if (!edit) {
      setAnchor(null);
      return undefined;
    }
    const raf = requestAnimationFrame(() => {
      let rect: DOMRect | null = null;
      if (origin === "bar") rect = document.querySelector('[data-testid="formula-input"]')?.getBoundingClientRect() ?? null;
      else rect = ctl.grid?.editorRect() ?? null;
      if (!rect) return setAnchor(null);
      const next = { left: Math.round(rect.left), top: Math.round(rect.top), bottom: Math.round(rect.bottom) };
      setAnchor((a) => (a && a.left === next.left && a.top === next.top && a.bottom === next.bottom ? a : next));
    });
    return () => cancelAnimationFrame(raf);
  }, [ctl, edit, origin]);

  if (!edit || !anchor) return null;
  const list = assistFor(edit.text, edit.caret, functions, dismissed);
  const pos = { left: anchor.left, top: anchor.bottom + 4 };

  if (list) {
    const active = Math.min(index, list.items.length - 1);
    const current = list.items[active];
    return (
      <div className={styles.popup} style={pos} role="listbox" aria-label={t("wizard.functions", "Functions")} data-testid="fn-autocomplete">
        {list.items.map((f, i) => (
          <div
            key={f.name}
            role="option"
            aria-selected={i === active}
            className={i === active ? `${styles.item} ${styles.active}` : styles.item}
            onMouseDown={(e) => {
              e.preventDefault(); // keep focus in the editor
              ctl.insertCompletion(f.name);
            }}
            onMouseEnter={() => useEditStore.getState().setAssist(i)}
          >
            <span className={styles.name}>
              <b>{f.name.slice(0, list.prefix.length)}</b>
              {f.name.slice(list.prefix.length)}
            </span>
            <span className={styles.category}>{f.category}</span>
          </div>
        ))}
        {current ? (
          <div className={styles.detail}>
            <code className={styles.signature}>{current.signature}</code>
            <div className={styles.description}>{current.description}</div>
            <div className={styles.keys}>
              <kbd>Tab</kbd> {t("formulaUi.assistInsert", "insert")} · <kbd>Esc</kbd> {t("formulaUi.assistClose", "close")}
            </div>
          </div>
        ) : null}
      </div>
    );
  }

  const sig = signatureAt(edit.text, edit.caret, functions);
  if (!sig) return null;
  const [before, arg, after] = splitSignature(sig.info.signature, sig.argIndex);
  return (
    <div className={styles.tooltip} style={pos} role="tooltip" data-testid="fn-signature">
      <code className={styles.signature}>
        {before}
        <b className={styles.arg}>{arg}</b>
        {after}
      </code>
      <div className={styles.description}>{sig.info.description}</div>
    </div>
  );
}
