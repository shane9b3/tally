/**
 * `V3Features`: the v0.3 half of the controller (conditional formatting,
 * data validation, notes, hyperlinks, protection, goal seek, document
 * properties, page setup and breaks, locales, filter refresh, row auto-fit).
 * It talks to the engine only through the {@link AppController} it belongs
 * to (`ctl.apply`, `ctl.client`), so undo, selection and error handling stay
 * in one place.
 */
import {
  isApiError,
  type ApplyResult,
  type CellPos,
  type CfRuleDto,
  type ConditionalFormatDto,
  type DocPropertiesDto,
  type EngineClientV3,
  type GoalSeekRequest,
  type GoalSeekResult,
  type Op,
  type OpError,
  type PageSetupDto,
  type ProtectionOptions,
  type RangeDto,
  type ValidationDto,
} from "@tally/bridge";
import { drawStyleId, type CellButtonKind, type ClientRect } from "@tally/grid";
import { chipColorsFor } from "./palette";
import type { AppController } from "./controller";
import { a1 } from "./dataOps";
import { useDocStore } from "./stores/docStore";
import { useEditStore } from "./stores/editStore";
import { useSelectionStore } from "./stores/selectionStore";
import { useUiStore } from "./stores/uiStore";
import { useV3Store } from "./stores/v3Store";
import {
  autoPageBreaks,
  cellText,
  colorByValueRules,
  defaultPageSetup,
  defaultValidation,
  isColorByValue,
  isInternalLink,
  isOpenableUrl,
  normalizeLink,
  presetRule,
  UI_LOCALES,
  type HighlightPreset,
} from "./v03";
import { t } from "./i18n";

/** Optional shell commands of the desktop app (`TauriClient.openUrl`, `userName`). */
export interface LinkShell {
  openUrl?(url: string): Promise<void>;
  userName?(): Promise<string | null>;
}

/** The cell an op writes first (where a validation or protection refusal points). */
export function opCell(op: Op | undefined): CellPos | null {
  if (!op) return null;
  switch (op.type) {
    case "setCellInput":
    case "setRangeInputs":
    case "setNote":
    case "deleteNote":
    case "setLink":
      return { r: op.r, c: op.c };
    case "paste":
    case "pasteSpecial":
      return { r: op.target.r0, c: op.target.c0 };
    case "fillRange":
      return { r: op.target.r0, c: op.target.c0 };
    case "setArrayFormula":
      return { r: op.range.r0, c: op.range.c0 };
    case "clear":
    case "applyStyle":
    case "setNumberFormat":
      return op.ranges[0] ? { r: op.ranges[0].r0, c: op.ranges[0].c0 } : null;
    default:
      return null;
  }
}

export const PROTECTED_MESSAGE = "This cell is protected. Unprotect the sheet (Tools ▸ Protect Sheet…) to change it.";
export const STRUCTURE_PROTECTED_MESSAGE = "The workbook structure is protected. Unprotect it (Tools ▸ Protect Document…) to change sheets.";
export const SHEET_PROTECTED_MESSAGE = "The sheet is protected. Unprotect it (Tools ▸ Protect Sheet…) to do that.";

/** The toast for a `protected` refusal of `op`, and whether it was a cell edit (lock badge). */
export function protectedMessage(op: Op | undefined): { message: string; cell: boolean } {
  switch (op?.type) {
    case "addSheet":
    case "deleteSheet":
    case "renameSheet":
    case "moveSheet":
      return { message: STRUCTURE_PROTECTED_MESSAGE, cell: false };
    case "setCellInput":
    case "setRangeInputs":
    case "clear":
    case "paste":
    case "pasteSpecial":
    case "fillRange":
    case "setArrayFormula":
    case "moveRange":
    case "replaceAll":
    case "setNote":
    case "deleteNote":
    case "setLink":
    case "clearLink":
    case undefined:
      return { message: PROTECTED_MESSAGE, cell: true };
    case "unprotectSheet":
    case "unprotectDocument":
      // The password dialog says so itself.
      return { message: "", cell: false };
    default:
      return { message: SHEET_PROTECTED_MESSAGE, cell: false };
  }
}

