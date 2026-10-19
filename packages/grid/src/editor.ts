/**
 * The in-cell editor host: one `<textarea>` that is always focused while the
 * grid is. When idle it is invisible and parked over the active cell, so
 * typing and IME composition start there naturally (and the IME candidate
 * window appears next to the cell); when editing it becomes the visible
 * editor, styled like the cell, growing to the right as text is typed.
 *
 * Formula references are coloured with a "mirror": a div behind the
 * textarea with the same box, font and text, whose spans carry the colours;
 * while highlights are shown the textarea's own text is transparent (the
 * caret and selection stay native).
 */
import type { CellPos, StyleDto } from "@tally/bridge";
import type { Theme } from "@tally/tokens";
import type { Geometry } from "./geometry";
import { CELL_PADDING, cellFont, fontPx, type TextMeasurer } from "./renderer";
import type { EditState, TextHighlight } from "./types";

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
  /** Coloured copy of the text, drawn behind the textarea. */
  readonly mirror: HTMLDivElement;
  private highlights: TextHighlight[] = [];
  private mirrorKey = "";
  private textColor = "";
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
    const m = doc.createElement("div");
    m.setAttribute("aria-hidden", "true");
    Object.assign(m.style, {
      position: "absolute",
      zIndex: "3",
      overflow: "hidden",
      whiteSpace: "pre",
      boxSizing: "border-box",
      pointerEvents: "none",
      display: "none",
    } satisfies Partial<CSSStyleDeclaration>);
    this.mirror = m;
    ta.style.zIndex = "4";
    ta.addEventListener("scroll", () => this.syncScroll());
    ta.addEventListener("input", () => this.renderMirror());
    this.setHidden();
  }

  /** Adds the mirror and the textarea to the grid root. */
  mount(root: HTMLElement): void {
    root.append(this.mirror, this.el);
  }

  /** Sets the coloured spans (formula references) of the editor text. */
  setHighlights(spans: readonly TextHighlight[]): void {
    this.highlights = [...spans].sort((a, b) => a.start - b.start);
    this.lastBox = "";
    this.renderMirror();
  }

  private get mirrored(): boolean {
    return this.open && this.highlights.length > 0;
  }

  /** Rebuilds the mirror's text runs (cheap no-op when unchanged). */
  private renderMirror(): void {
    const m = this.mirror;
    if (!this.mirrored) {
      if (m.style.display !== "none") m.style.display = "none";
      this.mirrorKey = "";
      if (this.open) this.el.style.color = this.textColor;
      return;
    }
    const text = this.el.value;
    const key = `${text}\u0000${this.highlights.map((h) => `${h.start}:${h.end}:${h.color}`).join(",")}`;
    if (key !== this.mirrorKey) {
      this.mirrorKey = key;
      const doc = m.ownerDocument;
      m.textContent = "";
      let at = 0;
      for (const h of this.highlights) {
        const s0 = Math.max(at, Math.min(text.length, h.start));
        const s1 = Math.max(s0, Math.min(text.length, h.end));
        if (s0 > at) m.append(doc.createTextNode(text.slice(at, s0)));
        if (s1 > s0) {
          const span = doc.createElement("span");
          span.textContent = text.slice(s0, s1);
          span.style.color = h.color;
          m.append(span);
        }
        at = Math.max(at, s1);
      }
      if (at < text.length) m.append(doc.createTextNode(text.slice(at)));
      // A trailing newline needs a character after it to take up a line.
      if (text.endsWith("\n")) m.append(doc.createTextNode(" "));
    }
    m.style.display = "block";
    this.el.style.color = "transparent";
    this.syncScroll();
  }

  private syncScroll(): void {
    if (!this.mirrored) return;
    this.mirror.scrollLeft = this.el.scrollLeft;
    this.mirror.scrollTop = this.el.scrollTop;
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
    this.renderMirror();
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
    this.mirror.style.display = "none";
    this.mirrorKey = "";
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
    this.textColor = info.theme.colors.text;
    s.border = `${border}px solid ${info.theme.colors.selectionBorder}`;
    s.boxShadow = "0 2px 8px rgba(0, 0, 0, 0.12)";
    // The mirror sits behind the textarea with the same box and paints the background.
    const m = this.mirror.style;
    m.left = s.left;
    m.top = s.top;
    m.width = s.width;
    m.height = s.height;
    m.font = font;
    m.lineHeight = s.lineHeight;
    m.textAlign = info.align;
    m.padding = s.padding;
    m.border = `${border}px solid transparent`;
    m.color = info.theme.colors.text;
    m.background = info.theme.colors.editorBg;
    s.background = this.highlights.length > 0 ? "transparent" : info.theme.colors.editorBg;
    this.renderMirror();
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
    const forced = (edit.seq ?? 0) !== (prev?.seq ?? 0);
    if (forced && !this.composing) {
      this.setValue(edit.text, edit.caret);
      this.lastBox = "";
    } else if (!this.composing && edit.text !== this.el.value && !this.wasEmitted(edit.text)) {
      this.setValue(edit.text, edit.caret);
      this.lastBox = "";
    }
    return opened;
  }
}

function clampN(v: number, lo: number, hi: number): number {
  return v < lo ? lo : v > hi ? hi : v;
}
