/**
 * One document per window (windows.ts, desktop `windows.rs`): the reuse
 * rule, a window starting with the document it was opened for, New / Open /
 * Open Recent / templates / recovery placing their document through the
 * shell, ⌘W closing the window through the guard, prefs and theme changes
 * reaching the other windows, and the `TauriClient` side of the window
 * commands.
 */
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import type { DocId, WorkbookInfo } from "@tally/bridge";
import { COMMANDS, EVENTS, TauriClient, WINDOW_SHELL_COMMANDS, type InvokeArgs, type TauriEvent } from "@tally/bridge/tauri";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AppController } from "../src/controller";
import { WebMockClient } from "../src/dev/webMock";
import { useW56Store } from "../src/data/store";
import { useDocStore } from "../src/stores/docStore";
import { useEditStore } from "../src/stores/editStore";
import { useUiStore } from "../src/stores/uiStore";
import { useSavePromptStore } from "../src/unsaved";
import { canReuseWindow, type ThemeChoice } from "../src/windows";

const tick = () => new Promise((r) => setTimeout(r, 0));

type Placed = { doc: DocId | null; reuse: boolean };

interface Options {
  /** What `window_init` hands over. */
  init?: { doc: WorkbookInfo | null; offerRecovery: boolean };
  /** The shell's placement answer. */
  answer?: (p: Placed) => "here" | "window";
  /** The stored prefs (`get_prefs`). */
  prefs?: string;
}

/** A controller over the browser mock plus a fake desktop window shell. */
async function windowController(opts: Options = {}) {
  const client = new WebMockClient();
  const placed: Placed[] = [];
  const broadcasts: (ThemeChoice | null)[] = [];
  const confirms: boolean[] = [];
  const reports: { edited: boolean; doc: DocId | null; title: string | null | undefined }[] = [];
  let prefsCb: ((e: { theme: ThemeChoice | null }) => void) | null = null;
  const recoveryLists: number[] = [];
  Object.assign(client, {
    windowInit: () => Promise.resolve(opts.init ?? { doc: null, offerRecovery: true }),
    placeDocument: (doc: DocId | null, reuse: boolean) => {
      placed.push({ doc, reuse });
      return Promise.resolve(opts.answer?.({ doc, reuse }) ?? (reuse ? "here" : "window"));
    },
    broadcastPrefs: (theme: ThemeChoice | null) => {
      broadcasts.push(theme);
      return Promise.resolve();
    },
    onPrefsChanged: (cb: (e: { theme: ThemeChoice | null }) => void) => {
      prefsCb = cb;
      return () => {
        prefsCb = null;
      };
    },
    setDocumentEdited: (edited: boolean, doc: DocId | null, title?: string | null) => {
      reports.push({ edited, doc, title });
      return Promise.resolve();
    },
    confirmClose: (quit: boolean) => {
      confirms.push(quit);
      return Promise.resolve();
    },
    cancelQuit: () => Promise.resolve(),
    onCloseRequested: () => () => undefined,
    getPrefs: () => Promise.resolve(opts.prefs ?? "{}"),
    setPrefs: () => Promise.resolve(),
    listRecovery: () => {
      recoveryLists.push(1);
      return Promise.resolve([]);
    },
    recoverSnapshot: () => client.newWorkbook(null),
  });
  const opened: string[] = [];
  client.open = async (path: string): Promise<WorkbookInfo> => {
    opened.push(path);
    const info = await client.newWorkbook("budget");
    return { ...info, title: path.split("/").pop() ?? path, path };
  };
  const newWorkbook = vi.spyOn(client, "newWorkbook");
  const ctl = new AppController(client, client, { nativeMenu: true });
  const detach = ctl.attach();
  await ctl.start(null);
  await tick();
  const edit = async () => {
    await ctl.apply("Typing", [{ type: "setCellInput", sheet: ctl.sheet, r: 0, c: 0, input: "x" }]);
    expect(ctl.info?.dirty).toBe(true);
  };
  return { client, ctl, detach, placed, broadcasts, confirms, reports, opened, edit, newWorkbook, recoveryLists, firePrefs: (theme: ThemeChoice | null) => prefsCb?.({ theme }) };
}

