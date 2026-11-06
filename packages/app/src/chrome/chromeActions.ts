/** Toolbar visibility actions (no React): stored in the app prefs via `W56Features.savePrefs`. */
import { create } from "zustand";
import type { AppController } from "../controller";
import { useEditStore } from "../stores/editStore";
import { useW56Store } from "../data/store";
import { withBar, type BarId, type ChromePrefs } from "./chromeModel";

/** Stores new chrome prefs (prefs.json + every window in the desktop app, localStorage in the browser). */
export function saveChrome(ctl: AppController, next: ChromePrefs): Promise<void> {
  return ctl.w56.savePrefs({ ...useW56Store.getState().prefs, chrome: next }).then(() => undefined);
}

/** The formula bar shown for a moment while it is hidden (⇧⌘T Go to the Name Box). */
export const usePeekStore = create<{ formulaBar: boolean; setFormulaBar(v: boolean): void }>()((set) => ({
  formulaBar: false,
  setFormulaBar: (formulaBar) => set({ formulaBar }),
}));

/**
 * View ▸ Toolbars ▸ <bar>. Hiding the formula bar while an edit typed in it
 * is open commits that edit first (as leaving the bar would); if the
 * commit is refused (e.g. validation), the bar stays.
 */
export async function toggleBar(ctl: AppController, bar: BarId): Promise<void> {
  const c = useW56Store.getState().prefs.chrome;
  const hide = c.bars[bar];
  if (bar === "formulaBar" && hide) {
    const ed = useEditStore.getState();
    if (ed.edit && ed.origin === "bar" && !(await ctl.commitEdit("none"))) return;
    usePeekStore.getState().setFormulaBar(false);
  }
  return saveChrome(ctl, withBar(useW56Store.getState().prefs.chrome, bar, !hide));
}

/** View ▸ Toolbars ▸ Customize Toolbars…. */
export function openCustomize(): void {
  useW56Store.getState().openDialog({ kind: "customizeToolbars" });
}
