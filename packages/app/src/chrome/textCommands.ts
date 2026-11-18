/**
 * Edit ▸ Cut / Copy / Paste / Select All chosen in the in-app menu bar
 * (Windows). In a native menu the OS sends these to whatever has the
 * focus; the in-app bar gives the focus back before the item runs, so when
 * that is a text field (formula bar, name box, find bar, a dialog input, an
 * open cell editor) the item must act on its text, not on the sheet
 * selection behind it. Otherwise the shell sends the sheet command
 * (`NATIVE_MENU_COMMANDS`).
 */

/** What the shell sends for the items a native menu leaves to the OS (`Native::in_app`, menu.rs). */
export const NATIVE_MENU_COMMANDS: Readonly<Record<string, string>> = {
  "native.cut": "edit.cut",
  "native.copy": "edit.copy",
  "native.paste": "edit.paste",
  "native.selectAll": "edit.selectAll",
};

export type TextCommand = "cut" | "copy" | "paste" | "selectAll";

const TEXT_COMMANDS: Readonly<Record<string, TextCommand>> = { "native.cut": "cut", "native.copy": "copy", "native.paste": "paste", "native.selectAll": "selectAll" };
const NOT_TEXT = new Set(["checkbox", "radio", "button", "submit", "reset", "range", "color", "file", "image", "hidden"]);

/** The text command of a menu item id, if it is one of the OS's clipboard items. */
export function textCommandOf(id: string): TextCommand | undefined {
  return TEXT_COMMANDS[id];
}

/**
 * The focused text field a clipboard item acts on, or `null` when the sheet
 * is the target. The grid's own textarea counts only while a cell edit is
 * open (`editing`): idle, it merely holds the keyboard for the grid.
 */
export function textTarget(editing: boolean, active: Element | null = typeof document === "undefined" ? null : document.activeElement): HTMLElement | null {
  if (!(active instanceof HTMLElement)) return null;
  const text = active.isContentEditable || active instanceof HTMLTextAreaElement || (active instanceof HTMLInputElement && !NOT_TEXT.has(active.type));
  if (!text) return null;
  if (active.closest(".tally-grid") && !editing) return null;
  return active;
}

type Field = HTMLInputElement | HTMLTextAreaElement;
const isField = (el: HTMLElement): el is Field => el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement;
const exec = (command: string, value?: string): boolean => {
  try {
    return typeof document.execCommand === "function" && document.execCommand(command, false, value);
  } catch {
    return false;
  }
};

function selectedText(el: HTMLElement): string {
  if (isField(el)) return el.value.slice(el.selectionStart ?? 0, el.selectionEnd ?? 0);
  return window.getSelection()?.toString() ?? "";
}

/** Replaces the selection of `el` with `text` as typing would (the field's `input` listeners run). */
export function insertText(el: HTMLElement, text: string): void {
  el.focus();
  if (exec(text === "" ? "delete" : "insertText", text)) return;
  if (!isField(el)) return;
  const start = el.selectionStart ?? el.value.length;
  const end = el.selectionEnd ?? start;
  if (text === "" && start === end) return;
  el.setRangeText(text, start, end, "end");
  el.setSelectionRange(start + text.length, start + text.length);
  el.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: text === "" ? "deleteContentBackward" : "insertText", data: text }));
}

export interface TextClipboard {
  /** The clipboard's plain text; `null` when it cannot be read. */
  readText(): Promise<string | null>;
  writeText(text: string): Promise<void>;
  /** Paste could not read the clipboard: tell the user to use the keyboard. */
  unreadable(): void;
}

/** Carries out a clipboard item on the text field `el`. */
export function runTextCommand(el: HTMLElement, command: TextCommand, clipboard: TextClipboard): void {
  el.focus();
  if (command === "selectAll") {
    if (isField(el)) el.select();
    else exec("selectAll");
    return;
  }
  if (command === "copy" || command === "cut") {
    // The browser's own copy / cut first (it runs inside the click that chose the item).
    if (exec(command)) return;
    const text = selectedText(el);
    if (text === "") return;
    void clipboard.writeText(text).then(
      () => {
        if (command === "cut") insertText(el, "");
      },
      () => undefined,
    );
    return;
  }
  // Paste: browsers refuse `execCommand("paste")` to pages; read the clipboard instead.
  if (exec("paste")) return;
  void clipboard.readText().then(
    (text) => {
      if (text === null) clipboard.unreadable();
      else if (el.isConnected) insertText(el, text);
    },
    () => clipboard.unreadable(),
  );
}