beforeEach(() => {
  useDocStore.getState().reset();
  useEditStore.getState().reset();
  useUiStore.getState().reset();
  useSavePromptStore.getState().answer("cancel");
});

afterEach(() => {
  useSavePromptStore.getState().answer("cancel");
  vi.restoreAllMocks();
});

describe("window reuse (pure)", () => {
  const blank = { doc: 1 as DocId, path: null, dirty: false };
  it("reuses only the untouched blank workbook the window started with", () => {
    expect(canReuseWindow({ info: blank, pristineDoc: 1 as DocId, edited: false })).toBe(true);
    expect(canReuseWindow({ info: null, pristineDoc: null, edited: false })).toBe(false);
    // Edited, being typed into, saved once, or not the start-up workbook (a template, an opened file).
    expect(canReuseWindow({ info: { ...blank, dirty: true }, pristineDoc: 1 as DocId, edited: true })).toBe(false);
    expect(canReuseWindow({ info: blank, pristineDoc: 1 as DocId, edited: true })).toBe(false);
    expect(canReuseWindow({ info: { ...blank, path: "/a.ods" }, pristineDoc: 1 as DocId, edited: false })).toBe(false);
    expect(canReuseWindow({ info: blank, pristineDoc: null, edited: false })).toBe(false);
    expect(canReuseWindow({ info: blank, pristineDoc: 2 as DocId, edited: false })).toBe(false);
  });
});

describe("a window starts with its document", () => {
  it("loads the document handed to it instead of a new blank one, and offers no recovery", async () => {
    // The mock engine numbers documents from 1 in every client, so this id exists in the window's own engine.
    const own = await new WebMockClient().newWorkbook("invoices");
    const { ctl, newWorkbook, recoveryLists } = await windowController({ init: { doc: { ...own, title: "Invoices.ods" }, offerRecovery: false } });
    expect(newWorkbook).not.toHaveBeenCalled();
    expect(ctl.info?.title).toBe("Invoices.ods");
    expect(recoveryLists).toEqual([]);
    // A handed-over document is never replaced by a later Open.
    expect(ctl.canReuseWindow()).toBe(false);
  });

  it("without one it creates a blank workbook, which later opens may reuse; the first window offers recovery", async () => {
    const { ctl, newWorkbook, recoveryLists } = await windowController();
    expect(newWorkbook).toHaveBeenCalledWith(null);
    expect(ctl.canReuseWindow()).toBe(true);
    expect(recoveryLists).toEqual([1]);
  });

  it("reports its title with the edited flag (Window menu)", async () => {
    const { ctl, reports } = await windowController();
    expect(reports.at(-1)).toEqual({ edited: false, doc: ctl.info!.doc, title: ctl.info!.title });
  });
});

