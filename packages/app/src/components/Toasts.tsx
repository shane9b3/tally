/** Error and info toasts (bottom centre, auto-dismiss). */
import { useEffect } from "react";
import { cx } from "../context";
import { t } from "../i18n";
import { useUiStore, type Toast } from "../stores/uiStore";
import styles from "./Toasts.module.css";

export const TOAST_MS = 5000;

/** Toasts with buttons stay this long (they wait for a choice). */
export const ACTION_TOAST_MS = 15000;

function ToastView({ toast }: { toast: Toast }) {
  const actions = toast.actions ?? [];
  useEffect(() => {
    const t = setTimeout(() => useUiStore.getState().dismissToast(toast.id), actions.length > 0 ? ACTION_TOAST_MS : TOAST_MS);
    return () => clearTimeout(t);
  }, [toast.id, actions.length]);
  const alert = toast.kind === "error" || toast.kind === "warning";
  return (
    <div
      className={cx(styles.toast, toast.kind === "error" && styles.error, toast.kind === "warning" && styles.warning)}
      role={alert ? "alert" : "status"}
      data-testid={`toast-${toast.kind}`}
    >
      <span className={styles.icon}>{toast.kind === "info" ? "✓" : "!"}</span>
      <span className={styles.message}>{toast.message}</span>
      {actions.map((a) => (
        <button
          key={a.label}
          type="button"
          className={styles.action}
          data-testid={`toast-action-${a.id ?? a.label}`}
          onClick={() => {
            useUiStore.getState().dismissToast(toast.id);
            a.run();
          }}
        >
          {a.label}
        </button>
      ))}
      <button type="button" className={styles.close} aria-label={t("shell.dismiss", "Dismiss")} onClick={() => useUiStore.getState().dismissToast(toast.id)}>
        ×
      </button>
    </div>
  );
}

export function Toasts() {
  const toasts = useUiStore((s) => s.toasts);
  return (
    <div className={styles.stack} aria-live="polite">
      {toasts.map((t) => (
        <ToastView key={t.id} toast={t} />
      ))}
    </div>
  );
}
