/**
 * The unsaved-changes prompt (unsaved.ts): "Do you want to save the changes
 * you made to “<title>”?" with Don't Save on the left and Cancel / Save on
 * the right, as on macOS and in AOO. Save is the default button (Enter);
 * Escape is Cancel. Built on the shared dialog styles (`Dialogs.module.css`)
 * and focus trap (`trapTab`) like the other modal dialogs.
 */
import { useEffect, useRef } from "react";
import { cx } from "../context";
import { t } from "../i18n";
import { useSavePromptStore } from "../unsaved";
import { trapTab } from "./trapTab";
import dlg from "./Dialogs.module.css";
import styles from "./SavePrompt.module.css";

export function SavePrompt() {
  const prompt = useSavePromptStore((s) => s.prompt);
  const answer = useSavePromptStore((s) => s.answer);
  const formRef = useRef<HTMLFormElement>(null);
  const saveRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (prompt) saveRef.current?.focus();
  }, [prompt]);

  if (!prompt) return null;
  const title = t("dialog.saveChanges.title", "Do you want to save the changes you made to “{name}”?", { name: prompt.title });
  return (
    <div className={cx(dlg.overlay, styles.overlay)}>
      <form
        ref={formRef}
        className={cx(dlg.card, dlg.wide)}
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="save-prompt-title"
        aria-describedby="save-prompt-detail"
        data-testid="save-prompt"
        onKeyDown={(e) => {
          e.stopPropagation();
          if (e.key === "Escape") {
            e.preventDefault();
            answer("cancel");
            return;
          }
          if (formRef.current) trapTab(formRef.current, e, document.activeElement);
        }}
        onSubmit={(e) => {
          e.preventDefault();
          answer("save");
        }}
      >
        <h2 id="save-prompt-title" className={cx(dlg.title, styles.title)}>
          {title}
        </h2>
        <p id="save-prompt-detail" className={dlg.lead}>
          {t("dialog.saveChanges.detail", "Your changes will be lost if you don't save them.")}
        </p>
        <div className={dlg.actions}>
          <div className={dlg.extra}>
            <button type="button" className={dlg.secondary} onClick={() => answer("discard")} data-testid="save-prompt-discard">
              {t("dialog.dontSave", "Don't Save")}
            </button>
          </div>
          <button type="button" className={dlg.secondary} onClick={() => answer("cancel")} data-testid="save-prompt-cancel">
            {t("dialog.cancel", "Cancel")}
          </button>
          <button ref={saveRef} type="submit" className={dlg.primary} data-testid="save-prompt-save">
            {t("dialog.save", "Save")}
          </button>
        </div>
      </form>
    </div>
  );
}
