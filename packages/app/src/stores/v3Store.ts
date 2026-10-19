/**
 * v0.3 sheet state the chrome shows besides cells: the active sheet's page
 * setup and page breaks ("View ▸ Page breaks"), protection, whether it has
 * validations (input help), the document locale and the cell popovers
 * (note hover/editor, validation list, input help). Refreshed by
 * `V3Features` (features.ts).
 */
import type { CellPos, NoteDto, PageSetupDto, ProtectionInfo } from "@tally/bridge";
import type { ClientRect, PageBreaks } from "@tally/grid";
import { create } from "zustand";

/** A popover anchored at a cell. */
export type CellPopover =
  | { kind: "note"; cell: CellPos; rect: ClientRect; note: NoteDto; pinned: boolean }
  | { kind: "noteEdit"; cell: CellPos; rect: ClientRect; text: string; exists: boolean; author: string | null }
  | { kind: "list"; cell: CellPos; rect: ClientRect; items: string[]; current: string; chips: Record<string, { bg: number; fg: number }> | null }
  | { kind: "inputHelp"; cell: CellPos; rect: ClientRect; title: string; text: string };

export interface V3State {
  /** "View ▸ Page breaks". */
  pageBreaksOn: boolean;
  /** The active sheet's page setup (null until loaded). */
  pageSetup: PageSetupDto | null;
  /** Lines to draw while `pageBreaksOn`. */
  pageBreaks: PageBreaks | null;
  protection: ProtectionInfo | null;
  /** The active sheet has data validations (input help is looked up on moves). */
  hasValidations: boolean;
  /** The document locale tag. */
  locale: string;
  popover: CellPopover | null;
  setPageBreaksOn(on: boolean): void;
  setPageSetup(p: PageSetupDto | null): void;
  setPageBreaks(b: PageBreaks | null): void;
  setProtection(p: ProtectionInfo | null): void;
  setHasValidations(v: boolean): void;
  setLocale(tag: string): void;
  setPopover(p: CellPopover | null): void;
  reset(): void;
}

export const useV3Store = create<V3State>()((set) => ({
  pageBreaksOn: false,
  pageSetup: null,
  pageBreaks: null,
  protection: null,
  hasValidations: false,
  locale: "en-US",
  popover: null,
  setPageBreaksOn: (pageBreaksOn) => set({ pageBreaksOn }),
  setPageSetup: (pageSetup) => set({ pageSetup }),
  setPageBreaks: (pageBreaks) => set({ pageBreaks }),
  setProtection: (protection) => set({ protection }),
  setHasValidations: (hasValidations) => set({ hasValidations }),
  setLocale: (locale) => set({ locale }),
  setPopover: (popover) => set({ popover }),
  reset: () => set({ pageSetup: null, pageBreaks: null, protection: null, hasValidations: false, popover: null }),
}));
