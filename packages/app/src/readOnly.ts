/**
 * Read-only mode and document locks (AOO 4.1: `SfxMedium` read-only,
 * `SID_EDITDOC` "Edit File", `SfxMedium::LockOrigFileOnDemand`).
 *
 * - The engine refuses every edit of a read-only document (`read_only`);
 *   here a {@link CommandGate} disables the editing commands with AOO's
 *   message, typing in a cell shows it, and the banner offers Edit
 *   Document.
 * - Opening a file someone else is editing fails with `locked` (the
 *   shell read their `.~lock.<name>#`); {@link ReadOnlyFeature.resolveLockedOpen}
 *   asks AOO's "Document in Use" question and opens again with the answer.
 * - Edit ▸ Edit Mode ({@link ReadOnlyFeature.toggleEditMode}) switches; to
 *   read-only it first asks to save (as AOO); to edit, a lock or an
 *   unwritable file asks again.
 */
import { create } from "zustand";
import { LOCKED, NOT_WRITABLE, isApiError, type DocId, type EditModeOption, type EditModeResult, type LockInfo, type OpenMode, type WorkbookInfo } from "@tally/bridge";
import { t } from "./i18n";
import type { CommandGate } from "./registry";
import { useDocStore } from "./stores/docStore";
import { useUiStore } from "./stores/uiStore";
import { useSavePromptStore } from "./unsaved";

/**
 * Commands that still run on a read-only document: viewing, selecting,
 * copying, finding, printing and exporting, zoom, windows, Save As (an
 * editable copy) and Edit Mode itself. Everything else is disabled (an
 * allowlist, so a new command is safe by default).
 */
export const READ_ONLY_COMMANDS: ReadonlySet<string> = new Set([
  "file.new",
  "file.open",
  "file.openReadOnly",
  "file.templates",
  "file.clearRecent",
  "file.saveAs",
  "file.saveAsTemplate",
  "file.exportXlsx",
  "file.exportOds",
  "file.exportCsv",
  "file.exportPdf",
  "file.print",
  "file.printPreview",
  "file.close",
  "file.documentStats",
  "edit.copy",
  "edit.cancelClip",
  "edit.find",
  "edit.findNext",
  "edit.findPrev",
  "edit.focusNameBox",
  "edit.selectArray",
  "edit.selectData",
  "edit.editMode",
  "sheet.next",
  "sheet.prev",
  "sheet.selectAll",
  "sheet.deselectAll",
  "sheet.selectNext",
  "sheet.selectPrev",
  "view.toggleSidebar",
  "view.toolbar",
  "view.formattingBar",
  "view.formulaBar",
  "view.statusBar",
  "view.theme",
  "view.fullScreen",
  "view.glance",
  "view.navigator",
  "view.customizeToolbars",
  "view.pageBreaks",
  "view.pageBreakPreview",
  "palette.toggle",
  "tools.options",
  "tools.tracePrecedents",
  "tools.traceDependents",
  "tools.removeTraces",
  "tools.markInvalid",
  "data.recalc",
  "chart.copyImage",
]);

/** Command id prefixes allowed on a read-only document (zoom, windows, help). */
const READ_ONLY_PREFIXES = ["view.zoom", "window.", "help."];

/** `true` if command `id` may run on a read-only document. */
export function allowedWhileReadOnly(id: string): boolean {
  return READ_ONLY_COMMANDS.has(id) || READ_ONLY_PREFIXES.some((p) => id.startsWith(p));
}

/** AOO's `STR_READONLYERR`. */
export function readOnlyMessage(): string {
  return t("msg.readOnly", "Document opened in read-only mode.");
}

/** The gate that disables editing commands while `active()` (the document is read-only). */
export function readOnlyGate(active: () => boolean): CommandGate {
  return { active, allow: allowedWhileReadOnly, reason: readOnlyMessage };
}

/** The window/tab title: AOO appends " (read-only)" (`STR_READONLY`). */
export function displayTitle(info: Pick<WorkbookInfo, "title" | "readOnly">): string {
  return info.readOnly ? `${info.title}${t("app.readOnlySuffix", " (read-only)")}` : info.title;
}

/** The lock carried by a `locked` error, or `null` for any other error. */
export function parseLock(e: unknown): LockInfo | null {
  if (!isApiError(e) || e.code !== LOCKED) return null;
  try {
    const v = JSON.parse(e.message) as Partial<LockInfo>;
    if (typeof v.path !== "string") return null;
    return { file: String(v.file ?? ""), path: v.path, owner: String(v.owner ?? ""), time: String(v.time ?? ""), own: v.own === true };
  } catch {
    return null;
  }
}

/** Wraps text from a lock file in Unicode isolates (FSI … PDI) for plain-text messages. */
const isolate = (s: string): string => `\u2068${s}\u2069`;

/** A save or open refused by someone else's lock, for a toast (AOO's lock wording). */
export function lockedMessage(lock: LockInfo): string {
  return t("msg.lockedFile", "Document file '{file}' is locked for editing by {owner}.", { file: isolate(lock.file), owner: isolate(lockOwner(lock)) });
}

