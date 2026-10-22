/**
 * The unsaved-changes guard (unsaved.ts, AppController.confirmUnsaved):
 * the pure decision, the prompt store, every replace/close path through the
 * controller with the browser mock, the window/quit hand-off to the shell
 * (`close-requested` → `confirm_close`), the dirty-state mirror
 * (`set_document_edited`) and uncommitted typing (committed first, as in AOO).
 */
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { CANCELLED, type DocId, type WorkbookInfo } from "@tally/bridge";
import { CLOSE_SHELL_COMMANDS, COMMANDS, EVENTS, TauriClient, type InvokeArgs, type TauriEvent } from "@tally/bridge/tauri";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AppController } from "../src/controller";
import { WebMockClient } from "../src/dev/webMock";
import { useDocStore } from "../src/stores/docStore";
import { isPendingEdit, useEditStore } from "../src/stores/editStore";
import { useUiStore } from "../src/stores/uiStore";
import { guardDecision, isSequence, proceedAfter, useSavePromptStore, type CloseKind, type SaveChoice } from "../src/unsaved";

const tick = () => new Promise((r) => setTimeout(r, 0));

/** Waits until the save prompt is open (or fails after a few ticks). */
async function promptOpen(): Promise<string> {
  for (let i = 0; i < 20; i++) {
    const p = useSavePromptStore.getState().prompt;
    if (p) return p.title;
    await tick();
  }
  throw new Error("the save prompt did not open");
}

const answer = (c: SaveChoice) => useSavePromptStore.getState().answer(c);

type Shell = {
  setDocumentEdited: (edited: boolean, doc: DocId | null) => Promise<void>;
  confirmClose: (quit: boolean) => Promise<void>;
  cancelQuit: () => Promise<void>;
  onCloseRequested: (cb: (kind: CloseKind) => void) => () => void;
};

async function controller(withShell = false) {
  const client = new WebMockClient();
  const edited: boolean[] = [];
  const editedDocs: (DocId | null)[] = [];
  const confirms: boolean[] = [];
  let cancels = 0;
  let closeCb: ((kind: CloseKind) => void) | null = null;
  if (withShell) {
    Object.assign(client, {
      setDocumentEdited: (e: boolean, doc: DocId | null) => {
        edited.push(e);
        editedDocs.push(doc);
        return Promise.resolve();
      },
      confirmClose: (quit: boolean) => {
        confirms.push(quit);
        return Promise.resolve();
      },
      cancelQuit: () => {
        cancels++;
        return Promise.resolve();
      },
      onCloseRequested: (cb: (kind: CloseKind) => void) => {
        closeCb = cb;
        return () => {
          closeCb = null;
        };
      },
    } satisfies Shell);
  }
  // The browser mock has no recovery snapshots: open a blank workbook instead.
  Object.assign(client, { recoverSnapshot: () => client.newWorkbook(null) });
  const ctl = new AppController(client, client, { nativeMenu: false });
  const detach = ctl.attach();
  await ctl.start(null);
  await tick();
  // "Opening" a recent file in the mock: a new workbook titled after the path.
  const opened: string[] = [];
  client.open = async (path: string): Promise<WorkbookInfo> => {
    opened.push(path);
    const info = await client.newWorkbook("budget");
    return { ...info, title: path.split("/").pop() ?? path, path };
  };
  const edit = async (text = "edited") => {
    await ctl.apply("Typing", [{ type: "setCellInput", sheet: ctl.sheet, r: 0, c: 0, input: text }]);
    expect(ctl.info?.dirty).toBe(true);
  };
  const a1 = async () => (await client.cells(ctl.info!.doc, ctl.sheet, [{ r0: 0, c0: 0, r1: 0, c1: 0 }]))[0]?.cells[0]?.text ?? "";
  /** Types into A1 (in-cell editor or formula bar) without committing. */
  const type = (text: string, origin: "grid" | "bar" = "grid") => {
    ctl.startEdit({ r: 0, c: 0 }, text, origin);
    expect(ctl.pendingEdit).toBe(true);
  };
  return {
    client,
    ctl,
    detach,
    opened,
    edit,
    type,
    a1,
    edited,
    editedDocs,
    confirms,
    cancels: () => cancels,
    fireClose: (k: CloseKind) => closeCb?.(k),
  };
}

