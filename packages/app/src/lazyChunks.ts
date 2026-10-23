/**
 * Code-split points for the rarely used UI. Each loader is shared by the
 * `React.lazy` wrapper in `App.tsx` and by `preloadChunks`, which warms the
 * chunks in idle time after start-up so a dialog's first open is instant
 * (the keystrokes that open it are never lost to a network or parse wait).
 */
export const loadDialogs = () => import("./components/Dialogs");
export const loadW56Dialogs = () => import("./data/W56Dialogs");
export const loadCommandPalette = () => import("./components/CommandPalette");
export const loadShortcutsModal = () => import("./components/ShortcutsModal");
export const loadPrintPreview = () => import("./print/PrintPreview");
export const loadChartEditor = () => import("./charts/ChartEditorPanel");
export const loadChartLayer = () => import("./charts/ChartLayer");
export const loadStylesUi = () => import("./styles/StylesUi");
export const loadNavigator = () => import("./navigator/NavigatorPanel");

/** Most likely first; each is fetched one idle slot after the previous. */
const PRELOAD = [loadDialogs, loadW56Dialogs, loadCommandPalette, loadStylesUi, loadPrintPreview, loadShortcutsModal, loadChartEditor, loadNavigator];

type IdleWindow = Window & { requestIdleCallback?: (cb: () => void, opts?: { timeout: number }) => number };

/** Warms every lazy chunk, one per idle slot. Safe to call more than once. */
let started = false;
export function preloadChunks(): void {
  if (started || typeof window === "undefined") return;
  started = true;
  const queue = [...PRELOAD];
  const next = (): void => {
    const load = queue.shift();
    if (!load) return;
    void load().catch(() => undefined).finally(schedule);
  };
  const schedule = (): void => {
    const w = window as IdleWindow;
    if (w.requestIdleCallback) w.requestIdleCallback(next, { timeout: 2000 });
    else setTimeout(next, 200);
  };
  schedule();
}
