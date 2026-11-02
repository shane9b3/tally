/**
 * AOO's "Document in Use" question (uui `OpenLockedQueryBox`,
 * `AlreadyOpenQueryBox`) and its "edit a copy" question
 * (sfx2 `MSG_QUERY_OPENASTEMPLATE`), with AOO's wording. The first button
 * is the default (Enter); Escape is Cancel.
 */
import { useEffect, useRef } from "react";
import { cx } from "../context";
import { t } from "../i18n";
import { lockPromptButtons, lockPromptParts, useLockPromptStore, type LockAnswer } from "../readOnly";
import { trapTab } from "./trapTab";
import dlg from "./Dialogs.module.css";
import styles from "./SavePrompt.module.css";

function label(a: LockAnswer, yesNo: boolean): string {
  switch (a) {
    case "readOnly":
      return t("dialog.locked.openReadOnly", "Open Read-Only");
    case "copy":
      return yesNo ? t("dialog.yes", "Yes") : t("dialog.locked.openCopy", "Open Copy");
    case "open":
      return t("dialog.locked.open", "Open");
    case "cancel":
      return yesNo ? t("dialog.no", "No") : t("dialog.cancel", "Cancel");
  }
}

export function LockPrompt() {
  const prompt = useLockPromptStore((s) => s.prompt);
  const answer = useLockPromptStore((s) => s.answer);
  const formRef = useRef<HTMLFormElement>(null);
  const firstRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (prompt) firstRef.current?.focus();
  }, [prompt]);

  if (!prompt) return null;
  const { title, parts } = lockPromptParts(prompt.kind, prompt.lock);
  const buttons = lockPromptButtons(prompt.kind);
  const yesNo = prompt.kind === "notWritable";
  const [first] = buttons;
  return (
    <div className={cx(dlg.overlay, styles.overlay)}>
      <form
        ref={formRef}
        className={cx(dlg.card, dlg.wide)}
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="lock-prompt-title"
        aria-describedby="lock-prompt-detail"
        data-testid="lock-prompt"
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
          if (first) answer(first);
        }}
      >
        <h2 id="lock-prompt-title" className={cx(dlg.title, styles.title)}>
          {title}
        </h2>
        <p id="lock-prompt-detail" className={dlg.lead} style={{ whiteSpace: "pre-line" }}>
          {parts.map((p, i) => (typeof p === "string" ? (i === parts.length - 1 ? p.trimEnd() : p) : <bdi key={i}>{p.bdi}</bdi>))}
        </p>
        <div className={dlg.actions}>
          {buttons.map((b, i) => (
            <button
              key={b}
              ref={i === 0 ? firstRef : undefined}
              type={i === 0 ? "submit" : "button"}
              className={i === 0 ? dlg.primary : dlg.secondary}
              onClick={i === 0 ? undefined : () => answer(b)}
              data-testid={`lock-prompt-${b}`}
            >
              {label(b, yesNo)}
            </button>
          ))}
        </div>
      </form>
    </div>
  );
}