/** Who holds the lock, as AOO shows it: `name ( date )`, or "Unknown User". */
export function lockOwner(lock: LockInfo): string {
  if (!lock.owner) return t("dialog.locked.unknownUser", "Unknown User");
  return lock.time ? `${lock.owner} ( ${lock.time} )` : lock.owner;
}

/**
 * Which question: `open` / `openOwn` when opening (AOO's
 * `STR_OPENLOCKED_MSG` / `STR_ALREADYOPEN_MSG`), `edit` / `editOwn` when
 * switching to Edit Mode, `notWritable` (AOO's `MSG_QUERY_OPENASTEMPLATE`).
 */
export type LockPromptKind = "open" | "openOwn" | "edit" | "editOwn" | "notWritable";

/** The user's answer. `open`: ignore our own lock ("Open"); `copy`: edit a copy. */
export type LockAnswer = "readOnly" | "copy" | "open" | "cancel";

/** A piece of a prompt message: plain text, or a value from the lock file shown in a `<bdi>`. */
export type PromptPart = string | { bdi: string };

/** The prompt's text: title and message (AOO's wording). */
export function lockPromptText(kind: LockPromptKind, lock: LockInfo | null): { title: string; message: string } {
  const { title, parts } = lockPromptParts(kind, lock);
  return { title, message: parts.map((p) => (typeof p === "string" ? p : p.bdi)).join("") };
}

/**
 * The prompt's message split so that the file name and the owner (text
 * from someone else's lock file) render in `<bdi>` elements and cannot
 * reorder the sentence around them.
 */
export function lockPromptParts(kind: LockPromptKind, lock: LockInfo | null): { title: string; parts: PromptPart[] } {
  const mark = (name: string) => `\u0001${name}\u0001`;
  const values: Record<string, string> = {};
  const { title, message } = lockPromptRaw(kind, lock, (name, value) => {
    values[name] = value;
    return mark(name);
  });
  const parts: PromptPart[] = [];
  message.split(/\u0001(\w+)\u0001/).forEach((piece, i) => {
    if (i % 2 === 1) parts.push({ bdi: values[piece] ?? "" });
    else if (piece) parts.push(piece);
  });
  return { title, parts };
}

function lockPromptRaw(kind: LockPromptKind, lock: LockInfo | null, v: (name: string, value: string) => string): { title: string; message: string } {
  const file = v("file", lock?.file ?? "");
  if (kind === "notWritable") {
    return {
      title: t("dialog.editCopy.title", "Edit File"),
      message: t("dialog.editCopy.message", "This document cannot be edited, possibly due to missing access rights. Do you want to edit a copy of the document?"),
    };
  }
  const title = t("dialog.locked.title", "Document in Use");
  if (kind === "openOwn" || kind === "editOwn") {
    return {
      title,
      message: t("dialog.locked.ownMessage", "Document file '{file}' is locked for editing by yourself on a different system since {time}\n\nOpen document read-only, or ignore own file locking and open the document for editing.", { file, time: v("time", lock?.time ?? "") }),
    };
  }
  return {
    title,
    message: t("dialog.locked.message", "Document file '{file}' is locked for editing by:\n\n{owner}\n\nOpen document read-only or open a copy of the document for editing.", { file, owner: v("owner", lock ? lockOwner(lock) : t("dialog.locked.unknownUser", "Unknown User")) }),
  };
}

/** The buttons of each prompt, the default first (AOO's order follows in the dialog). */
export function lockPromptButtons(kind: LockPromptKind): LockAnswer[] {
  switch (kind) {
    case "open":
      return ["readOnly", "copy", "cancel"];
    case "openOwn":
      return ["readOnly", "open", "cancel"];
    case "edit":
      return ["copy", "cancel"];
    case "editOwn":
      return ["open", "cancel"];
    case "notWritable":
      return ["copy", "cancel"];
  }
}

interface LockPromptState {
  prompt: { kind: LockPromptKind; lock: LockInfo | null } | null;
  ask(kind: LockPromptKind, lock: LockInfo | null): Promise<LockAnswer>;
  answer(a: LockAnswer): void;
}

let resolver: ((a: LockAnswer) => void) | null = null;

/** The "Document in Use" / "edit a copy" prompt (components/LockPrompt.tsx). */
export const useLockPromptStore = create<LockPromptState>()((set) => ({
  prompt: null,
  ask: (kind, lock) =>
    new Promise<LockAnswer>((resolve) => {
      resolver?.("cancel");
      resolver = resolve;
      set({ prompt: { kind, lock } });
    }),
  answer: (a) => {
    const r = resolver;
    resolver = null;
    set({ prompt: null });
    r?.(a);
  },
}));

/** The open mode for an answer to an open prompt (`null`: cancelled). */
export function openModeFor(a: LockAnswer): OpenMode | null {
  switch (a) {
    case "readOnly":
      return "readOnly";
    case "copy":
      return "copy";
    case "open":
      return "ignoreLock";
    case "cancel":
      return null;
  }
}