describe("New / Open / templates / recovery place their document", () => {
  it("Open Recent from the untouched start-up window shows the file here", async () => {
    const { ctl, placed, opened } = await windowController();
    await ctl.openRecent("/x/A.ods");
    expect(opened).toEqual(["/x/A.ods"]);
    expect(placed).toHaveLength(1);
    expect(placed[0]?.reuse).toBe(true);
    expect(ctl.info?.title).toBe("A.ods");
    // The file is a real document now: the next open gets its own window.
    expect(ctl.canReuseWindow()).toBe(false);
    await ctl.openRecent("/x/B.ods");
    expect(placed[1]?.reuse).toBe(false);
    expect(ctl.info?.title).toBe("A.ods");
  });

  it("an edited window keeps its document and asks nothing: the file opens in a new window", async () => {
    const { ctl, placed, edit } = await windowController();
    await edit();
    const doc = ctl.info!.doc;
    await ctl.openRecent("/x/A.ods");
    expect(useSavePromptStore.getState().prompt).toBeNull();
    expect(placed.map((p) => p.reuse)).toEqual([false]);
    expect(placed[0]?.doc).not.toBe(doc);
    expect(ctl.info?.doc).toBe(doc);
    expect(ctl.info?.dirty).toBe(true);
  });

  it("uncommitted typing also counts as touched", async () => {
    const { ctl, placed } = await windowController();
    ctl.startEdit({ r: 0, c: 0 }, "typing", "grid");
    expect(ctl.canReuseWindow()).toBe(false);
    await ctl.newFromTemplate("budget");
    expect(placed[0]?.reuse).toBe(false);
  });

  it("the shell may still answer `window` (it knows the window is edited)", async () => {
    const { ctl, placed } = await windowController({ answer: () => "window" });
    const doc = ctl.info!.doc;
    await ctl.newFromTemplate("budget");
    expect(placed[0]?.reuse).toBe(true);
    expect(ctl.info?.doc).toBe(doc);
  });

  it("File ▸ New always opens a new blank window", async () => {
    const { ctl, placed, newWorkbook } = await windowController();
    newWorkbook.mockClear();
    const doc = ctl.info!.doc;
    await ctl.newWorkbook();
    expect(placed).toEqual([{ doc: null, reuse: false }]);
    expect(newWorkbook).not.toHaveBeenCalled();
    expect(ctl.info?.doc).toBe(doc);
  });

  it("File ▸ Open… answered `null` (cancelled, or already open elsewhere and now in front) does nothing", async () => {
    const { ctl, client, placed } = await windowController();
    vi.spyOn(client, "openDialog").mockResolvedValueOnce(null);
    const doc = ctl.info!.doc;
    await ctl.openDialog();
    expect(placed).toEqual([]);
    expect(ctl.info?.doc).toBe(doc);
  });

  it("recovery and user templates go through the same placement", async () => {
    const { ctl, placed, edit } = await windowController();
    const other = { path: "/autosave/doc-2-Other.ods", title: "Other.ods", savedAt: "now" };
    useW56Store.getState().openDialog({ kind: "recovery", files: [other] });
    expect(await ctl.w56.recover({ path: "/autosave/doc-1-Book.ods", title: "Book.ods", savedAt: "now" }, [other])).toBe(true);
    expect(placed.map((p) => p.reuse)).toEqual([true]);
    // Taking over the start-up window closed the dialog: the other snapshot is offered again.
    expect(useW56Store.getState().dialog).toEqual({ kind: "recovery", files: [other] });
    useW56Store.getState().openDialog(null);
    await edit();
    await ctl.w56.newFromUserTemplate("user:Budget");
    expect(placed.map((p) => p.reuse)).toEqual([true, false]);
    expect(useSavePromptStore.getState().prompt).toBeNull();
  });

  it("open warnings of a document placed in a new window are shown here (the new window gets the engine's summary)", async () => {
    const { ctl, client } = await windowController({ answer: () => "window" });
    const open = client.open.bind(client);
    client.open = async (path: string) => ({ ...(await open(path)), warnings: ["Recalculation stopped"] });
    await ctl.openRecent("/x/Big.ods");
    expect(useUiStore.getState().toasts.at(-1)?.message).toBe("Big.ods: Recalculation stopped");
  });

  it("a refused placement shows an error and leaves the window alone", async () => {
    const { ctl, client } = await windowController();
    Object.assign(client, { placeDocument: () => Promise.reject({ code: "internal", message: "cannot open a window" }) });
    const doc = ctl.info!.doc;
    await ctl.openRecent("/x/A.ods");
    expect(ctl.info?.doc).toBe(doc);
    expect(useUiStore.getState().toasts.at(-1)?.message).toBe("cannot open a window");
  });
});

describe("⌘W closes the window", () => {
  it("clean: the shell closes the window at once", async () => {
    const { ctl, confirms } = await windowController();
    await ctl.closeDocument();
    expect(confirms).toEqual([false]);
  });

  it("edited: the prompt first; Cancel keeps it, Don't Save closes", async () => {
    const { ctl, confirms, edit } = await windowController();
    await edit();
    let run = ctl.closeDocument();
    await tick();
    expect(useSavePromptStore.getState().prompt).not.toBeNull();
    useSavePromptStore.getState().answer("cancel");
    await run;
    expect(confirms).toEqual([]);
    run = ctl.closeDocument();
    await tick();
    useSavePromptStore.getState().answer("discard");
    await run;
    expect(confirms).toEqual([false]);
  });
});

