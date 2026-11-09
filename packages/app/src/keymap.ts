/**
 * The app keymap: one table of bindings and a pure resolver. The grid
 * handles navigation/editing keys (arrows, F2, Enter, Delete…) itself;
 * everything here is a CommandRegistry command id.
 */

/** The parts of a KeyboardEvent the resolver reads. */
export interface KeyLike {
  key: string;
  code?: string;
  metaKey: boolean;
  ctrlKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
}

export interface Binding {
  /** `KeyboardEvent.key`, lower-case for letters. */
  key: string;
  /** Alternative match on `KeyboardEvent.code` (layout-independent keys). */
  code?: string;
  /**
   * Match on `code` only, never on `key` (the Alt-digit tab keys: ⌥ changes
   * `key` on macOS, and Alt combinations give odd `key` values on Windows).
   */
  codeOnly?: boolean;
  /** Cmd on macOS, Ctrl elsewhere (either is accepted). */
  mod: boolean;
  shift: boolean;
  alt?: boolean;
  command: string;
  /**
   * The native menu has the same accelerator, so inside Tauri the menu
   * handles it (and the webview may never see the key).
   */
  menu?: boolean;
  /** Also active while typing in a text field or editing a cell. */
  global?: boolean;
  /** Platform restriction: `true` macOS only, `false` Windows/Linux only, absent everywhere. */
  mac?: boolean;
  /** Label shown in menus and the palette, e.g. "⌘Z". */
  label: string;
}

/**
 * Keys the grid handles itself (navigation, editing, clipboard), listed in
 * the shortcut reference (⌘/) next to {@link BINDINGS}.
 */
export const GRID_KEYS: readonly { keys: string; action: string; group: string }[] = [
  { group: "Move", keys: "← ↑ → ↓", action: "Move (⇧ extends the selection)" },
  { group: "Move", keys: "⌘ + arrows", action: "Jump to the edge of the data" },
  { group: "Move", keys: "Home / End", action: "Start / end of the row" },
  { group: "Move", keys: "⌘Home / ⌘End", action: "A1 / last used cell" },
  { group: "Move", keys: "PgUp / PgDn", action: "One screen up / down" },
  { group: "Move", keys: "Tab / ⇧Tab", action: "Next / previous cell" },
  { group: "Select", keys: "⌘A", action: "Select the data, then everything" },
  { group: "Select", keys: "⌘ + click", action: "Add a range to the selection" },
  { group: "Edit", keys: "F2", action: "Edit the active cell" },
  { group: "Edit", keys: "Enter / ⇧Enter", action: "Commit and move down / up" },
  { group: "Edit", keys: "⌥Enter", action: "While typing in a cell: new line in the cell" },
  { group: "Edit", keys: "Esc", action: "Cancel editing" },
  { group: "Edit", keys: "⌫ / Delete", action: "Clear contents" },
  { group: "Edit", keys: "⌘C / ⌘X / ⌘V", action: "Copy / cut / paste" },
  { group: "Edit", keys: "⇧⌘V", action: "Paste values only" },
  { group: "Edit", keys: "⌥⌘V", action: "Paste special…" },
  { group: "Edit", keys: "⌘D / ⌘R", action: "Fill down / right" },
  { group: "Edit", keys: "⌘ + drag the fill handle", action: "Copy instead of continuing a series" },
  { group: "Edit", keys: "⌘X then ⌘V", action: "Move cells (references follow)" },
  { group: "Formulas", keys: "⌃⇧↩", action: "Enter as an array formula over the selection" },
  { group: "Find", keys: "⌘F / ⌥⌘F", action: "Find / Find & replace" },
  { group: "Find", keys: "⌘G / ↩ / ⇧↩", action: "Next / previous match (in the find bar)" },
  { group: "Formulas", keys: "Arrows after = + ( ,", action: "Point at a reference" },
  { group: "Formulas", keys: "Tab / Enter", action: "Accept a function suggestion" },
  { group: "Cells", keys: "⌘ + click a link", action: "Open the link (a plain click selects)" },
  { group: "Cells", keys: "Click ▾ in a cell", action: "Choose from the validation list" },
];

