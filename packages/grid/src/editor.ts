/**
 * The in-cell editor host: one `<textarea>` that is always focused while the
 * grid is. When idle it is invisible and parked over the active cell, so
 * typing and IME composition start there naturally (and the IME candidate
 * window appears next to the cell); when editing it becomes the visible
 * editor, styled like the cell, growing to the right as text is typed.
 */
import type { CellPos, StyleDto } from "@tally/bridge";
import type { Theme } from "@tally/tokens";
import type { Geometry } from "./geometry";
import { CELL_PADDING, cellFont, fontPx, type TextMeasurer } from "./renderer";
import type { EditState } from "./types";

/** "enter": started by typing (arrows commit); "edit": F2/double-click (arrows move the caret). */
export type EditMode = "enter" | "edit";

export interface EditorStyleInfo {
  style: StyleDto | undefined;
  theme: Theme;
  /** Right-align the editor text (explicit right alignment only). */
  align: "left" | "center" | "right";
}

export class CellEditor {
  readonly el: HTMLTextAreaElement;
  open = false;
  mode: EditMode = "enter";
  cell: CellPos | null = null;
  composing = false;
  /** Texts we reported recently; an `edit` prop echoing one of them is not applied back. */
  private readonly emitted: string[] = [];
  private lastBox = "";

  constructor(doc: Document) {
    const ta = doc.createElement("textarea");
    ta.setAttribute("aria-label", "Cell editor");
    ta.setAttribute("autocomplete", "off");
    ta.setAttribute("autocorrect", "off");
    ta.setAttribute("autocapitalize", "off");
    ta.spellcheck = false;
    ta.wrap = "off";
    ta.tabIndex = 0;
    const s = ta.style;
    s.position = "absolute";
    s.zIndex = "3";
    s.margin = "0";
    s.resize = "none";
    s.overflow = "hidden";
    s.whiteSpace = "pre";
    s.boxSizing = "border-box";
    s.outline = "none";
    s.border = "0";
    s.padding = "0";
    s.background = "transparent";
    this.el = ta;
    this.setHidden();
  }

  /** Remembers a text we emitted (so its echo is not written back). */
  noteEmitted(text: string): void {
    this.emitted.push(text);
    if (this.emitted.length > 16) this.emitted.shift();
  }

  wasEmitted(text: string): boolean {
    return this.emitted.includes(text);
  }

  get value(): string {
    return this.el.value;
  }

  get caret(): number {
    return this.el.selectionStart;
  }

  setValue(text: string, caret = text.length): void {
    this.el.value = text;
    const c = Math.max(0, Math.min(text.length, caret));
    try {
      this.el.setSelectionRange(c, c);
    } catch {
      // Detached or not focusable; ignore.
    }
  }

  begin(cell: CellPos, mode: EditMode): void {
    this.open = true;
    this.mode = mode;
    this.cell = { ...cell };
    this.lastBox = "";
  }

  end(): void {
    this.open = false;
    this.cell = null;
    this.composing = false;
    this.emitted.length = 0;
    this.el.value = "";
    this.lastBox = "";
    this.setHidden();
  }

  /** A placeholder selection so `copy`/`cut` fire even in WebKit. */
  prepareClipboard(): void {
    if (this.open) return;
    this.el.value = " ";
    this.el.select();
  }

  resetIdleValue(): void {
    if (!this.open && this.el.value !== "") this.el.value = "";
  }

  private setHidden(): void {
    const s = this.el.style;
    s.opacity = "0";
    s.pointerEvents = "none";
    s.caretColor = "transparent";
    s.boxShadow = "none";
    s.border = "0";
    s.background = "transparent";
    s.color = "transparent";
  }

  /**
   * Positions the textarea: parked over `active` when idle, or as the
   * visible editor over the edited cell. Cheap when nothing changed.
   */
  layout(geo: Geometry, active: CellPos, info: EditorStyleInfo, measurer: TextMeasurer): void {
    const cell = this.open && this.cell ? this.cell : active;
    const rect = geo.cellRect(cell.r, cell.c);
    const zoom = geo.zoom;
    if (!this.open) {
      const x = clampN(rect.x, geo.headerW, Math.max(geo.headerW, geo.width - 2));
      const y = clampN(rect.y, geo.headerH, Math.max(geo.headerH, geo.height - 2));
      const key = `idle:${x}:${y}:${rect.h}`;
      if (key === this.lastBox) return;
      this.lastBox = key;
      const s = this.el.style;
      s.left = `${x}px`;
      s.top = `${y}px`;
      s.width = "2px";
      s.height = `${Math.max(2, Math.min(rect.h, 40))}px`;
      s.font = cellFont(info.style, info.theme, zoom);
      return;
    }
    const font = cellFont(info.style, info.theme, zoom);
    const fpx = fontPx(info.style, info.theme, zoom);
    const lineH = Math.round(fpx * 1.3);
    const lines = this.el.value.split("\n");
    let textW = 0;
    for (const line of lines) textW = Math.max(textW, measurer.measure(font, line));
    const pad = CELL_PADDING * zoom;
    const border = 2;
    const left = rect.x - 1;
    const top = rect.y - 1;
    const maxW = Math.max(rect.w + 1, geo.width - left - 2);
    const width = Math.min(maxW, Math.max(rect.w + 1, Math.ceil(textW + 2 * pad + 8)));
    const minH = rect.h + 1;
    const contentH = lines.length * lineH;
    const height = Math.min(Math.max(minH, contentH + 2 * border + 4), Math.max(minH, geo.height - top - 2));
    const padTop = Math.max(0, (minH - 2 * border - lineH) / 2);
    const key = `open:${left}:${top}:${width}:${height}:${font}:${info.align}`;
    if (key === this.lastBox) return;
    this.lastBox = key;
    const s = this.el.style;
    s.left = `${left}px`;
    s.top = `${top}px`;
    s.width = `${width}px`;
    s.height = `${height}px`;
    s.font = font;
    s.lineHeight = `${lineH}px`;
    s.textAlign = info.align;
    s.padding = `${padTop}px ${Math.max(0, pad - border + 1)}px 0`;
    s.opacity = "1";
    s.pointerEvents = "auto";
    s.caretColor = info.theme.colors.accent;
    s.color = info.theme.colors.text;
    s.background = info.theme.colors.editorBg;
    s.border = `${border}px solid ${info.theme.colors.selectionBorder}`;
    s.boxShadow = "0 2px 8px rgba(0, 0, 0, 0.12)";
  }

  /** Applies an `edit` prop from the app. Returns true if the editor (re)opened. */
  sync(edit: EditState | null, prev: EditState | null): boolean {
    if (edit === null) {
      if (this.open && prev !== null) this.end();
      return false;
    }
    let opened = false;
    if (!this.open || !this.cell || this.cell.r !== edit.cell.r || this.cell.c !== edit.cell.c) {
      this.begin(edit.cell, this.open ? this.mode : "edit");
      opened = true;
      this.setValue(edit.text, edit.caret);
      return opened;
    }
    if (!this.composing && edit.text !== this.el.value && !this.wasEmitted(edit.text)) {
      this.setValue(edit.text, edit.caret);
      this.lastBox = "";
    }
    return opened;
  }
}

function clampN(v: number, lo: number, hi: number): number {
  return v < lo ? lo : v > hi ? hi : v;
}