/** What the feature needs from the controller. */
export interface ReadOnlyHost {
  readonly info: WorkbookInfo | null;
  readonly shell: {
    openWith?(path: string, mode: OpenMode): Promise<WorkbookInfo>;
    setReadOnly?(doc: DocId, readOnly: boolean, mode?: EditModeOption | null): Promise<EditModeResult>;
  };
  /** Commits or ends a cell edit; `false` when refused. */
  settleEdits(): Promise<boolean>;
  /** Save (Save As when untitled); the document's info afterwards. */
  save(saveAs: boolean): Promise<void>;
  /** Shows a reloaded document. */
  load(info: WorkbookInfo): void;
  notifyError(e: unknown): void;
  focusGrid(): void;
}

/** Read-only mode in the UI (see the module docs). */
export class ReadOnlyFeature {
  private lastRefusal = 0;

  constructor(private readonly host: ReadOnlyHost) {}

  /** The shown document is read-only. */
  get active(): boolean {
    return useDocStore.getState().info?.readOnly === true;
  }

  /** Edit Mode can switch: the document has a file and the shell can switch it (AOO: `HasName()`). */
  get canToggle(): boolean {
    const info = useDocStore.getState().info;
    return info !== null && info.path !== null && this.host.shell.setReadOnly !== undefined;
  }

  /**
   * An edit attempt on a read-only document (typing in a cell): AOO's
   * message, at most once a second. Returns `true` when refused.
   */
  refuseEdit(quiet = false): boolean {
    if (!this.active) return false;
    const now = Date.now();
    if (!quiet && now - this.lastRefusal > 1000) {
      this.lastRefusal = now;
      useUiStore.getState().pushToast("info", readOnlyMessage());
    }
    return true;
  }

  /**
   * After an open failed with `locked`: asks AOO's "Document in Use"
   * question and opens again with the answer. `null` when cancelled;
   * rethrows any other error.
   */
  async resolveLockedOpen(e: unknown): Promise<WorkbookInfo | null> {
    const lock = parseLock(e);
    const openWith = this.host.shell.openWith;
    if (!lock || !openWith) throw e;
    const mode = openModeFor(await useLockPromptStore.getState().ask(lock.own ? "openOwn" : "open", lock));
    if (mode === null) return null;
    return openWith.call(this.host.shell, lock.path, mode);
  }

  /** Edit ▸ Edit Mode / the banner's Edit Document. */
  toggleEditMode = async (): Promise<void> => {
    if (!this.canToggle) return;
    if (this.active) await this.toEdit(null);
    else await this.toReadOnly();
  };

  /** To read-only: AOO saves or discards first (`PrepareClose`), then reloads read-only. */
  private async toReadOnly(): Promise<void> {
    if (!(await this.host.settleEdits())) return;
    const info = this.host.info;
    if (!info) return;
    let mode: EditModeOption | null = null;
    if (info.dirty) {
      const choice = await useSavePromptStore.getState().ask(info.title);
      if (choice === "cancel") return;
      if (choice === "save") {
        await this.host.save(false);
        if (this.host.info?.dirty !== false) return;
      } else {
        mode = "discard";
      }
    }
    await this.switch(true, mode);
  }

  /** To edit: a lock or an unwritable file asks (Open Copy / Open / Cancel). */
  private async toEdit(mode: EditModeOption | null): Promise<void> {
    try {
      await this.switch(false, mode, true);
    } catch (e) {
      const lock = parseLock(e);
      let answer: LockAnswer;
      if (lock) answer = await useLockPromptStore.getState().ask(lock.own ? "editOwn" : "edit", lock);
      else if (isApiError(e) && e.code === NOT_WRITABLE) answer = await useLockPromptStore.getState().ask("notWritable", null);
      else {
        this.host.notifyError(e);
        return;
      }
      if (answer === "copy") await this.switch(false, "copy");
      else if (answer === "open") await this.switch(false, "ignoreLock");
    }
  }

  private async switch(readOnly: boolean, mode: EditModeOption | null, rethrowAsk = false): Promise<void> {
    const info = this.host.info;
    const setReadOnly = this.host.shell.setReadOnly;
    if (!info || !setReadOnly) return;
    let res: EditModeResult;
    try {
      res = await setReadOnly.call(this.host.shell, info.doc, readOnly, mode);
    } catch (e) {
      if (rethrowAsk && isApiError(e) && (e.code === LOCKED || e.code === NOT_WRITABLE)) throw e;
      this.host.notifyError(e);
      return;
    }
    if (res.reloaded) {
      this.host.load(res.info);
      if (!readOnly) useUiStore.getState().pushToast("info", t("msg.reloadedFromDisk", "The document changed on disk and was reloaded."));
    } else {
      useDocStore.getState().setInfo(res.info);
    }
    this.host.focusGrid();
  }
}