export class V3Features {
  private author: string | null | undefined = undefined;
  private helpSeq = 0;

  constructor(private readonly ctl: AppController) {}

  private get client(): EngineClientV3 {
    return this.ctl.client;
  }

  private get doc(): number {
    return this.ctl.requireDoc();
  }

  private get sheet(): number {
    return this.ctl.sheet;
  }

  private get links(): LinkShell {
    return this.ctl.shell as unknown as LinkShell;
  }

  private toast(kind: "info" | "error", message: string): void {
    useUiStore.getState().pushToast(kind, message);
  }

  // -- sheet state --------------------------------------------------------------

  /** Re-reads protection, page setup and validations of the active sheet. */
  async refreshSheet(): Promise<void> {
    const info = this.ctl.info;
    if (!info) return;
    const sheet = this.sheet;
    const doc = info.doc;
    const [prot, setup, vals] = await Promise.all([
      this.client.protectionInfo(doc, sheet).catch(() => null),
      this.client.pageSetup(doc, sheet).catch(() => null),
      this.client.validations(doc, sheet).catch(() => []),
    ]);
    if (sheet !== this.sheet || this.ctl.info?.doc !== doc) return;
    const st = useV3Store.getState();
    st.setProtection(prot);
    st.setPageSetup(setup);
    st.setHasValidations(vals.length > 0);
    this.computePageBreaks();
  }

  /** After an engine result: refresh what the ops may have changed. */
  afterResult(res: ApplyResult, ops: readonly Op[] | null): void {
    const sheet = this.sheet;
    const touches = (types: Op["type"][]) => ops === null || ops.some((o) => types.includes(o.type));
    if (touches(["protectSheet", "unprotectSheet", "protectDocument", "unprotectDocument", "setPageSetup", "setPageBreak", "addValidation", "updateValidation", "deleteValidation"])) {
      void this.refreshSheet();
    } else if (useV3Store.getState().pageBreaksOn && (res.layoutChanged.includes(sheet) || res.changed.some((c) => c.sheet === sheet))) {
      // The used area (and so the automatic breaks) may have moved; the layout refetch is async.
      setTimeout(() => this.computePageBreaks(), 50);
    }
    useV3Store.getState().setPopover(null);
  }

  /**
   * Engine refusals and warnings of `ops`: validation Stop → a dialog,
   * Warning → a toast with Keep / Undo, Information → a toast, a locked cell
   * → a toast and the lock badge. Returns the indices of `res.errors` it
   * reported.
   */
  reportProblems(res: ApplyResult, ops: readonly Op[]): Set<number> {
    const handled = new Set<number>();
    res.errors.forEach((e, i) => {
      if (e.code === "validation" && !handled.has(-1)) {
        handled.add(i);
        handled.add(-1);
        void this.validationStop(e, ops[e.opIndex]);
      } else if (e.code === "validation") {
        handled.add(i);
      } else if (e.code === "protected") {
        if (![...handled].some((j) => res.errors[j]?.code === "protected")) {
          const op = ops[e.opIndex];
          const p = protectedMessage(op);
          if (p.message) this.toast("error", p.message);
          if (p.cell) this.ctl.grid?.flashLocked?.(opCell(op) ?? this.ctl.selection.active);
        }
        handled.add(i);
      }
    });
    const warning = res.warnings?.[0];
    if (warning) void this.validationWarning(warning, ops[warning.opIndex]);
    return handled;
  }

  private async validationStop(e: OpError, op: Op | undefined): Promise<void> {
    const cell = opCell(op);
    let title = "";
    if (cell) {
      try {
        title = (await this.client.validationAt(this.doc, this.sheet, cell.r, cell.c))?.error?.title ?? "";
      } catch {
        // The message alone is enough.
      }
    }
    const retry = op?.type === "setCellInput" ? op.input : null;
    useUiStore.getState().openDialog({ kind: "validationAlert", title: title || "Invalid value", message: e.message || t("msg.validation.notAllowed", "This value is not allowed here."), cell, retry });
  }

