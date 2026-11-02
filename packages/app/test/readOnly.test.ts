// @vitest-environment happy-dom
/**
 * Read-only mode (readOnly.ts): the command gate, the banner, the lock
 * prompt's wording and answers, Edit Mode through the controller over the
 * mock engine, and the `TauriClient` side of `wb_open(path, mode)` /
 * `wb_set_read_only`.
 */
import { COMMANDS, READ_ONLY_SHELL_COMMANDS, TauriClient, type InvokeArgs } from "@tally/bridge/tauri";
import type { LockInfo } from "@tally/bridge";
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { ReadOnlyBanner } from "../src/components/ReadOnlyBanner";
import { LockPrompt } from "../src/components/LockPrompt";
import { ControllerContext } from "../src/context";
import { AppController } from "../src/controller";
import { buildCommands } from "../src/commands";
import { WebMockClient } from "../src/dev/webMock";
import { CommandRegistry } from "../src/registry";
import {
  allowedWhileReadOnly,
  displayTitle,
  lockedMessage,
  lockOwner,
  lockPromptButtons,
  lockPromptParts,
  lockPromptText,
  parseLock,
  readOnlyGate,
  useLockPromptStore,
} from "../src/readOnly";
import { useDocStore } from "../src/stores/docStore";
import { useUiStore } from "../src/stores/uiStore";
import { useSavePromptStore } from "../src/unsaved";

const JANE: LockInfo = { file: "Budget.ods", path: "/x/Budget.ods", owner: "Jane Doe", time: "02.11.2026 15:04", own: false };

const tick = () => new Promise((r) => setTimeout(r, 0));

describe("the read-only command gate", () => {
  it("allows viewing, copying, finding, printing, Save As and Edit Mode only", () => {
    for (const id of ["edit.copy", "edit.find", "file.print", "file.exportPdf", "file.saveAs", "view.zoomIn", "view.zoomReset", "window.nextTab", "edit.editMode", "sheet.next", "help.shortcuts"]) {
      expect(allowedWhileReadOnly(id), id).toBe(true);
    }
    for (const id of ["file.save", "edit.paste", "edit.cut", "edit.undo", "edit.delete", "edit.findReplace", "format.bold", "insert.chart", "sheet.insert", "data.sort", "view.freeze", "view.showFormulas", "file.properties", "some.future.command"]) {
      expect(allowedWhileReadOnly(id), id).toBe(false);
    }
  });

  it("disables commands with AOO's reason while active, and stacks with other gates", () => {
    const reg = new CommandRegistry();
    let ro = true;
    let picking = false;
    for (const id of ["file.save", "edit.copy", "format.bold", "window.nextTab"]) reg.register({ id, title: id, kind: "x", run: () => undefined });
    reg.addGate(readOnlyGate(() => ro));
    const offPick = reg.addGate({ active: () => picking, allow: (id) => id.startsWith("window."), reason: () => "Finish editing the cell first" });
    expect(reg.isEnabled("file.save")).toBe(false);
    expect(reg.run("format.bold")).toBe(false);
    expect(reg.reasonFor("file.save")).toBe("Document opened in read-only mode.");
    expect(reg.isEnabled("edit.copy")).toBe(true);
    picking = true;
    expect(reg.isEnabled("edit.copy")).toBe(false);
    expect(reg.reasonFor("edit.copy")).toBe("Finish editing the cell first");
    offPick();
    ro = false;
    expect(reg.isEnabled("file.save")).toBe(true);
    expect(reg.isEnabled("edit.copy")).toBe(true);
  });

  it("appends AOO's (read-only) to the title", () => {
    expect(displayTitle({ title: "Book.ods", readOnly: true })).toBe("Book.ods (read-only)");
    expect(displayTitle({ title: "Book.ods", readOnly: false })).toBe("Book.ods");
    expect(displayTitle({ title: "Book.ods" })).toBe("Book.ods");
  });
});