beforeEach(() => {
  useDocStore.getState().reset();
  useEditStore.getState().reset();
  useUiStore.getState().reset();
  answer("cancel");
});

afterEach(() => {
  answer("cancel");
  vi.restoreAllMocks();
});

describe("guard decision (pure)", () => {
  it("prompts only for a dirty document", () => {
    expect(guardDecision(null)).toBe("proceed");
    expect(guardDecision(undefined)).toBe("proceed");
    expect(guardDecision({ dirty: false })).toBe("proceed");
    expect(guardDecision({ dirty: true })).toBe("prompt");
  });

  it("proceeds after Don't Save, after a successful Save, never after Cancel", () => {
    expect(proceedAfter("discard", false)).toBe(true);
    expect(proceedAfter("save", true)).toBe(true);
    expect(proceedAfter("save", false)).toBe(false);
    expect(proceedAfter("cancel", false)).toBe(false);
    expect(proceedAfter("cancel", true)).toBe(false);
  });

  it("the prompt store resolves with the answer; a second ask cancels the first", async () => {
    const first = useSavePromptStore.getState().ask("A");
    const second = useSavePromptStore.getState().ask("B");
    expect(await first).toBe("cancel");
    expect(useSavePromptStore.getState().prompt).toEqual({ title: "B" });
    answer("discard");
    expect(await second).toBe("discard");
    expect(useSavePromptStore.getState().prompt).toBeNull();
  });
});

describe("open recent with unsaved changes", () => {
  it("a clean document is replaced without a prompt", async () => {
    const { ctl, opened } = await controller();
    await ctl.openRecent("/x/Other.ods");
    expect(useSavePromptStore.getState().prompt).toBeNull();
    expect(opened).toEqual(["/x/Other.ods"]);
    expect(ctl.info?.title).toBe("Other.ods");
  });

  it("Cancel keeps the edited document", async () => {
    const { ctl, opened, edit, a1 } = await controller();
    await edit("keep me");
    const doc = ctl.info!.doc;
    const run = ctl.openRecent("/x/Other.ods");
    expect(await promptOpen()).toBe(ctl.info!.title);
    answer("cancel");
    await run;
    expect(opened).toEqual([]);
    expect(ctl.info?.doc).toBe(doc);
    expect(ctl.info?.dirty).toBe(true);
    expect(await a1()).toBe("keep me");
  });

  it("Don't Save opens the other file", async () => {
    const { ctl, client, opened, edit } = await controller();
    await edit();
    const close = vi.spyOn(client, "close");
    const save = vi.spyOn(client, "saveWithDialog");
    const run = ctl.openRecent("/x/Other.ods");
    await promptOpen();
    answer("discard");
    await run;
    expect(save).not.toHaveBeenCalled();
    expect(opened).toEqual(["/x/Other.ods"]);
    expect(ctl.info?.title).toBe("Other.ods");
    // The edited document was closed (its autosave snapshot goes with it in the shell).
    expect(close).toHaveBeenCalled();
  });

  it("Save on a titled document saves it, then opens the other", async () => {
    const { ctl, client, opened, edit } = await controller();
    await ctl.save(false); // gives the document a file ("Untitled.ods" in the mock)
    expect(ctl.info?.path).not.toBeNull();
    await edit();
    const doc = ctl.info!.doc;
    const save = vi.spyOn(client, "saveWithDialog");
    const run = ctl.openRecent("/x/Other.ods");
    await promptOpen();
    answer("save");
    await run;
    expect(save).toHaveBeenCalledWith(doc, false, null);
    await expect(save.mock.results[0]?.value).resolves.toMatchObject({ dirty: false });
    expect(opened).toEqual(["/x/Other.ods"]);
  });

  it("Save on an untitled document goes through Save As and aborts if it is cancelled", async () => {
    const { ctl, client, opened, edit, a1 } = await controller();
    expect(ctl.info?.path).toBeNull();
    await edit("draft");
    const doc = ctl.info!.doc;
    const save = vi.spyOn(client, "saveWithDialog").mockRejectedValue({ code: CANCELLED, message: "cancelled" });
    const run = ctl.openRecent("/x/Other.ods");
    await promptOpen();
    answer("save");
    await run;
    // saveWithDialog shows the Save dialog for a document without a file.
    expect(save).toHaveBeenCalledWith(doc, false, null);
    expect(opened).toEqual([]);
    expect(ctl.info?.doc).toBe(doc);
    expect(ctl.info?.dirty).toBe(true);
    expect(await a1()).toBe("draft");
    // A cancelled dialog is not an error toast.
    expect(useUiStore.getState().toasts.filter((t) => t.kind === "error")).toEqual([]);
  });

  it("a failed save aborts too (with an error toast)", async () => {
    const { ctl, client, opened, edit } = await controller();
    await edit();
    vi.spyOn(client, "saveWithDialog").mockRejectedValue({ code: "io", message: "disk full" });
    const run = ctl.openRecent("/x/Other.ods");
    await promptOpen();
    answer("save");
    await run;
    expect(opened).toEqual([]);
    expect(useUiStore.getState().toasts.some((t) => t.kind === "error" && t.message.includes("disk full"))).toBe(true);
  });

  it("Save on an untitled document that is saved via Save As opens the other", async () => {
    const { ctl, opened, edit } = await controller();
    await edit();
    const run = ctl.openRecent("/x/Other.ods");
    await promptOpen();
    answer("save");
    await run;
    expect(opened).toEqual(["/x/Other.ods"]);
  });
});