  private async validationWarning(w: OpError, op: Op | undefined): Promise<void> {
    const cell = opCell(op);
    let style: string = "warning";
    let title = "";
    if (cell) {
      try {
        const v = await this.client.validationAt(this.doc, this.sheet, cell.r, cell.c);
        style = v?.error?.style ?? "warning";
        title = v?.error?.title ?? "";
      } catch {
        // Treat as a warning.
      }
    }
    const message = `${title ? `${title}: ` : ""}${w.message}`;
    if (style === "info") {
      useUiStore.getState().pushToast("info", message);
      return;
    }
    useUiStore.getState().pushToast("warning", message, [
      { id: "keep", label: t("msg.toast.keep", "Keep"), run: () => this.ctl.focusGrid() },
      { id: "undo", label: t("cmd.edit.undo", "Undo"), run: () => void this.ctl.undo() },
    ]);
  }

  /** The Stop alert's Retry: edit the cell again with the rejected text. */
  retryEdit(cell: CellPos, text: string): void {
    const sel = { range: { r0: cell.r, c0: cell.c, r1: cell.r, c1: cell.c }, active: cell, anchor: cell };
    useSelectionStore.getState().setSelection(sel);
    this.ctl.grid?.scrollCellIntoView(cell);
    this.ctl.startEdit(cell, text, "grid");
    this.ctl.focusGrid();
  }

  // -- conditional formatting -----------------------------------------------------

  conditionalFormats(): Promise<ConditionalFormatDto[]> {
    return this.client.conditionalFormats(this.doc, this.sheet);
  }

  addConditionalFormat(ranges: RangeDto[], rules: CfRuleDto[], label = "Conditional formatting"): Promise<ApplyResult | null> {
    return this.ctl.apply(label, [{ type: "addConditionalFormat", sheet: this.sheet, ranges: ranges.map((r) => ({ ...r })), rules }]);
  }

  updateConditionalFormat(id: number, ranges: RangeDto[], rules: CfRuleDto[]): Promise<ApplyResult | null> {
    return this.ctl.apply(t("msg.undo.editConditionalFormatting", "Edit conditional formatting"), [{ type: "updateConditionalFormat", sheet: this.sheet, id, ranges: ranges.map((r) => ({ ...r })), rules }]);
  }

  deleteConditionalFormat(id: number): Promise<ApplyResult | null> {
    return this.ctl.apply(t("msg.undo.deleteConditionalFormatting", "Delete conditional formatting"), [{ type: "deleteConditionalFormat", sheet: this.sheet, id }]);
  }

  /**
   * Puts the sheet's conditional formats in a new priority order (one undo
   * step): every format is deleted and re-added in `order`.
   */
  reorderConditionalFormats(order: readonly ConditionalFormatDto[]): Promise<ApplyResult | null> {
    const sheet = this.sheet;
    const ops: Op[] = [
      ...order.map((cf): Op => ({ type: "deleteConditionalFormat", sheet, id: cf.id })),
      ...order.map((cf): Op => ({ type: "addConditionalFormat", sheet, ranges: cf.ranges.map((r) => ({ ...r })), rules: cf.rules })),
    ];
    return this.ctl.apply(t("msg.undo.conditionalFormattingOrder", "Conditional formatting order"), ops);
  }

  /** Toolbar "Highlight cells" preset: applies at once or asks for the value(s). */
  highlight(preset: HighlightPreset): void {
    const ranges = this.ctl.ranges;
    if (preset === "duplicates" || preset === "top10" || preset === "colorScale" || preset === "dataBar" || preset === "iconSet") {
      void this.addConditionalFormat(ranges, [presetRule(preset, [])], "Highlight cells");
      return;
    }
    useUiStore.getState().openDialog({ kind: "cfPreset", preset, ranges });
  }

