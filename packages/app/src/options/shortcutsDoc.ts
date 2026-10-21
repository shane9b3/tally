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

/** The deliberate deviations from the AOO table (listed under the tables). */
export const DEVIATIONS: readonly string[] = [
  "⌘D stays Fill Down (Excel and macOS usage); the AOO selection list is ⌃D.",
  "⌘/ opens the shortcut reference; the AOO Select Array Formula is ⌘/ on the numeric keypad.",
  "⌘⇧F9 is the hard recalculation; plain F9 recalculates the dirty cells (AutoCalculate is always on).",
  "⌘K opens the command palette (AOO: ⌘K inserts a hyperlink, here ⌥⌘K).",
  "F4 (data source browser), F7 (spelling) and ⌥F11 (macros) are not bound: those features are out of scope (docs/parity-deviations.md).",
];

function esc(s: string): string {
  return s.replace(/\|/g, "\\|");
}

/** Renders the Markdown. `titleOf(commandId)` gives the command's palette title. */
export function renderShortcutsDoc(bindings: readonly Binding[], gridKeys: readonly GridKeyRow[], titleOf: (id: string) => string): string {
  const byCommand = new Map<string, string[]>();
  for (const b of bindings) byCommand.set(b.command, [...(byCommand.get(b.command) ?? []), b.label]);
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