describe("every replace/close path is guarded", () => {
  const paths: [string, (ctl: AppController) => Promise<unknown>][] = [
    ["New (⌘N, sidebar + New sheet)", (ctl) => ctl.newWorkbook()],
    ["template (sidebar, File ▸ New from Template)", (ctl) => ctl.newFromTemplate("invoices")],
    ["Open… (⌘O)", (ctl) => ctl.openDialog()],
    ["Close (⌘W)", (ctl) => ctl.closeDocument()],
    ["user template", (ctl) => ctl.w56.newFromUserTemplate("user:Budget")],
    ["recovery", (ctl) => ctl.w56.recover({ path: "/autosave/doc-1-Book.ods", title: "Book.ods", savedAt: "now" })],
    ["registry file.new / file.open / file.close", (ctl) => Promise.resolve(ctl.registry.run("file.new"))],
  ];

  for (const [name, run] of paths) {
    it(`${name}: Cancel keeps the document`, async () => {
      const { ctl, client, edit, a1 } = await controller();
      if (name.startsWith("registry")) {
        const { buildCommands } = await import("../src/commands");
        ctl.registry.registerAll(buildCommands(ctl, { renameActiveSheet() {}, showAbout() {}, customNumberFormat() {}, resize() {} }));
      }
      const newWb = vi.spyOn(client, "newWorkbook");
      const openDialog = vi.spyOn(client, "openDialog");
      await edit("mine");
      const doc = ctl.info!.doc;
      const p = run(ctl);
      await promptOpen();
      answer("cancel");
      await p;
      await tick();
      expect(ctl.info?.doc).toBe(doc);
      expect(await a1()).toBe("mine");
      expect(newWb).not.toHaveBeenCalled();
      expect(openDialog).not.toHaveBeenCalled();
    });
  }

  it("Don't Save on New replaces the document", async () => {
    const { ctl, edit } = await controller();
    await edit();
    const doc = ctl.info!.doc;
    const p = ctl.newWorkbook();
    await promptOpen();
    answer("discard");
    await p;
    expect(ctl.info?.doc).not.toBe(doc);
    expect(ctl.info?.dirty).toBe(false);
  });

  it("a second request while the prompt is open does not stack prompts", async () => {
    const { ctl, opened, edit } = await controller();
    await edit();
    const first = ctl.openRecent("/x/A.ods");
    await promptOpen();
    await ctl.openRecent("/x/B.ods"); // resolves at once: the guard is busy
    answer("discard");
    await first;
    expect(opened).toEqual(["/x/A.ods"]);
  });
});

