/** Error and info toasts (bottom centre, auto-dismiss). */
import { useEffect } from "react";
import { cx } from "../context";
import { useUiStore, type Toast } from "../stores/uiStore";
import styles from "./Toasts.module.css";

export const TOAST_MS = 5000;

function ToastView({ toast }: { toast: Toast }) {
  useEffect(() => {
    const t = setTimeout(() => useUiStore.getState().dismissToast(toast.id), TOAST_MS);
    return () => clearTimeout(t);
  }, [toast.id]);
  return (
    <div className={cx(styles.toast, toast.kind === "error" && styles.error)} role={toast.kind === "error" ? "alert" : "status"}>
      <span className={styles.icon}>{toast.kind === "error" ? "!" : "✓"}</span>
      <span className={styles.message}>{toast.message}</span>
      <button type="button" className={styles.close} aria-label="Dismiss" onClick={() => useUiStore.getState().dismissToast(toast.id)}>
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