describe("prefs and theme reach the other windows", () => {
  it("a theme change here is broadcast; one from elsewhere is applied, not echoed", async () => {
    const { broadcasts, firePrefs } = await windowController();
    useUiStore.getState().setThemePref("dark");
    expect(broadcasts).toEqual(["dark"]);
    firePrefs("light");
    expect(useUiStore.getState().themePref).toBe("light");
    expect(broadcasts).toEqual(["dark"]);
  });

  it("saving the prefs broadcasts; prefs-changed re-reads them (UI language, autosave)", async () => {
    const { ctl, broadcasts, firePrefs, client } = await windowController();
    await ctl.w56.savePrefs({ ...useW56Store.getState().prefs, autoInput: false });
    expect(broadcasts).toEqual([null]);
    Object.assign(client, { getPrefs: () => Promise.resolve(JSON.stringify({ ...useW56Store.getState().prefs, autoInput: true, autosaveMinutes: 7 })) });
    firePrefs(null);
    for (let i = 0; i < 5; i++) await tick();
    expect(useW56Store.getState().prefs.autoInput).toBe(true);
    expect(useW56Store.getState().prefs.autosaveMinutes).toBe(7);
    expect(broadcasts).toEqual([null]);
  });
});

describe("TauriClient window commands", () => {
  it("invoke windows.rs / prefs.rs commands and listen to prefs-changed", async () => {
    const calls: { cmd: string; args: InvokeArgs | undefined }[] = [];
    const handlers = new Map<string, (e: TauriEvent<unknown>) => void>();
    const client = new TauriClient({
      invoke: <T,>(cmd: string, args?: InvokeArgs): Promise<T> => {
        calls.push({ cmd, args });
        return Promise.resolve((cmd === "place_document" ? "window" : null) as T);
      },
      listen: <T,>(e: string, h: (e: TauriEvent<T>) => void) => {
        handlers.set(e, h as (e: TauriEvent<unknown>) => void);
        return Promise.resolve(() => undefined);
      },
    });
    await client.windowInit();
    expect(await client.placeDocument(4 as DocId, true)).toBe("window");
    await client.placeDocument(null, false);
    await client.broadcastPrefs("dark");
    expect(calls).toEqual([
      { cmd: "window_init", args: undefined },
      { cmd: "place_document", args: { doc: 4, reuse: true } },
      { cmd: "place_document", args: { doc: null, reuse: false } },
      { cmd: "broadcast_prefs", args: { theme: "dark" } },
    ]);
    const got: unknown[] = [];
    client.onPrefsChanged((e) => got.push(e.theme));
    await tick();
    handlers.get(EVENTS.prefsChanged)?.({ payload: { theme: "light" } });
    expect(got).toEqual(["light"]);
  });

  it("are registered in lib.rs and defined in their modules", () => {
    const src = new URL("../../../apps/desktop/src-tauri/src/", import.meta.url);
    const read = (f: string) => readFileSync(fileURLToPath(new URL(f, src)), "utf8");
    const lib = read("lib.rs");
    const modules: Record<string, string> = { window_init: "windows", place_document: "windows", broadcast_prefs: "prefs" };
    for (const m of WINDOW_SHELL_COMMANDS) {
      const name = COMMANDS[m];
      const module = modules[name] ?? "";
      expect(lib, name).toContain(`${module}::${name},`);
      expect(read(`${module}.rs`), name).toMatch(new RegExp(`#\\[tauri::command\\]\\s*pub (?:async )?fn ${name}\\(`));
    }
    expect(read("prefs.rs")).toContain(`"${EVENTS.prefsChanged}"`);
  });

  it("the desktop listens to its own window's events only", () => {
    const desktop = readFileSync(fileURLToPath(new URL("../../../apps/desktop/src/tauri.ts", import.meta.url)), "utf8");
    expect(desktop).toContain("getCurrentWebviewWindow()");
    expect(desktop).not.toMatch(/from "@tauri-apps\/api\/event"/);
  });
});
