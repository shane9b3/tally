/**
 * The unsaved-changes guard (unsaved.ts, AppController.confirmUnsaved):
 * the pure decision, the prompt store, every replace/close path through the
 * controller with the browser mock, the window/quit hand-off to the shell
 * (`close-requested` → `confirm_close`) and the dirty-state mirror
 * (`set_document_edited`).
 */
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { CANCELLED, type WorkbookInfo } from "@tally/bridge";
import { CLOSE_SHELL_COMMANDS, COMMANDS, EVENTS, TauriClient, type InvokeArgs, type TauriEvent } from "@tally/bridge/tauri";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AppController } from "../src/controller";
import { WebMockClient } from "../src/dev/webMock";
import { useDocStore } from "../src/stores/docStore";
import { useUiStore } from "../src/stores/uiStore";
import { guardDecision, proceedAfter, useSavePromptStore, type CloseKind, type SaveChoice } from "../src/unsaved";

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
  setDocumentEdited: (edited: boolean) => Promise<void>;
  confirmClose: (quit: boolean) => Promise<void>;
  onCloseRequested: (cb: (kind: CloseKind) => void) => () => void;
};

async function controller(withShell = false) {
  const client = new WebMockClient();
  const edited: boolean[] = [];
  const confirms: boolean[] = [];
  let closeCb: ((kind: CloseKind) => void) | null = null;
  if (withShell) {
    Object.assign(client, {
      setDocumentEdited: (e: boolean) => {
        edited.push(e);
        return Promise.resolve();
      },
      confirmClose: (quit: boolean) => {
        confirms.push(quit);
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
  return { client, ctl, detach, opened, edit, a1, edited, confirms, fireClose: (k: CloseKind) => closeCb?.(k) };
}

beforeEach(() => {
  useDocStore.getState().reset();
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
    const { ctl, edit, edited } = await controller(true);
    expect(edited).toEqual([false]);
    await edit();
    await edit("again");
    expect(edited).toEqual([false, true]);
    await ctl.save(false);
    expect(edited).toEqual([false, true, false]);
    await ctl.apply("Typing", [{ type: "setCellInput", sheet: ctl.sheet, r: 1, c: 0, input: "x" }]);
    await ctl.undo();
    expect(edited.at(-1)).toBe(true);
    const run = ctl.newWorkbook();
    await promptOpen();
    answer("discard");
    await run;
    expect(edited.at(-1)).toBe(false);
  });

  it("close-requested: Cancel keeps the window; Don't Save closes the doc and confirms", async () => {
    const { ctl, client, edit, confirms, fireClose } = await controller(true);
    await edit();
    fireClose("window");
    await promptOpen();
    answer("cancel");
    await tick();
    expect(confirms).toEqual([]);
    const close = vi.spyOn(client, "close");
    const doc = ctl.info!.doc;
    const run = ctl.closeRequested("quit");
    await promptOpen();
    answer("discard");
    expect(await run).toBe(true);
    expect(close).toHaveBeenCalledWith(doc);
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
    await client.setDocumentEdited(true);
    await client.confirmClose(false);
    expect(calls).toEqual([
      { cmd: "set_document_edited", args: { edited: true } },
      { cmd: "confirm_close", args: { quit: false } },
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
      expect(closing, name).toMatch(new RegExp(`#\\[tauri::command\\]\\s*pub async fn ${name}\\(`));
    }
    expect(closing).toContain(`"${EVENTS.closeRequested}"`);
  });
});