  /**
   * "Colour by value": one conditional format over the column body with a
   * chip rule per distinct value. Replaces an earlier "Colour by value"
   * format on the same range.
   */
  async colorByValue(body: RangeDto, values: string[], colors: ReadonlyMap<string, { bg: number; fg: number }>): Promise<void> {
    const rules = colorByValueRules(values, colors);
    const existing = (await this.conditionalFormats().catch(() => [])).find(
      (cf) => cf.ranges.length === 1 && sameRange(cf.ranges[0]!, body) && isColorByValue(cf.rules),
    );
    if (existing) await this.ctl.apply(t("cmd.format.colorByValue", "Colour by value"), [{ type: "updateConditionalFormat", sheet: this.sheet, id: existing.id, ranges: [{ ...body }], rules }]);
    else await this.addConditionalFormat([body], rules, "Colour by value");
  }

  // -- data validation --------------------------------------------------------------

  /** Data ▸ Validation…: the validation of the active cell, or a new one for the selection. */
  async openValidation(): Promise<void> {
    try {
      const a = this.ctl.selection.active;
      const list = await this.client.validations(this.doc, this.sheet);
      const found = list.find((v) => v.ranges.some((r) => a.r >= r.r0 && a.r <= r.r1 && a.c >= r.c0 && a.c <= r.c1));
      useUiStore.getState().openDialog(found ? { kind: "validation", validation: structuredClone(found), existing: true } : { kind: "validation", validation: defaultValidation(this.ctl.ranges), existing: false });
    } catch (e) {
      this.ctl.notifyError(e);
    }
  }

  async applyValidation(v: ValidationDto, existing: boolean): Promise<void> {
    const sheet = this.sheet;
    const ops: Op[] = existing ? [{ type: "updateValidation", sheet, id: v.id, validation: v }] : [{ type: "addValidation", sheet, validation: v }];
    await this.ctl.apply(t("v3.dvTitle", "Data validation"), ops);
  }

  async clearValidation(id: number): Promise<void> {
    await this.ctl.apply(t("msg.undo.removeValidation", "Remove validation"), [{ type: "deleteValidation", sheet: this.sheet, id }]);
  }

  /** A cell's ▾ (validation list) or ↗ (link) was clicked. */
  onCellButton = (kind: CellButtonKind, cell: CellPos, rect: ClientRect): void => {
    if (kind === "dropdown") void this.openList(cell, rect);
    else {
      const url = useDocStore.getState().source?.getCell(cell.r, cell.c)?.link;
      if (url) void this.openLink(url);
    }
  };

  /** Opens the list popover of a validation cell. */
  async openList(cell: CellPos, rect: ClientRect): Promise<void> {
    try {
      const v = await this.client.validationAt(this.doc, this.sheet, cell.r, cell.c);
      if (!v || v.kind.type !== "list") return;
      const source = useDocStore.getState().source;
      const view = source?.getCell(cell.r, cell.c);
      const current = view?.text ?? "";
      // A column shown as chips ("Colour by value") lists its items as chips too.
      const chip = source?.getStyle(drawStyleId(view))?.display?.type === "chip";
      const chips = chip ? Object.fromEntries([...chipColorsFor(v.listItems)].map(([k, c]) => [k, { bg: c.bg, fg: c.fg }])) : null;
      useV3Store.getState().setPopover({ kind: "list", cell, rect, items: v.listItems, current, chips });
    } catch (e) {
      this.ctl.notifyError(e);
    }
  }

  /** Picks a list item (one undo step, validated by the engine like typing). */
  async pickListItem(cell: CellPos, item: string): Promise<void> {
    useV3Store.getState().setPopover(null);
    await this.ctl.apply(t("msg.undo.chooseFromList", "Choose from list"), [{ type: "setCellInput", sheet: this.sheet, r: cell.r, c: cell.c, input: item }]);
    this.ctl.focusGrid();
  }

