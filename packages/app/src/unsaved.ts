/**
 * The unsaved-changes guard: every path that replaces or closes the current
 * document (New, Open…, Open recent, templates, recovery, Close, the window
 * close button, Quit) asks `AppController.confirmUnsaved()` first. When the
 * document is dirty (`WorkbookInfo.dirty` in the doc store) the save prompt
 * opens: "Do you want to save the changes you made to “<title>”?" with
 * Save / Don't Save / Cancel (as in macOS and AOO).
 *
 * The decision logic is pure and exported for tests; the prompt itself is a
 * small zustand store that `SavePrompt` (components/SavePrompt.tsx) renders.
 */
import type { WorkbookInfo } from "@tally/bridge";
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

/** What the window/quit request from the desktop shell (`close-requested`) asks for. */
export type CloseKind = "window" | "quit";

/** Optional desktop-shell hooks (`TauriClient`; the browser has none). */
export interface CloseShell {
  /** `set_document_edited`: the macOS "edited" dot and the shell's close guard flag. */
  setDocumentEdited?(edited: boolean): Promise<void>;
  /** `close-requested`: the shell held back a window close / quit because the document is edited. */
  onCloseRequested?(cb: (kind: CloseKind) => void): () => void;
  /** `confirm_close`: the guard resolved; close the window or quit for real. */
  confirmClose?(quit: boolean): Promise<void>;
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