describe("window close and quit (desktop shell)", () => {
  it("mirrors the dirty state to set_document_edited when it changes", async () => {
    const { ctl, edit, edited, editedDocs } = await controller(true);
    // Before the first document, then for it.
    expect(edited).toEqual([false, false]);
    expect(editedDocs).toEqual([null, ctl.info!.doc]);
    await edit();
    await edit("again");
    expect(edited).toEqual([false, false, true]);
    await ctl.save(false);
    expect(edited).toEqual([false, false, true, false]);
    await ctl.apply("Typing", [{ type: "setCellInput", sheet: ctl.sheet, r: 1, c: 0, input: "x" }]);
    await ctl.undo();
    expect(edited.at(-1)).toBe(true);
    const doc = ctl.info!.doc;
    const run = ctl.newWorkbook();
    await promptOpen();
    answer("discard");
    await run;
    expect(edited.at(-1)).toBe(false);
    // The new document is reported by its own id.
    expect(ctl.info!.doc).not.toBe(doc);
    expect(editedDocs.at(-1)).toBe(ctl.info!.doc);
  });

  it("close-requested: Cancel keeps the window; Don't Save confirms (the shell closes the doc)", async () => {
    const { ctl, client, edit, confirms, cancels, fireClose } = await controller(true);
    await edit();
    fireClose("window");
    await promptOpen();
    answer("cancel");
    await tick();
    expect(confirms).toEqual([]);
    // Cancelling a window close is not a cancelled quit.
    expect(cancels()).toBe(0);
    const close = vi.spyOn(client, "close");
    const run = ctl.closeRequested("quit");
    await promptOpen();
    answer("discard");
    expect(await run).toBe(true);
    // The document stays open until the quit really ends (another window may still cancel it).
    expect(close).not.toHaveBeenCalled();
    expect(confirms).toEqual([true]);
  });

  it("Cancel during a quit tells the shell, which aborts the whole quit", async () => {
    const { ctl, client, edit, confirms, cancels } = await controller(true);
    await edit();
    let run = ctl.closeRequested("quit");
    await promptOpen();
    answer("cancel");
    expect(await run).toBe(false);
    expect(cancels()).toBe(1);
    expect(confirms).toEqual([]);
    // A cancelled Save As counts as Cancel.
    vi.spyOn(client, "saveWithDialog").mockRejectedValueOnce({ code: CANCELLED, message: "" });
    run = ctl.closeRequested("quit");
    await promptOpen();
    answer("save");
    expect(await run).toBe(false);
    expect(cancels()).toBe(2);
  });

  it("⌘Q while this window's own close prompt is up: Cancel there also cancels the quit", async () => {
    const { ctl, edit, confirms, cancels, fireClose } = await controller(true);
    await edit();
    const windowClose = ctl.closeRequested("window");
    await promptOpen();
    // The quit asks this window while its window prompt is up: ignored for now…
    fireClose("quit");
    await tick();
    expect(cancels()).toBe(0);
    answer("cancel");
    expect(await windowClose).toBe(false);
    // …but the Cancel ends the quit too (no stale quit left in the shell).
    expect(cancels()).toBe(1);
    expect(confirms).toEqual([]);
  });

  it("the quit reaching a window whose close prompt is up: Don't Save closes it and the quit goes on", async () => {
    const { ctl, edit, confirms, cancels, fireClose } = await controller(true);
    await edit();
    const windowClose = ctl.closeRequested("window");
    await promptOpen();
    fireClose("quit");
    await tick();
    answer("discard");
    expect(await windowClose).toBe(true);
    // The window closes (the shell continues the quit when it is destroyed); nothing is cancelled.
    expect(confirms).toEqual([false]);
    expect(cancels()).toBe(0);
    // A later plain window close does not report a quit cancel.
    await edit();
    const again = ctl.closeRequested("window");
    await promptOpen();
    answer("cancel");
    await again;
    expect(cancels()).toBe(0);
  });

  it("after answering a quit, the next change is reported again (re-arms the shell's guard)", async () => {
    const { ctl, edit, edited, fireClose } = await controller(true);
    await edit();
    fireClose("quit");
    await promptOpen();
    answer("discard");
    for (let i = 0; i < 5; i++) await tick();
    const before = edited.length;
    await edit("more");
    expect(edited.length).toBe(before + 1);
    expect(edited.at(-1)).toBe(true);
  });

  it("a second close-requested while the prompt is up is ignored (not a cancel)", async () => {
    const { ctl, edit, confirms, cancels } = await controller(true);
    await edit();
    const first = ctl.closeRequested("quit");
    await promptOpen();
    expect(await ctl.closeRequested("quit")).toBe(false);
    expect(cancels()).toBe(0);
    expect(useSavePromptStore.getState().prompt).not.toBeNull();
    answer("discard");
    expect(await first).toBe(true);
    expect(confirms).toEqual([true]);
  });

  it("close-requested: Save saves first, then closes the window; a cancelled Save As aborts", async () => {
    const { client, ctl, edit, confirms } = await controller(true);
    await edit();
    const save = vi.spyOn(client, "saveWithDialog").mockRejectedValueOnce({ code: CANCELLED, message: "" });
    let run = ctl.closeRequested("window");
    await promptOpen();
    answer("save");
    expect(await run).toBe(false);
    expect(confirms).toEqual([]);
    run = ctl.closeRequested("window");
    await promptOpen();
    answer("save");
    expect(await run).toBe(true);
    expect(save).toHaveBeenCalledTimes(2);
    expect(confirms).toEqual([false]);
  });

  it("a clean document closes without a prompt", async () => {
    const { ctl, confirms } = await controller(true);
    expect(await ctl.closeRequested("window")).toBe(true);
    expect(useSavePromptStore.getState().prompt).toBeNull();
    expect(confirms).toEqual([false]);
  });
});