describe("the lock prompt", () => {
  it("parses the shell's locked error and words it as AOO does", () => {
    const lock = parseLock({ code: "locked", message: JSON.stringify(JANE) });
    expect(lock).toEqual(JANE);
    expect(parseLock({ code: "io", message: "{}" })).toBeNull();
    expect(parseLock({ code: "locked", message: "not json" })).toBeNull();
    expect(lockOwner(JANE)).toBe("Jane Doe ( 02.11.2026 15:04 )");
    expect(lockOwner({ ...JANE, owner: "", time: "" })).toBe("Unknown User");
    const open = lockPromptText("open", JANE);
    expect(open.title).toBe("Document in Use");
    expect(open.message).toBe("Document file 'Budget.ods' is locked for editing by:\n\nJane Doe ( 02.11.2026 15:04 )\n\nOpen document read-only or open a copy of the document for editing.");
    expect(lockPromptText("openOwn", { ...JANE, own: true }).message).toContain("locked for editing by yourself on a different system since 02.11.2026 15:04");
    expect(lockPromptText("notWritable", null).message).toBe("This document cannot be edited, possibly due to missing access rights. Do you want to edit a copy of the document?");
    expect(lockPromptButtons("open")).toEqual(["readOnly", "copy", "cancel"]);
    expect(lockPromptButtons("openOwn")).toEqual(["readOnly", "open", "cancel"]);
    expect(lockPromptButtons("edit")).toEqual(["copy", "cancel"]);
  });

  it("renders as an alert dialog and answers", async () => {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root = createRoot(host);
    act(() => root.render(createElement(LockPrompt)));
    let answer: Promise<string> = Promise.resolve("");
    act(() => {
      answer = useLockPromptStore.getState().ask("open", JANE);
    });
    const dlg = host.querySelector('[data-testid="lock-prompt"]');
    expect(dlg?.getAttribute("role")).toBe("alertdialog");
    expect(dlg?.textContent).toContain("Jane Doe ( 02.11.2026 15:04 )");
    const labels = [...host.querySelectorAll("button")].map((b) => b.textContent);
    expect(labels).toEqual(["Open Read-Only", "Open Copy", "Cancel"]);
    act(() => host.querySelector<HTMLButtonElement>('[data-testid="lock-prompt-copy"]')!.click());
    expect(await answer).toBe("copy");
    expect(host.querySelector('[data-testid="lock-prompt"]')).toBeNull();
    act(() => root.unmount());
    host.remove();
  });
});

describe("text from lock files is isolated (bidi)", () => {
  it("renders the file and the owner in <bdi> elements", () => {
    const evil: LockInfo = { ...JANE, owner: "Eve", file: "abc.ods" };
    const { parts } = lockPromptParts("open", evil);
    expect(parts.filter((p) => typeof p !== "string")).toEqual([{ bdi: "abc.ods" }, { bdi: "Eve ( 02.11.2026 15:04 )" }]);
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root = createRoot(host);
    act(() => root.render(createElement(LockPrompt)));
    act(() => void useLockPromptStore.getState().ask("open", evil));
    const bdis = [...host.querySelectorAll("bdi")].map((b) => b.textContent);
    expect(bdis).toEqual(["abc.ods", "Eve ( 02.11.2026 15:04 )"]);
    act(() => useLockPromptStore.getState().answer("cancel"));
    act(() => root.unmount());
    host.remove();
  });

  it("a locked Save As target shows AOO's wording, not JSON (isolated)", async () => {
    expect(lockedMessage(JANE)).toBe("Document file '\u2068Budget.ods\u2069' is locked for editing by \u2068Jane Doe ( 02.11.2026 15:04 )\u2069.");
    const client = new WebMockClient();
    const ctl = new AppController(client, client, { nativeMenu: false });
    useUiStore.setState({ toasts: [] });
    ctl.notifyError({ code: "locked", message: JSON.stringify(JANE) });
    const toast = useUiStore.getState().toasts.at(-1);
    expect(toast?.message).toBe(lockedMessage(JANE));
    expect(toast?.message).not.toContain("{");
  });
});