  /** Input help of the active cell's validation (a tooltip next to it). */
  async showInputHelp(cell: CellPos): Promise<void> {
    const seq = ++this.helpSeq;
    const st = useV3Store.getState();
    if (st.popover?.kind === "inputHelp") st.setPopover(null);
    if (!st.hasValidations || !this.ctl.info) return;
    try {
      const v = await this.client.validationAt(this.doc, this.sheet, cell.r, cell.c);
      if (seq !== this.helpSeq || !v?.inputHelp || (v.inputHelp.title === "" && v.inputHelp.text === "")) return;
      const rect = this.ctl.grid?.cellClientRect?.(cell);
      if (!rect || useV3Store.getState().popover) return;
      useV3Store.getState().setPopover({ kind: "inputHelp", cell, rect, title: v.inputHelp.title, text: v.inputHelp.text });
    } catch {
      // Background lookup.
    }
  }

  // -- notes -------------------------------------------------------------------------

  private async userName(): Promise<string | null> {
    if (this.author !== undefined) return this.author;
    try {
      this.author = (await this.links.userName?.()) ?? null;
    } catch {
      this.author = null;
    }
    return this.author;
  }

  /** ⇧F2 / Insert ▸ Note: the note editor at the active cell. */
  async editNote(cell: CellPos = this.ctl.selection.active): Promise<void> {
    try {
      if (useEditStore.getState().edit) return;
      this.ctl.grid?.scrollCellIntoView(cell);
      const [note, author] = await Promise.all([this.client.note(this.doc, this.sheet, cell.r, cell.c), this.userName()]);
      const rect = this.ctl.grid?.cellClientRect?.(cell) ?? { left: 200, top: 200, right: 300, bottom: 224 };
      useV3Store.getState().setPopover({ kind: "noteEdit", cell, rect, text: note?.text ?? "", exists: note !== null, author: note?.author ?? author });
    } catch (e) {
      this.ctl.notifyError(e);
    }
  }

  async saveNote(cell: CellPos, text: string, author: string | null): Promise<void> {
    useV3Store.getState().setPopover(null);
    const sheet = this.sheet;
    if (text.trim() === "") {
      const had = await this.client.note(this.doc, sheet, cell.r, cell.c).catch(() => null);
      if (had) await this.ctl.apply(t("cmd.insert.deleteNote", "Delete note"), [{ type: "deleteNote", sheet, r: cell.r, c: cell.c }]);
    } else {
      await this.ctl.apply(t("popover.note", "Note"), [{ type: "setNote", sheet, r: cell.r, c: cell.c, note: { r: cell.r, c: cell.c, text, author, created: null, shown: false } }]);
    }
    this.ctl.focusGrid();
  }

  async deleteNote(cell: CellPos = this.ctl.selection.active): Promise<void> {
    useV3Store.getState().setPopover(null);
    await this.ctl.apply(t("cmd.insert.deleteNote", "Delete note"), [{ type: "deleteNote", sheet: this.sheet, r: cell.r, c: cell.c }]);
  }

  /** The pointer rests on (or left) a cell with a note. */
  onNoteHover = (cell: CellPos | null, rect: ClientRect | null): void => {
    const st = useV3Store.getState();
    const cur = st.popover;
    if (!cell || !rect) {
      // A moment's grace so the pointer can reach the popover (which pins it).
      if (cur?.kind === "note" && !cur.pinned) {
        setTimeout(() => {
          const now = useV3Store.getState().popover;
          if (now === cur) useV3Store.getState().setPopover(null);
        }, 250);
      }
      return;
    }
    if (cur && cur.kind !== "note" && cur.kind !== "inputHelp") return;
    const info = this.ctl.info;
    if (!info) return;
    const sheet = this.sheet;
    void this.client.note(info.doc, sheet, cell.r, cell.c).then(
      (note) => {
        if (!note || sheet !== this.sheet) return;
        const now = useV3Store.getState().popover;
        if (now && now.kind !== "note" && now.kind !== "inputHelp") return;
        useV3Store.getState().setPopover({ kind: "note", cell, rect, note, pinned: false });
      },
      () => undefined,
    );
  };

  // -- hyperlinks ------------------------------------------------------------------------