describe("TauriClient close commands", () => {
  it("invoke closing.rs commands and listen to close-requested", async () => {
    const calls: { cmd: string; args: InvokeArgs | undefined }[] = [];
    const handlers = new Map<string, (e: TauriEvent<unknown>) => void>();
    const client = new TauriClient({
      invoke: <T,>(cmd: string, args?: InvokeArgs): Promise<T> => {
        calls.push({ cmd, args });
        return Promise.resolve(null as T);
      },
      listen: <T,>(e: string, h: (e: TauriEvent<T>) => void) => {
        handlers.set(e, h as (e: TauriEvent<unknown>) => void);
        return Promise.resolve(() => undefined);
      },
    });
    await client.setDocumentEdited(true, 3, "Book.ods");
    await client.confirmClose(false);
    await client.cancelQuit();
    expect(calls).toEqual([
      { cmd: "set_document_edited", args: { edited: true, doc: 3, title: "Book.ods" } },
      { cmd: "confirm_close", args: { quit: false } },
      { cmd: "cancel_quit", args: undefined },
    ]);
    const kinds: string[] = [];
    client.onCloseRequested((k) => kinds.push(k));
    await tick();
    handlers.get(EVENTS.closeRequested)?.({ payload: { kind: "quit" } });
    expect(kinds).toEqual(["quit"]);
  });

  it("are registered in lib.rs and defined in closing.rs", () => {
    const src = new URL("../../../apps/desktop/src-tauri/src/", import.meta.url);
    const lib = readFileSync(fileURLToPath(new URL("lib.rs", src)), "utf8");
    const closing = readFileSync(fileURLToPath(new URL("closing.rs", src)), "utf8");
    for (const m of CLOSE_SHELL_COMMANDS) {
      const name = COMMANDS[m];
      expect(lib, name).toContain(`closing::${name},`);
      expect(closing, name).toMatch(new RegExp(`#\\[tauri::command\\]\\s*pub (?:async )?fn ${name}\\(`));
    }
    expect(closing).toContain(`"${EVENTS.closeRequested}"`);
  });
});

