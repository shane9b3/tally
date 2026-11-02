/**
 * "This document is open in read-only mode." with an Edit Document button
 * (Edit ▸ Edit Mode), shown while the document is read-only. A polite
 * live region (`role="status"`) so screen readers announce it once.
 */
import { useController } from "../context";
import { t } from "../i18n";
import { useDocStore } from "../stores/docStore";
import { useW56Store } from "../data/store";
import styles from "./ReadOnlyBanner.module.css";

export function ReadOnlyBanner() {
  const ctl = useController();
  const readOnly = useDocStore((s) => s.info?.readOnly === true);
  const hasFile = useDocStore((s) => s.info?.path != null);
  useW56Store((s) => s.i18nRev);
  if (!readOnly) return null;
  return (
    <div className={styles.banner} role="status" data-testid="read-only-banner">
      <svg className={styles.icon} width="14" height="14" viewBox="0 0 16 16" aria-hidden="true">
        <rect x="3" y="7" width="10" height="7" rx="1.5" fill="none" stroke="currentColor" strokeWidth="1.5" />
        <path d="M5.5 7V5a2.5 2.5 0 0 1 5 0v2" fill="none" stroke="currentColor" strokeWidth="1.5" />
      </svg>
      <span className={styles.text}>{t("readOnly.banner", "This document is open in read-only mode.")}</span>
      {hasFile ? (
        <button
          type="button"
          className={styles.button}
          data-testid="read-only-edit"
          onClick={() => void ctl.registry.run("edit.editMode")}
        >
          {t("readOnly.editDocument", "Edit Document")}
        </button>
      ) : null}
    </div>
  );
}