  /** ⌥⌘K / Insert ▸ Hyperlink…: URL and display text for the active cell. */
  async openHyperlink(): Promise<void> {
    try {
      const cell = { ...this.ctl.selection.active };
      const view = useDocStore.getState().source?.getCell(cell.r, cell.c);
      const info = await this.client.cellEditInfo(this.doc, this.sheet, cell.r, cell.c).catch(() => null);
      const input = info?.input ?? "";
      const hasText = input !== "" && !input.startsWith("=");
      useUiStore.getState().openDialog({ kind: "hyperlink", cell, url: view?.link ?? "", text: hasText ? input : "", hasText });
    } catch (e) {
      this.ctl.notifyError(e);
    }
  }

  /** Sets the link (and the display text when given) as one undo step. */
  async setLink(cell: CellPos, rawUrl: string, text: string): Promise<boolean> {
    const url = normalizeLink(rawUrl);
    const sheet = this.sheet;
    const ops: Op[] = [];
    const current = await this.client.cellEditInfo(this.doc, sheet, cell.r, cell.c).catch(() => null);
    const display = text.trim() === "" && (current?.input ?? "") === "" ? url : text;
    // Not typed input (AOO inserts a text field, not through EnterData): a 1×1 block, so a
    // leading +/- ("+1-555-1234") stays text instead of becoming a formula (P18).
    if (display !== "" && display !== (current?.input ?? "")) ops.push({ type: "setRangeInputs", sheet, r: cell.r, c: cell.c, rows: [[display]] });
    ops.push({ type: "setLink", sheet, r: cell.r, c: cell.c, url });
    const res = await this.ctl.apply(t("hyperlink.title", "Hyperlink"), ops);
    return res !== null && res.errors.length === 0;
  }

  async removeLink(): Promise<void> {
    await this.ctl.apply(t("cmd.insert.removeHyperlink", "Remove hyperlink"), [{ type: "clearLink", sheet: this.sheet, ranges: this.ctl.ranges }]);
  }

  /** ⌘-click / ↗: `#Sheet.A1` jumps; http(s)/mailto open in the OS; anything else is refused. */
  async openLink(url: string): Promise<void> {
    const t = url.trim();
    if (isInternalLink(t)) {
      await this.ctl.goTo(t.slice(1));
      return;
    }
    if (!isOpenableUrl(t)) {
      this.toast("error", "Only web (http, https) and e-mail links can be opened.");
      return;
    }
    if (!this.links.openUrl) {
      this.toast("info", "Opening links needs the desktop app.");
      return;
    }
    try {
      await this.links.openUrl(t);
    } catch (e) {
      this.ctl.notifyError(e);
    }
  }

  onOpenLink = (_cell: CellPos, url: string): void => {
    void this.openLink(url);
  };

  // -- protection ---------------------------------------------------------------------------

  /** Tools ▸ Protect Sheet…: protect, or (when protected) ask for the password to unprotect. */
  async protectSheetCommand(): Promise<void> {
    try {
      const p = await this.client.protectionInfo(this.doc, this.sheet);
      useV3Store.getState().setProtection(p);
      if (!p.sheetProtected) useUiStore.getState().openDialog({ kind: "protectSheet" });
      else if (p.sheetHasPassword) useUiStore.getState().openDialog({ kind: "unprotectSheet" });
      else await this.unprotectSheet(null);
    } catch (e) {
      this.ctl.notifyError(e);
    }
  }

  async protectSheet(password: string | null, options: ProtectionOptions): Promise<void> {
    const res = await this.ctl.apply(t("msg.undo.protectSheet", "Protect sheet"), [{ type: "protectSheet", sheet: this.sheet, password: password || null, options }]);
    if (res && res.errors.length === 0) this.toast("info", `Protected “${this.ctl.sheetName()}”. Locked cells can't be changed.`);
  }

  async unprotectSheet(password: string | null): Promise<boolean> {
    const res = await this.ctl.apply(t("msg.undo.unprotectSheet", "Unprotect sheet"), [{ type: "unprotectSheet", sheet: this.sheet, password }]);
    const ok = res !== null && res.errors.length === 0;
    if (ok) this.toast("info", `“${this.ctl.sheetName()}” is no longer protected.`);
    return ok;
  }