describe("uncommitted typing (in-cell editor, formula bar)", () => {
  it("isPendingEdit: an open session whose text differs from the cell's input", () => {
    expect(isPendingEdit(null, "a")).toBe(false);
    expect(isPendingEdit({ text: "a" }, "a")).toBe(false);
    expect(isPendingEdit({ text: "" }, null)).toBe(false);
    expect(isPendingEdit({ text: "b" }, "a")).toBe(true);
    expect(isPendingEdit({ text: "x" }, null)).toBe(true);
  });

  const paths: [string, (ctl: AppController) => Promise<unknown>][] = [
    ["New", (ctl) => ctl.newWorkbook()],
    ["Open…", (ctl) => ctl.openDialog()],
    ["Open recent", (ctl) => ctl.openRecent("/x/Other.ods")],
    ["template", (ctl) => ctl.newFromTemplate("invoices")],
    ["Close", (ctl) => ctl.closeDocument()],
    ["window close", (ctl) => ctl.closeRequested("window")],
    ["quit", (ctl) => ctl.closeRequested("quit")],
  ];

  for (const [name, run] of paths) {
    it(`${name}: commits the typing, then asks; Cancel keeps it`, async () => {
      const { ctl, client, opened, type, a1, confirms } = await controller(true);
      const newWb = vi.spyOn(client, "newWorkbook");
      const openDialog = vi.spyOn(client, "openDialog");
      expect(ctl.info?.dirty).toBe(false);
      type("typed");
      const doc = ctl.info!.doc;
      const p = run(ctl);
      await promptOpen();
      expect(useEditStore.getState().edit).toBeNull();
      expect(ctl.info?.dirty).toBe(true);
      expect(await a1()).toBe("typed");
      answer("cancel");
      expect(await p).not.toBe(true);
      expect(ctl.info?.doc).toBe(doc);
      expect(await a1()).toBe("typed");
      expect(opened).toEqual([]);
      expect(newWb).not.toHaveBeenCalled();
      expect(openDialog).not.toHaveBeenCalled();
      expect(confirms).toEqual([]);
    });
  }

  it("the formula bar and {=…} array edits take the same path", async () => {
    const { ctl, client, type, a1 } = await controller();
    type("=1+2", "bar");
    let p = ctl.newWorkbook();
    await promptOpen();
    answer("cancel");
    await p;
    expect(await a1()).toBe("3");

    // Re-entering an array block: the whole block is re-entered.
    const apply = vi.spyOn(client, "apply");
    useEditStore.getState().setActive({ sheet: ctl.sheet, cell: { r: 0, c: 0 }, input: "{=1}", hint: null, arrayRange: { r0: 0, c0: 0, r1: 1, c1: 1 } });
    type("{=7}");
    p = ctl.newWorkbook();
    await promptOpen();
    answer("cancel");
    await p;
    expect(apply.mock.calls[0]?.[1].ops).toEqual([{ type: "setArrayFormula", sheet: ctl.sheet, range: { r0: 0, c0: 0, r1: 1, c1: 1 }, input: "=7" }]);
  });

  it("an unchanged edit session just ends: no prompt, still clean", async () => {
    const { ctl, client, opened } = await controller();
    const apply = vi.spyOn(client, "apply");
    ctl.startEdit({ r: 0, c: 0 }); // F2 on the (empty) cell
    expect(useEditStore.getState().edit).not.toBeNull();
    expect(ctl.pendingEdit).toBe(false);
    await ctl.openRecent("/x/Other.ods");
    expect(useSavePromptStore.getState().prompt).toBeNull();
    expect(apply).not.toHaveBeenCalled();
    expect(opened).toEqual(["/x/Other.ods"]);
  });

  it("a commit refused by validation (Stop) aborts without a prompt", async () => {
    const { ctl, opened } = await controller();
    await ctl.v3.applyValidation(
      {
        id: 0,
        ranges: [{ r0: 0, c0: 0, r1: 0, c1: 0 }],
        kind: { type: "list", source: { type: "values", items: ["Paid", "Due"] } },
        allowBlank: true,
        showDropdown: true,
        inputHelp: null,
        error: { style: "stop", title: "Status", text: "Choose Paid or Due." },
      },
      false,
    );
    await ctl.save(false); // clean again
    expect(ctl.info?.dirty).toBe(false);
    const doc = ctl.info!.doc;
    ctl.startEdit({ r: 0, c: 0 }, "Maybe", "grid");
    await ctl.openRecent("/x/Other.ods");
    expect(opened).toEqual([]);
    expect(ctl.info?.doc).toBe(doc);
    expect(useSavePromptStore.getState().prompt).toBeNull();
    await tick();
    await tick();
    // The Stop alert is up (its Retry re-opens the editor with the text).
    expect(useUiStore.getState().dialog).toMatchObject({ kind: "validationAlert", retry: "Maybe" });
    expect(ctl.info?.dirty).toBe(false);
  });

  it("a failed commit aborts too", async () => {
    const { ctl, client, opened, type } = await controller();
    vi.spyOn(client, "apply").mockRejectedValueOnce({ code: "io", message: "engine gone" });
    type("x");
    await ctl.newFromTemplate("invoices");
    expect(opened).toEqual([]);
    expect(useSavePromptStore.getState().prompt).toBeNull();
  });

  it("set_document_edited counts uncommitted typing (true while typing, false after cancel)", async () => {
    const { ctl, edited, editedDocs } = await controller(true);
    expect(edited).toEqual([false, false]);
    ctl.startEdit({ r: 0, c: 0 }, "a", "grid");
    ctl.onEditChange("ab", 2);
    ctl.onEditChange("abc", 3);
    // Once, not per keystroke.
    expect(edited).toEqual([false, false, true]);
    expect(editedDocs.at(-1)).toBe(ctl.info!.doc);
    ctl.cancelEdit();
    expect(edited).toEqual([false, false, true, false]);
    // Typing back to the original text is not an edit.
    ctl.startEdit({ r: 0, c: 0 }, "a", "grid");
    ctl.onEditChange("", 0);
    expect(edited.at(-1)).toBe(false);
    ctl.cancelEdit();
    // Enter: stays edited through the commit (no blink) as the document turns dirty.
    const before = edited.length;
    ctl.startEdit({ r: 0, c: 0 }, "z", "grid");
    await ctl.commitEdit("down");
    expect(ctl.info?.dirty).toBe(true);
    expect(edited.slice(before)).toEqual([true]);
  });

  it("a refused trailing autoFitRows (protected sheet) still counts as committed", async () => {
    const { ctl, client, type, a1 } = await controller();
    const real = client.apply.bind(client);
    vi.spyOn(client, "apply").mockImplementationOnce(async (doc, tx) => {
      expect(tx.ops.map((o) => o.type)).toEqual(["setCellInput", "autoFitRows"]);
      const res = await real(doc, { ...tx, ops: tx.ops.slice(0, 1) });
      return { ...res, errors: [{ opIndex: 1, code: "protected", message: "The sheet is protected." }] };
    });
    type("Two\nlines");
    const p = ctl.newWorkbook();
    await promptOpen();
    answer("cancel");
    await p;
    expect(await a1()).toBe("Two\nlines");
  });

  it("a throwing commit step does not leave the edited flag stuck", async () => {
    const { ctl, edited } = await controller(true);
    vi.spyOn(ctl, "typingOps").mockImplementationOnce(() => {
      throw new Error("boom");
    });
    ctl.startEdit({ r: 0, c: 0 }, "x", "grid");
    expect(edited.at(-1)).toBe(true);
    await expect(ctl.commitEdit("down")).rejects.toThrow("boom");
    expect(edited.at(-1)).toBe(false);
  });

  it("a late cellEditInfo reply for an earlier session does not mark new typing unchanged", async () => {
    const { ctl, client, a1 } = await controller();
    await ctl.apply("Typing", [{ type: "setCellInput", sheet: ctl.sheet, r: 0, c: 0, input: "same" }]);
    await ctl.save(false); // clean
    const real = client.cellEditInfo.bind(client);
    let release: () => void = () => undefined;
    vi.spyOn(client, "cellEditInfo").mockImplementationOnce(
      (...args) =>
        new Promise((resolve) => {
          release = () => resolve(real(...args));
        }),
    );
    // F2 on A1 with its input not known yet (the reply is held back), then Esc.
    useEditStore.getState().setActive(null);
    ctl.startEdit({ r: 0, c: 0 });
    ctl.cancelEdit();
    // Typing A1's content into B1; then A1's reply arrives.
    ctl.startEdit({ r: 0, c: 1 }, "same", "grid");
    release();
    await tick();
    await tick();
    expect(ctl.pendingEdit).toBe(true);
    const p = ctl.newWorkbook();
    await promptOpen();
    answer("cancel");
    await p;
    const [b] = await client.cells(ctl.info!.doc, ctl.sheet, [{ r0: 0, c0: 1, r1: 0, c1: 1 }]);
    expect(b?.cells[0]?.text).toBe("same");
    expect(await a1()).toBe("same");
  });

  it("Enter then an immediate close: the guard waits for the commit and asks", async () => {
    const { ctl, client, confirms } = await controller(true);
    const real = client.apply.bind(client);
    vi.spyOn(client, "apply").mockImplementationOnce(async (doc, tx) => {
      await new Promise((r) => setTimeout(r, 20));
      return real(doc, tx);
    });
    ctl.startEdit({ r: 0, c: 0 }, "late", "grid");
    const enter = ctl.commitEdit("down");
    expect(ctl.info?.dirty).toBe(false); // the apply is still in flight
    const run = ctl.closeRequested("window");
    await promptOpen();
    expect(ctl.info?.dirty).toBe(true);
    answer("cancel");
    expect(await run).toBe(false);
    expect(await enter).toBe(true);
    expect(confirms).toEqual([]);
  });

  it("quit with typing: the typing is committed, then Don't Save confirms", async () => {
    const { ctl, type, a1, confirms, fireClose } = await controller(true);
    type("typed");
    fireClose("quit");
    await promptOpen();
    expect(await a1()).toBe("typed");
    answer("discard");
    for (let i = 0; i < 5; i++) await tick();
    expect(ctl.info?.dirty).toBe(true);
    expect(confirms).toEqual([true]);
  });
});

describe("close-requested { kind: \"tabs\" } (a window with several tabs, Close Other Tabs)", () => {
  it("is a sequence like a quit: Don't Save answers, Cancel aborts", async () => {
    expect(isSequence("tabs")).toBe(true);
    expect(isSequence("quit")).toBe(true);
    expect(isSequence("window")).toBe(false);
    const { ctl, client, edit, confirms, cancels } = await controller(true);
    await edit();
    const close = vi.spyOn(client, "close");
    let run = ctl.closeRequested("tabs");
    await promptOpen();
    answer("discard");
    expect(await run).toBe(true);
    // Nothing is closed here: the shell closes the tabs once every one answered.
    expect(close).not.toHaveBeenCalled();
    expect(confirms).toEqual([true]);
    await edit("again");
    run = ctl.closeRequested("tabs");
    await promptOpen();
    answer("cancel");
    expect(await run).toBe(false);
    expect(cancels()).toBe(1);
  });
});