export const BINDINGS: readonly Binding[] = [
  { key: "z", mod: true, shift: false, command: "edit.undo", menu: true, label: "⌘Z" },
  { key: "z", mod: true, shift: true, command: "edit.redo", menu: true, label: "⇧⌘Z" },
  { key: "y", mod: true, shift: false, command: "edit.redo", label: "⌘Y" },
  { key: "b", mod: true, shift: false, command: "format.bold", menu: true, label: "⌘B" },
  { key: "i", mod: true, shift: false, command: "format.italic", menu: true, label: "⌘I" },
  { key: "u", mod: true, shift: false, command: "format.underline", menu: true, label: "⌘U" },
  { key: "l", mod: true, shift: true, command: "format.alignLeft", label: "⇧⌘L" },
  { key: "e", mod: true, shift: true, command: "format.alignCenter", label: "⇧⌘E" },
  { key: "r", mod: true, shift: true, command: "format.alignRight", label: "⇧⌘R" },
  { key: "!", code: "Digit1", mod: true, shift: true, command: "format.number.number", label: "⇧⌘1" },
  { key: "@", code: "Digit2", mod: true, shift: true, command: "format.number.scientific", label: "⇧⌘2" },
  { key: "#", code: "Digit3", mod: true, shift: true, command: "format.number.date", label: "⇧⌘3" },
  { key: "$", code: "Digit4", mod: true, shift: true, command: "format.currency", label: "⇧⌘4" },
  { key: "%", code: "Digit5", mod: true, shift: true, command: "format.percent", label: "⇧⌘5" },
  { key: "^", code: "Digit6", mod: true, shift: true, command: "format.number.general", label: "⇧⌘6" },
  { key: "g", mod: true, shift: true, command: "view.glance", global: true, label: "⇧⌘G" },
  { key: "/", code: "Slash", mod: true, shift: false, command: "help.shortcuts", menu: true, global: true, label: "⌘/" },
  { key: "k", mod: true, shift: false, command: "palette.toggle", global: true, label: "⌘K" },
  { key: "\\", code: "Backslash", mod: true, shift: false, command: "view.toggleSidebar", menu: true, global: true, label: "⌘\\" },
  { key: "s", mod: true, shift: false, command: "file.save", menu: true, global: true, label: "⌘S" },
  { key: "s", mod: true, shift: true, command: "file.saveAs", menu: true, global: true, label: "⇧⌘S" },
  { key: "n", mod: true, shift: false, command: "file.new", menu: true, global: true, label: "⌘N" },
  { key: "o", mod: true, shift: false, command: "file.open", menu: true, global: true, label: "⌘O" },
  { key: "=", code: "Equal", mod: true, shift: false, command: "view.zoomIn", menu: true, global: true, label: "⌘=" },
  { key: "+", code: "Equal", mod: true, shift: true, command: "view.zoomIn", global: true, label: "⌘+" },
  { key: "-", code: "Minus", mod: true, shift: false, command: "view.zoomOut", menu: true, global: true, label: "⌘−" },
  { key: "0", code: "Digit0", mod: true, shift: false, command: "view.zoomReset", menu: true, global: true, label: "⌘0" },
  { key: "f", mod: true, shift: false, command: "edit.find", menu: true, global: true, label: "⌘F" },
  { key: "f", code: "KeyF", mod: true, shift: false, alt: true, command: "edit.findReplace", menu: true, global: true, label: "⌥⌘F" },
  { key: "g", mod: true, shift: false, command: "edit.findNext", global: true, label: "⌘G" },
  { key: "v", code: "KeyV", mod: true, shift: false, alt: true, command: "edit.pasteSpecial", menu: true, label: "⌥⌘V" },
  { key: "d", mod: true, shift: false, command: "edit.fillDown", menu: true, label: "⌘D" },
  { key: "r", mod: true, shift: false, command: "edit.fillRight", menu: true, label: "⌘R" },
  { key: "F9", mod: true, shift: true, command: "data.recalc", menu: true, global: true, label: "⇧⌘F9" },
  { key: "F3", mod: true, shift: false, command: "data.defineName", menu: true, label: "⌘F3" },
  { key: "Escape", mod: false, shift: false, command: "edit.cancelClip", label: "Esc" },
  { key: "PageDown", mod: true, shift: false, command: "sheet.next", label: "⌃PgDn" },
  { key: "PageUp", mod: true, shift: false, command: "sheet.prev", label: "⌃PgUp" },
  { key: "PageDown", mod: true, shift: true, command: "sheet.selectNext", label: "⇧⌃PgDn" },
  { key: "PageUp", mod: true, shift: true, command: "sheet.selectPrev", label: "⇧⌃PgUp" },
  { key: "F2", mod: false, shift: true, command: "insert.note", menu: true, label: "⇧F2" },
  { key: "k", code: "KeyK", mod: true, shift: false, alt: true, command: "insert.hyperlink", menu: true, label: "⌥⌘K" },
  // Wave 5 (W5-0b seams; the commands are disabled until W5-5/W5-6 except the Function Wizard).
  { key: "F2", mod: true, shift: false, command: "insert.function", menu: true, label: "⌘F2" },
  { key: "1", code: "Digit1", mod: true, shift: false, command: "format.cells", menu: true, label: "⌘1" },
  { key: "F12", mod: false, shift: false, command: "data.group", menu: true, label: "F12" },
  { key: "F12", mod: true, shift: false, command: "data.ungroup", menu: true, label: "⌘F12" },
  { key: "F5", mod: false, shift: false, command: "view.navigator", menu: true, label: "F5" },
  { key: "F7", mod: false, shift: true, command: "tools.tracePrecedents", menu: true, label: "⇧F7" },
  { key: "F5", mod: false, shift: true, command: "tools.traceDependents", menu: true, label: "⇧F5" },
  { key: "F8", mod: true, shift: false, command: "view.valueHighlighting", menu: true, label: "⌘F8" },
  { key: "F11", mod: true, shift: false, command: "view.stylesPanel", menu: true, label: "⌘F11" },
  { key: "`", code: "Backquote", mod: true, shift: false, command: "view.showFormulas", menu: true, label: "⌃`" },
  { key: "p", mod: true, shift: false, command: "file.print", menu: true, global: true, label: "⌘P" },
  { key: ",", code: "Comma", mod: true, shift: false, command: "tools.options", menu: true, global: true, label: "⌘," },
  // Wave 5 (W5-6): shortcut parity (§4 item 49).
  { key: "y", mod: true, shift: true, command: "edit.repeat", label: "⇧⌘Y" },
  { key: "j", mod: true, shift: true, command: "view.fullScreen", global: true, label: "⇧⌘J" },
  { key: "t", mod: true, shift: true, command: "edit.focusNameBox", global: true, label: "⇧⌘T" },
  { key: "c", code: "KeyC", mod: true, shift: false, alt: true, command: "insert.note", label: "⌥⌘C" },
  { key: "F11", mod: false, shift: true, command: "file.saveAsTemplate", menu: true, label: "⇧F11" },
  { key: "F9", mod: false, shift: false, command: "data.recalc", label: "F9" },
  { key: "m", mod: true, shift: false, command: "edit.clearFormats", label: "⌘M" },
  // Edit ▸ Edit Mode (AOO 4.1 has no shortcut for its "Edit File" button; LibreOffice uses ⇧⌘M).
  { key: "m", mod: true, shift: true, command: "edit.editMode", menu: true, global: true, label: "⇧⌘M" },
  // Document tabs (desktop; tabs.ts). The native Window menu has ⌃⇥ / ⌃⇧⇥.
  // ⌘1 stays Format Cells and ⌃PgDn/⌃PgUp switch sheets (spreadsheet
  // conventions), so tabs by number are ⌥⌘N on macOS and Alt+N elsewhere.
  { key: "Tab", mod: true, shift: false, command: "window.nextTab", menu: true, global: true, label: "⌃⇥" },
  { key: "Tab", mod: true, shift: true, command: "window.prevTab", menu: true, global: true, label: "⌃⇧⇥" },
  { key: "}", code: "BracketRight", mod: true, shift: true, command: "window.nextTab", global: true, mac: true, label: "⇧⌘]" },
  { key: "{", code: "BracketLeft", mod: true, shift: true, command: "window.prevTab", global: true, mac: true, label: "⇧⌘[" },
  ...tabNumberBindings(),
];