  async protectDocumentCommand(): Promise<void> {
    try {
      const p = await this.client.protectionInfo(this.doc, this.sheet);
      useUiStore.getState().openDialog({ kind: "protectDocument", protected: p.structureProtected, hasPassword: p.structureHasPassword });
    } catch (e) {
      this.ctl.notifyError(e);
    }
  }

  async setDocumentProtection(protect: boolean, password: string | null): Promise<boolean> {
    const op: Op = protect ? { type: "protectDocument", password: password || null } : { type: "unprotectDocument", password: password || null };
    const res = await this.ctl.apply(protect ? t("v3.protectDoc", "Protect document") : t("v3.unprotectDoc", "Unprotect document"), [op]);
    const ok = res !== null && res.errors.length === 0;
    if (ok) this.toast("info", protect ? "Sheets can no longer be added, deleted, renamed or moved." : "The document structure is no longer protected.");
    return ok;
  }

  // -- goal seek ----------------------------------------------------------------------------------

  openGoalSeek(): void {
    const a = this.ctl.selection.active;
    useUiStore.getState().openDialog({ kind: "goalSeek", formulaCell: cellText(a), target: "", variableCell: "" });
  }

  goalSeek(formulaCell: CellPos, target: number, variableCell: CellPos): Promise<GoalSeekResult> {
    const req: GoalSeekRequest = { formulaSheet: this.sheet, formulaCell, target, variableSheet: this.sheet, variableCell };
    return this.client.goalSeek(this.doc, req);
  }

  /** Accepts a goal seek result: writes the value into the variable cell (undoable). */
  async applyGoalSeek(variableCell: CellPos, value: number): Promise<void> {
    const input = String(Number(value.toPrecision(15)));
    const cell = { ...variableCell };
    await this.ctl.apply(t("v3.goalTitle", "Goal seek"), [{ type: "setCellInput", sheet: this.sheet, r: cell.r, c: cell.c, input }], {
      range: { r0: cell.r, c0: cell.c, r1: cell.r, c1: cell.c },
      active: cell,
      anchor: cell,
    });
  }

  // -- document properties ---------------------------------------------------------------------------

  async openProperties(): Promise<void> {
    try {
      const props = await this.client.documentProperties(this.doc);
      useUiStore.getState().openDialog({ kind: "properties", props });
    } catch (e) {
      this.ctl.notifyError(e);
    }
  }

  async saveProperties(props: DocPropertiesDto): Promise<void> {
    await this.ctl.apply(t("v3.propsTitle", "Document properties"), [{ type: "setDocumentProperties", props }]);
  }

  // -- page setup and breaks ------------------------------------------------------------------------------

  async openPageSetup(): Promise<void> {
    try {
      const setup = await this.client.pageSetup(this.doc, this.sheet).catch(() => defaultPageSetup());
      useUiStore.getState().openDialog({ kind: "pageSetup", setup });
    } catch (e) {
      this.ctl.notifyError(e);
    }
  }

  async savePageSetup(setup: PageSetupDto): Promise<void> {
    const res = await this.ctl.apply(t("v3.pageSetup", "Page setup"), [{ type: "setPageSetup", sheet: this.sheet, setup }]);
    if (res && res.errors.length === 0) useV3Store.getState().setPageSetup(setup);
  }

  /** Insert ▸ Page Break: above the active row (`row`) or left of the active column (`col`). */
  async insertPageBreak(axis: "row" | "col"): Promise<void> {
    const a = this.ctl.selection.active;
    const at = axis === "row" ? a.r : a.c;
    if (at === 0) {
      this.toast("info", axis === "row" ? "A page break goes above a row below row 1." : "A page break goes left of a column right of A.");
      return;
    }
    await this.ctl.apply(t("msg.undo.insertPageBreak", "Insert page break"), [{ type: "setPageBreak", sheet: this.sheet, axis, at, present: true }]);
    if (!useV3Store.getState().pageBreaksOn) this.setPageBreaksOn(true);
  }