describe("read-only documents in the app (mock engine)", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    useDocStore.getState().reset();
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
  });

  afterEach(() => {
    act(() => root.unmount());
    host.remove();
  });

  async function start(): Promise<{ client: WebMockClient; ctl: AppController }> {
    const client = new WebMockClient();
    client.readOnlyHooks.openReadOnly = true;
    const ctl = new AppController(client, client, { nativeMenu: false });
    ctl.registry.registerAll(buildCommands(ctl, { renameActiveSheet() {}, showAbout() {}, customNumberFormat() {}, resize() {} }));
    await ctl.start(null);
    return { client, ctl };
  }

  /** Edit Mode on, then off again: now read-only without holding the lock (as after a switch in AOO). */
  async function releaseLock(ctl: AppController): Promise<void> {
    await ctl.readOnly.toggleEditMode();
    await ctl.readOnly.toggleEditMode();
    expect(useDocStore.getState().info?.readOnly).toBe(true);
  }

  it("shows the banner (role=status) while read-only; Edit Document switches to editing", async () => {
    const { ctl } = await start();
    act(() => root.render(createElement(ControllerContext.Provider, { value: ctl }, createElement(ReadOnlyBanner))));
    const banner = host.querySelector('[data-testid="read-only-banner"]');
    expect(banner?.getAttribute("role")).toBe("status");
    expect(banner?.textContent).toContain("This document is open in read-only mode.");
    expect(ctl.registry.isEnabled("file.save")).toBe(false);
    expect(ctl.registry.isEnabled("file.saveAs")).toBe(true);
    await act(async () => {
      host.querySelector<HTMLButtonElement>('[data-testid="read-only-edit"]')!.click();
      await tick();
    });
    expect(useDocStore.getState().info?.readOnly).toBe(false);
    expect(host.querySelector('[data-testid="read-only-banner"]')).toBeNull();
    expect(ctl.registry.isEnabled("file.save")).toBe(true);
  });

  it("refuses typing and engine edits while read-only", async () => {
    const { client, ctl } = await start();
    const info = useDocStore.getState().info!;
    ctl.startEdit({ r: 0, c: 0 }, "x");
    expect(ctl.registry.isEnabled("edit.paste")).toBe(false);
    expect(useUiStore.getState().toasts.some((t) => t.message === "Document opened in read-only mode.")).toBe(true);
    await expect(client.apply(info.doc, { ops: [], label: null, selectionBefore: null, selectionAfter: null })).rejects.toMatchObject({ code: "read_only" });
  });

  it("to read-only asks to save first; Don't Save discards", async () => {
    const { client, ctl } = await start();
    await ctl.readOnly.toggleEditMode();
    const info = useDocStore.getState().info!;
    expect(info.readOnly).toBe(false);
    await client.apply(info.doc, { ops: [{ type: "setCellInput", sheet: info.activeSheet, r: 0, c: 0, input: "1" }], label: null, selectionBefore: null, selectionAfter: null });
    useDocStore.getState().setInfo({ ...info, dirty: true });
    const pending = ctl.readOnly.toggleEditMode();
    await tick();
    expect(useSavePromptStore.getState().prompt).not.toBeNull();
    useSavePromptStore.getState().answer("discard");
    await pending;
    expect(useDocStore.getState().info?.readOnly).toBe(true);
    expect(useDocStore.getState().info?.dirty).toBe(false);
  });

  it("to edit: someone else's lock offers Open Copy (an untitled editable copy)", async () => {
    const { client, ctl } = await start();
    await releaseLock(ctl);
    client.readOnlyHooks.lockedBy = JANE;
    const pending = ctl.readOnly.toggleEditMode();
    await tick();
    const prompt = useLockPromptStore.getState().prompt;
    expect(prompt?.kind).toBe("edit");
    expect(prompt?.lock?.owner).toBe("Jane Doe");
    useLockPromptStore.getState().answer("copy");
    await pending;
    const info = useDocStore.getState().info!;
    expect(info.readOnly).toBe(false);
    expect(info.path).toBeNull();
  });

  it("to edit: an unwritable file asks to edit a copy; No keeps it read-only", async () => {
    const { client, ctl } = await start();
    await releaseLock(ctl);
    client.readOnlyHooks.unwritable = true;
    const pending = ctl.readOnly.toggleEditMode();
    await tick();
    expect(useLockPromptStore.getState().prompt?.kind).toBe("notWritable");
    useLockPromptStore.getState().answer("cancel");
    await pending;
    expect(useDocStore.getState().info?.readOnly).toBe(true);
  });

  it("opening a locked file asks, then opens with the answer", async () => {
    const { ctl } = await start();
    const opened: [string, string][] = [];
    const shell = ctl.shell as unknown as { openWith: (p: string, m: string) => Promise<unknown> };
    shell.openWith = (p, m) => {
      opened.push([p, m]);
      return Promise.resolve({ ...useDocStore.getState().info!, readOnly: m === "readOnly" });
    };
    const res = ctl.readOnly.resolveLockedOpen({ code: "locked", message: JSON.stringify(JANE) });
    await tick();
    expect(useLockPromptStore.getState().prompt?.kind).toBe("open");
    useLockPromptStore.getState().answer("readOnly");
    expect((await res)?.readOnly).toBe(true);
    expect(opened).toEqual([["/x/Budget.ods", "readOnly"]]);
    const cancelled = ctl.readOnly.resolveLockedOpen({ code: "locked", message: JSON.stringify({ ...JANE, own: true }) });
    await tick();
    expect(useLockPromptStore.getState().prompt?.kind).toBe("openOwn");
    useLockPromptStore.getState().answer("cancel");
    expect(await cancelled).toBeNull();
    await expect(ctl.readOnly.resolveLockedOpen({ code: "io", message: "x" })).rejects.toMatchObject({ code: "io" });
  });
});

describe("TauriClient read-only commands", () => {
  it("sends wb_open(path, mode), wb_open_dialog(readOnly) and wb_set_read_only", async () => {
    const calls: { cmd: string; args: InvokeArgs | undefined }[] = [];
    const client = new TauriClient({
      invoke: <T>(cmd: string, args?: InvokeArgs) => {
        calls.push({ cmd, args });
        return Promise.resolve(null as T);
      },
      listen: () => Promise.resolve(() => undefined),
    });
    await client.openWith("/a.ods", "readOnly");
    await client.openDialog(true);
    await client.openDialog();
    await client.setReadOnly(3, false, "copy");
    await client.setReadOnly(3, true);
    expect(calls).toEqual([
      { cmd: "wb_open", args: { path: "/a.ods", mode: "readOnly" } },
      { cmd: "wb_open_dialog", args: { readOnly: true } },
      { cmd: "wb_open_dialog", args: undefined },
      { cmd: "wb_set_read_only", args: { doc: 3, readOnly: false, mode: "copy" } },
      { cmd: "wb_set_read_only", args: { doc: 3, readOnly: true, mode: null } },
    ]);
    expect(READ_ONLY_SHELL_COMMANDS.map((m) => COMMANDS[m])).toEqual(["wb_set_read_only"]);
  });
});
