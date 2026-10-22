/**
 * `docs/shortcuts.md` is generated from the keymap (§4 item 49): the table
 * below is rendered from `BINDINGS` (one row per command, alternates
 * joined) plus the grid's own keys (`GRID_KEYS`) and the extra keys the
 * W5-6 key hook handles (`EXTRA_KEYS`). `keymap.test.ts` renders it again
 * and compares with the file, so the two cannot drift.
 */
import type { Binding } from "../keymap";

export interface GridKeyRow {
  keys: string;
  action: string;
  group: string;
}

/** Keys handled by `useW56Keys` (window capture) rather than the keymap table. */
export const EXTRA_KEYS: readonly GridKeyRow[] = [
  { group: "Edit", keys: "Delete", action: "Delete Contents dialog (⌫ clears at once; Tools ▸ Options ▸ Input turns the dialog off)" },
  { group: "Edit", keys: "⌥↩", action: "Fill the selection with the entered text" },
  { group: "Edit", keys: "⌃D", action: "Selection list (entries of the column)" },
  { group: "Select", keys: "F8 / ⇧F8", action: "Extend-selection mode / add-selection mode (toggle)" },
  { group: "Select", keys: "⌘* (numpad) / ⇧⌘8", action: "Select the data area" },
  { group: "Select", keys: "⌘/ (numpad)", action: "Select the array formula around the cursor" },
  { group: "Cells", keys: "⌥ + arrows", action: "Change the row height / column width (⇧ sets the optimal size)" },
  { group: "Cells", keys: "⌃⌘+ / ⌃⌘−", action: "Insert / delete cells…" },
];

/**
 * Document tabs (desktop): the native Window menu (handled by the shell,
 * `apps/desktop/src-tauri/src/menu.rs`) and the sidebar's "Open" list
 * (`components/OpenDocuments.tsx`). Next/previous and by-number switch
 * among the tabs of the front window, also with the sidebar hidden.
 */
export const TAB_KEYS: readonly { keys: string; action: string }[] = [
  { keys: "⌃⇥ / ⇧⌘] (macOS)", action: "Window ▸ Show Next Tab" },
  { keys: "⌃⇧⇥ / ⇧⌘[ (macOS)", action: "Window ▸ Show Previous Tab" },
  { keys: "⌥⌘1 … ⌥⌘8 (macOS), Alt+1 … Alt+8 (Windows/Linux)", action: "Show tab 1 … 8 of the window" },
  { keys: "⌥⌘9 (macOS), Alt+9 (Windows/Linux)", action: "Show the window's last tab" },
  { keys: "⌘W", action: "Close the tab (the window with its last tab); asks to save an edited document" },
  { keys: "—", action: "Window ▸ Move Tab to New Window / Merge All Windows; the open documents are listed below them" },
  { keys: "Click / ×", action: "Sidebar ▸ Open (every open document, this window's first): show it (another window comes to the front) / close it; drag to reorder this window's tabs; right-click for Close, Close Others, Move to New Window" },
  { keys: "↑ ↓ Home End, ↩ / Space, ⌫", action: "Sidebar ▸ Open with focus: move between documents, show the focused one, close it" },
];

/** The deliberate deviations from the AOO table (listed under the tables). */
export const DEVIATIONS: readonly string[] = [
  "⌘D stays Fill Down (Excel and macOS usage); the AOO selection list is ⌃D.",
  "⌘/ opens the shortcut reference; the AOO Select Array Formula is ⌘/ on the numeric keypad.",
  "⌘⇧F9 is the hard recalculation; plain F9 recalculates the dirty cells (AutoCalculate is always on).",
  "⌘K opens the command palette (AOO: ⌘K inserts a hyperlink, here ⌥⌘K).",
  "⌘1 stays Format Cells and ⌃PgDn / ⌃PgUp stay Next / Previous Sheet, so document tabs use ⌥⌘1–9 (Alt+1–9 on Windows/Linux) and ⌃⇥ / ⌃⇧⇥ rather than the browser-style ⌘1–9 and ⌃PgDn / ⌃PgUp.",
  "F4 (data source browser), F7 (spelling) and ⌥F11 (macros) are not bound: those features are out of scope (docs/parity-deviations.md).",
];

function esc(s: string): string {
  return s.replace(/\|/g, "\\|");
}

/** Renders the Markdown. `titleOf(commandId)` gives the command's palette title. */
export function renderShortcutsDoc(bindings: readonly Binding[], gridKeys: readonly GridKeyRow[], titleOf: (id: string) => string): string {
  const byCommand = new Map<string, string[]>();
  // One table for every platform: platform-only keys say which platform.
  const labelOf = (b: Binding): string => (b.mac === undefined ? b.label : `${b.label} (${b.mac ? "macOS" : "Windows/Linux"})`);
  for (const b of bindings) byCommand.set(b.command, [...(byCommand.get(b.command) ?? []), labelOf(b)]);
  const rows = [...byCommand.entries()].map(([id, labels]) => ({ id, keys: labels.join(" / "), title: titleOf(id) }));
  const groups = new Map<string, GridKeyRow[]>();
  for (const g of [...gridKeys, ...EXTRA_KEYS]) groups.set(g.group, [...(groups.get(g.group) ?? []), g]);
  const lines: string[] = [];
  lines.push("# Keyboard shortcuts");
  lines.push("");
  lines.push("Generated from `packages/app/src/keymap.ts` (`BINDINGS`, `GRID_KEYS`) and `packages/app/src/options/shortcutsDoc.ts`; `packages/app/test/keymap.test.ts` fails when this file is out of date. Regenerate with `TALLY_WRITE_SHORTCUTS=1 pnpm --filter @tally/app test -- keymap`.");
  lines.push("");
  lines.push("⌘ is Ctrl on Windows and Linux; ⌃ is the Control key on every platform.");
  lines.push("");
  lines.push("## Commands (keymap table)");
  lines.push("");
  lines.push("| Keys | Command | Id |");
  lines.push("|---|---|---|");
  for (const r of rows) lines.push(`| ${esc(r.keys)} | ${esc(r.title)} | \`${r.id}\` |`);
  lines.push("");
  lines.push("## Documents and tabs");
  lines.push("");
  lines.push("Desktop app. Tools ▸ Options ▸ Load / Save ▸ \"Open documents in\" chooses tabs in one window (default) or separate windows.");
  lines.push("");
  lines.push("| Keys | Action |");
  lines.push("|---|---|");
  for (const r of TAB_KEYS) lines.push(`| ${esc(r.keys)} | ${esc(r.action)} |`);
  lines.push("");
  lines.push("## Grid and editor keys");
  lines.push("");
  for (const [group, items] of groups) {
    lines.push(`### ${group}`);
    lines.push("");
    lines.push("| Keys | Action |");
    lines.push("|---|---|");
    for (const it of items) lines.push(`| ${esc(it.keys)} | ${esc(it.action)} |`);
    lines.push("");
  }
  lines.push("## Deliberate deviations from Apache OpenOffice");
  lines.push("");
  for (const d of DEVIATIONS) lines.push(`- ${d}`);
  lines.push("");
  return lines.join("\n");
}