  /** Removes the manual breaks above the active row and left of the active column. */
  async removePageBreaks(): Promise<void> {
    const a = this.ctl.selection.active;
    const setup = useV3Store.getState().pageSetup ?? (await this.client.pageSetup(this.doc, this.sheet).catch(() => null));
    const ops: Op[] = [];
    if (setup?.rowBreaks.includes(a.r)) ops.push({ type: "setPageBreak", sheet: this.sheet, axis: "row", at: a.r, present: false });
    if (setup?.colBreaks.includes(a.c)) ops.push({ type: "setPageBreak", sheet: this.sheet, axis: "col", at: a.c, present: false });
    if (ops.length === 0) {
      this.toast("info", "There is no manual page break at the active cell.");
      return;
    }
    await this.ctl.apply(t("msg.undo.removePageBreak", "Remove page break"), ops);
  }

  setPageBreaksOn(on: boolean): void {
    useV3Store.getState().setPageBreaksOn(on);
    if (on) void this.refreshSheet();
    else useV3Store.getState().setPageBreaks(null);
  }

  /** Recomputes the page-break lines from the page setup and the layout. */
  computePageBreaks(): void {
    const st = useV3Store.getState();
    if (!st.pageBreaksOn) {
      if (st.pageBreaks) st.setPageBreaks(null);
      return;
    }
    const setup = st.pageSetup ?? defaultPageSetup();
    const layout = useDocStore.getState().source?.getLayout(this.sheet);
    const area = setup.printRange ?? layout?.used ?? null;
    if (!layout || !area) {
      st.setPageBreaks({ rows: setup.rowBreaks.map((at) => ({ at, manual: true })), cols: setup.colBreaks.map((at) => ({ at, manual: true })) });
      return;
    }
    const auto = autoPageBreaks(setup, area, { runs: layout.rowRuns, dflt: layout.defaultRowHeightPx }, { runs: layout.colRuns, dflt: layout.defaultColWidthPx });
    const merge = (autoAt: number[], manual: number[]) => {
      const m = new Set(manual);
      const all = new Set([...autoAt, ...manual]);
      return [...all].sort((a, b) => a - b).map((at) => ({ at, manual: m.has(at) }));
    };
    st.setPageBreaks({ rows: merge(auto.rows, setup.rowBreaks), cols: merge(auto.cols, setup.colBreaks) });
  }

  // -- locale, filter, rows ---------------------------------------------------------------------------

  async setLocale(tag: string): Promise<void> {
    try {
      const res = await this.client.setLocale(this.doc, tag);
      useV3Store.getState().setLocale(tag);
      this.ctl.handleResult(res);
      const label = UI_LOCALES.find((l) => l.tag === tag)?.label ?? tag;
      this.toast("info", `Locale: ${label}. Numbers and dates now follow it.`);
    } catch (e) {
      this.ctl.notifyError(e);
    }
  }

  async refreshFilter(): Promise<void> {
    await this.ctl.apply(t("cmd.data.refreshFilter", "Refresh filter"), [{ type: "refreshFilter", sheet: this.sheet }]);
    await this.ctl.refreshFilter();
  }

  /** Format ▸ Fit Row Height: wrapped / multi-line rows of the selection get the height they need. */
  async fitRows(): Promise<void> {
    const ops: Op[] = this.ctl.ranges.map((r) => ({ type: "autoFitRows", sheet: this.sheet, start: r.r0, end: Math.min(r.r1, r.r0 + 100_000) }));
    await this.ctl.apply(t("msg.undo.fitRowHeight", "Fit row height"), ops);
  }

  /** Describes a selection for dialogs (`F2:F15`). */
  selectionText(): string {
    return this.ctl.ranges.map(a1).join(", ");
  }
}

function sameRange(a: RangeDto, b: RangeDto): boolean {
  return a.r0 === b.r0 && a.c0 === b.c0 && a.r1 === b.r1 && a.c1 === b.c1;
}

/** Narrows an engine rejection to its code. */
export function errorCode(e: unknown): string | null {
  return isApiError(e) ? e.code : null;
}