/** ⌥⌘1–⌥⌘9 (macOS) and Alt+1–Alt+9 (Windows/Linux): show tab N, 9 = the last tab. */
function tabNumberBindings(): Binding[] {
  const out: Binding[] = [];
  for (let n = 1; n <= 9; n++) {
    const command = `window.selectTab${n}`;
    // Top-row digits only: Alt + numeric keypad is Windows' Alt-code character input.
    out.push({ key: String(n), code: `Digit${n}`, codeOnly: true, mod: true, shift: false, alt: true, command, global: true, mac: true, label: `⌥⌘${n}` });
    out.push({ key: String(n), code: `Digit${n}`, codeOnly: true, mod: false, shift: false, alt: true, command, mac: false, label: `Alt+${n}` });
  }
  return out;
}

export interface KeyContext {
  /** Focus is in a text field (formula bar, name box, palette) or a cell edit is open. */
  typing: boolean;
  /** Running inside Tauri with the native menu (menu accelerators win). */
  nativeMenu: boolean;
  /** macOS (platform-restricted bindings); absent counts as not macOS. */
  mac?: boolean;
}

function matches(b: Binding, e: KeyLike): boolean {
  const mod = e.metaKey || e.ctrlKey;
  if (b.mod !== mod || b.shift !== e.shiftKey || (b.alt ?? false) !== e.altKey) return false;
  if (b.codeOnly) return b.code !== undefined && e.code === b.code;
  const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
  return key === b.key || (b.code !== undefined && e.code === b.code);
}

