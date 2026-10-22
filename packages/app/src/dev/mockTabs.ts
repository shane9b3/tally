/**
 * Dev/e2e only (`?tabs=<n>` in the browser build): a fake tab shell so the
 * sidebar's "Open" list can be seen and driven without the desktop app:
 * `n` tabs in this window plus one document in another window. The actions
 * edit the tab store directly; every tab still shows this one document (the
 * browser has one webview), and showing the other window's document does
 * nothing here (the real shell brings that window to the front).
 */
import { useTabsStore, type TabShell, type TabsState } from "../tabs";

/** A dev/e2e mock of `n` tabs (`?tabs=<n>` in the browser build). */
export function mockTabsState(n: number): TabsState {
  const tabs = Array.from({ length: Math.max(0, Math.min(20, n)) }, (_, i) => ({
    label: `doc-${i + 1}`,
    window: "win-1",
    title: i === 0 ? "" : `Book ${i + 1}.ods`,
    edited: i === 1,
    path: i === 0 ? null : `/Users/me/Documents/Book ${i + 1}.ods`,
  }));
  const others = [{ label: "doc-21", window: "win-2", title: "Report.xlsx", edited: false, path: "/Users/me/Documents/Report.xlsx" }];
  return { window: "win-1", tabs, active: "doc-1", others };
}


function update(f: (s: TabsState) => TabsState | null): Promise<void> {
  const s = useTabsStore.getState().state;
  if (s) useTabsStore.getState().setState(f(s));
  return Promise.resolve();
}

/** Fills the tab store with `n` mock tabs and gives `client` the tab commands. */
export function installMockTabs(client: object, n: number): void {
  useTabsStore.getState().setSelf("doc-1");
  useTabsStore.getState().setState(mockTabsState(n));
  const shell: TabShell = {
    // The shown tab's webview is the one that is looked at: it becomes `self` too.
    tabActivate: (label) =>
      update((s) => {
        if (!s.tabs.some((t) => t.label === label)) return s;
        useTabsStore.getState().setSelf(label);
        return { ...s, active: label };
      }),
    tabClose: (label) =>
      update((s) => {
        const i = s.tabs.findIndex((t) => t.label === label);
        if (i < 0) return { ...s, others: s.others.filter((t) => t.label !== label) };
        const tabs = s.tabs.filter((t) => t.label !== label);
        const active = s.active === label ? (tabs[Math.min(i, tabs.length - 1)]?.label ?? "") : s.active;
        if (s.active === label) useTabsStore.getState().setSelf(active);
        return { ...s, tabs, active };
      }),
    tabCloseOthers: (label) =>
      update((s) => {
        useTabsStore.getState().setSelf(label);
        return { ...s, tabs: s.tabs.filter((t) => t.label === label), active: label };
      }),
    tabMove: (label, index) =>
      update((s) => {
        const tab = s.tabs.find((t) => t.label === label);
        if (!tab) return s;
        const tabs = s.tabs.filter((t) => t.label !== label);
        tabs.splice(index, 0, tab);
        return { ...s, tabs };
      }),
    tabDetach: (label) => shell.tabClose?.(label) ?? Promise.resolve(),
    mergeWindows: () => Promise.resolve(),
  };
  Object.assign(client, shell);
}
