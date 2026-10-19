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
  /** Label shown in menus and the palette, e.g. "⌘Z". */
  label: string;
}

export const BINDINGS: readonly Binding[] = [
  { key: "z", mod: true, shift: false, command: "edit.undo", menu: true, label: "⌘Z" },
  { key: "z", mod: true, shift: true, command: "edit.redo", menu: true, label: "⇧⌘Z" },
  { key: "y", mod: true, shift: false, command: "edit.redo", label: "⌘Y" },
  { key: "b", mod: true, shift: false, command: "format.bold", label: "⌘B" },
  { key: "i", mod: true, shift: false, command: "format.italic", label: "⌘I" },
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
  { key: "PageDown", mod: true, shift: false, command: "sheet.next", label: "⌃PgDn" },
  { key: "PageUp", mod: true, shift: false, command: "sheet.prev", label: "⌃PgUp" },
];

export interface KeyContext {
  /** Focus is in a text field (formula bar, name box, palette) or a cell edit is open. */
  typing: boolean;
  /** Running inside Tauri with the native menu (menu accelerators win). */
  nativeMenu: boolean;
}

function matches(b: Binding, e: KeyLike): boolean {
  const mod = e.metaKey || e.ctrlKey;
  if (b.mod !== mod || b.shift !== e.shiftKey || (b.alt ?? false) !== e.altKey) return false;
  const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
  return key === b.key || (b.code !== undefined && e.code === b.code);
}

/** The binding for a key event in a context, or `null`. */
export function resolveBinding(e: KeyLike, ctx: KeyContext, bindings: readonly Binding[] = BINDINGS): Binding | null {
  for (const b of bindings) {
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

/** The first shortcut label for a command (palette, tooltips). */
export function shortcutFor(command: string, bindings: readonly Binding[] = BINDINGS): string | undefined {
  return bindings.find((b) => b.command === command)?.label;
}