/** The binding for a key event in a context, or `null`. */
export function resolveBinding(e: KeyLike, ctx: KeyContext, bindings: readonly Binding[] = BINDINGS): Binding | null {
  for (const b of bindings) {
    if (b.mac !== undefined && b.mac !== (ctx.mac ?? false)) continue;
    if (!matches(b, e)) continue;
    if (ctx.nativeMenu && b.menu) return null;
    if (ctx.typing && !b.global) return null;
    return b;
  }
  return null;
}

/** The command id for a key event in a context, or `null`. */
export function resolveKey(e: KeyLike, ctx: KeyContext, bindings: readonly Binding[] = BINDINGS): string | null {
  return resolveBinding(e, ctx, bindings)?.command ?? null;
}

/** Whether the UI runs on macOS (platform-restricted bindings and labels). */
export function isMacPlatform(): boolean {
  return typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);
}

/** Pure: the bindings that apply on one platform (`mac`), in table order. */
export function platformBindings(mac: boolean, bindings: readonly Binding[] = BINDINGS): Binding[] {
  return bindings.filter((b) => b.mac === undefined || b.mac === mac);
}

/** The first shortcut label for a command on this platform (palette, tooltips). */
export function shortcutFor(command: string, bindings: readonly Binding[] = BINDINGS, mac: boolean = isMacPlatform()): string | undefined {
  return platformBindings(mac, bindings).find((b) => b.command === command)?.label;
}
