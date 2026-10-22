/**
 * The unsaved-changes guard: every path that replaces or closes the current
 * document (New, Open…, Open recent, templates, recovery, Close, the window
 * close button, Quit) asks `AppController.confirmUnsaved()` first. When the
 * document is dirty (`WorkbookInfo.dirty` in the doc store) the save prompt
 * opens: "Do you want to save the changes you made to “<title>”?" with
 * Save / Don't Save / Cancel (as in macOS and AOO). In the desktop app each
 * document has its own webview, a tab or a window (windows.ts, tabs.ts), so
 * New/Open/… no longer replace a document there; the guard runs for ⌘W, a
 * tab's ×, the window's close button and, document by document, for Quit.
 *
 * The decision logic is pure and exported for tests; the prompt itself is a
 * small zustand store that `SavePrompt` (components/SavePrompt.tsx) renders.
 */
import type { DocId, WorkbookInfo } from "@tally/bridge";
import { create } from "zustand";

/** The user's answer to the save prompt. */
export type SaveChoice = "save" | "discard" | "cancel";

/** Whether a replace/close can go ahead at once or must ask first. */
export type GuardDecision = "proceed" | "prompt";

/** Pure: a dirty document must be asked about; a clean (or no) document proceeds. */
export function guardDecision(info: Pick<WorkbookInfo, "dirty"> | null | undefined): GuardDecision {
  return info?.dirty ? "prompt" : "proceed";
}

/**
 * Pure: whether the pending action runs after `choice`. Don't Save
 * proceeds; Save proceeds only when the save succeeded (a cancelled Save As
 * dialog or a failed save aborts); Cancel aborts.
 */
export function proceedAfter(choice: SaveChoice, saved: boolean): boolean {
  return choice === "discard" || (choice === "save" && saved);
}

/**
 * What the request from the desktop shell (`close-requested`) asks for:
 * `"window"` closes this one document (its tab, or its window when it is the
 * only tab); `"quit"` and `"tabs"` are steps of a sequence (Quit, the close
 * button of a window with several tabs, Close Other Tabs) in which the shell
 * asks each edited document in turn and closes nothing until all answered.
 */
export type CloseKind = "window" | "quit" | "tabs";

/** Pure: whether a `close-requested` kind is a step of a sequence (answer, then wait for the shell). */
export function isSequence(kind: CloseKind): boolean {
  return kind !== "window";
}

/** Optional desktop-shell hooks (`TauriClient`; the browser has none). */
export interface CloseShell {
  /** `set_document_edited`: the macOS "edited" dot, the shell's close guard flag, the document it snapshots on exit and its title (Window menu). */
  setDocumentEdited?(edited: boolean, doc: DocId | null, title?: string | null): Promise<void>;
  /** `close-requested`: the shell held back closing this document, a quit or a multi-tab close because the document is edited. */
  onCloseRequested?(cb: (kind: CloseKind) => void): () => void;
  /** `confirm_close`: the guard resolved; close this document (`false`), or go on with the sequence (`true`). */
  confirmClose?(quit: boolean): Promise<void>;
  /** `cancel_quit`: the guard was cancelled during a sequence (quit, multi-tab close); it is aborted. */
  cancelQuit?(): Promise<void>;
}

export interface SavePromptState {
  /** The open prompt (document title), or null. */
  prompt: { title: string } | null;
  /** Opens the prompt; resolves with the user's choice. */
  ask(title: string): Promise<SaveChoice>;
  /** Answers the open prompt (the dialog buttons, Escape). */
  answer(choice: SaveChoice): void;
}

let resolver: ((c: SaveChoice) => void) | null = null;

export const useSavePromptStore = create<SavePromptState>()((set) => ({
  prompt: null,
  ask: (title) =>
    new Promise<SaveChoice>((resolve) => {
      // A second request while one is open cancels the first.
      resolver?.("cancel");
      resolver = resolve;
      set({ prompt: { title } });
    }),
  answer: (choice) => {
    const r = resolver;
    resolver = null;
    set({ prompt: null });
    r?.(choice);
  },
}));
