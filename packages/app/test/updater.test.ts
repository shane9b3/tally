/**
 * Auto-update (updater.ts, bridge updates.ts, updater.rs): version order
 * and the skip rule, the pref, the dialog's state machine against a fake
 * shell (automatic prompt, manual check, download → restart, the browser
 * guard, cancel), the Tauri commands (registered, granted, argument
 * names, events) and the browser mock's simulated server.
 */
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { compareVersions, MockClient, shouldPrompt, type RestartAborted, type UpdateInfo, type UpdateProgress, type UpdateShell } from "@tally/bridge";
import { COMMANDS, EVENTS, TauriClient, UPDATE_SHELL_COMMANDS, type InvokeArgs, type TauriEvent } from "@tally/bridge/tauri";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { buildCommands } from "../src/commands";
import type { AppController } from "../src/controller";
import { DEFAULT_PREFS, parsePrefs, serializePrefs } from "../src/options/prefs";
import { acceptsAutomatic, megabytes, progressPercent, UpdateFeature, useUpdateStore, withProgress } from "../src/updater";

const tick = () => new Promise((r) => setTimeout(r, 0));
const SRC = new URL("../../../apps/desktop/src-tauri/", import.meta.url);
const read = (p: string) => readFileSync(fileURLToPath(new URL(p, SRC)), "utf8");

const INFO: UpdateInfo = { version: "0.2.0", currentVersion: "0.1.0", notes: "Fixes", date: "2026-11-20", downloaded: false };

/** A scripted update shell. */
function fakeShell(over: Partial<UpdateShell> = {}) {
  const calls: string[] = [];
  const listeners: { available?: (i: UpdateInfo) => void; progress?: (p: UpdateProgress) => void; aborted?: (e: RestartAborted) => void } = {};
  const shell: UpdateShell = {
    updateStatus: () => Promise.resolve({ currentVersion: "0.1.0", supported: true, reason: null, staged: null }),
    updateCheck: () => (calls.push("check"), Promise.resolve(INFO)),
    updateDownload: async () => {
      calls.push("download");
      listeners.progress?.({ received: 50, total: 100 });
      await tick();
      return { ...INFO, downloaded: true };
    },
    updateRestart: () => (calls.push("restart"), Promise.resolve()),
    updateSkip: (v) => (calls.push(`skip ${v}`), Promise.resolve()),
    onUpdateAvailable: (cb) => ((listeners.available = cb), () => (listeners.available = undefined)),
    onUpdateProgress: (cb) => ((listeners.progress = cb), () => (listeners.progress = undefined)),
    onUpdateRestartAborted: (cb) => ((listeners.aborted = cb), () => (listeners.aborted = undefined)),
    ...over,
  };
  return { shell, calls, listeners };
}

beforeEach(() => useUpdateStore.setState({ phase: null, staged: null }));

describe("versions and the skip rule", () => {
  it("compare as semver", () => {
    expect(compareVersions("0.2.0", "0.1.0")).toBeGreaterThan(0);
    expect(compareVersions("v0.10.0", "0.9.9")).toBeGreaterThan(0);
    expect(compareVersions("1.0.0", "1.0.0")).toBe(0);
    expect(compareVersions("1.0.0-rc.1", "1.0.0")).toBeLessThan(0);
    expect(compareVersions("1.0.0", "1.0.0-rc.1")).toBeGreaterThan(0);
    expect(compareVersions("latest", "1.0.0")).toBeNull();
  });

  it("a skipped version is silent until a newer one; a manual check ignores it", () => {
    expect(shouldPrompt("0.2.0", null, false)).toBe(true);
    expect(shouldPrompt("0.2.0", "0.2.0", false)).toBe(false);
    expect(shouldPrompt("0.1.9", "0.2.0", false)).toBe(false);
    expect(shouldPrompt("0.2.1", "0.2.0", false)).toBe(true);
    expect(shouldPrompt("0.2.0", "0.2.0", true)).toBe(true);
    expect(shouldPrompt("0.2.0", "junk", false)).toBe(true);
  });
});

describe("the pref", () => {
  it("checks automatically by default and keeps an off switch", () => {
    expect(DEFAULT_PREFS.checkForUpdates).toBe(true);
    expect(parsePrefs({}).checkForUpdates).toBe(true);
    expect(parsePrefs({ checkForUpdates: "no" }).checkForUpdates).toBe(true);
    expect(parsePrefs({ checkForUpdates: false }).checkForUpdates).toBe(false);
    // The shell reads the stored key itself (updater.rs AUTO_CHECK_KEY).
    expect(JSON.parse(serializePrefs({ ...DEFAULT_PREFS, checkForUpdates: false }))).toMatchObject({ checkForUpdates: false });
    expect(read("src/updater.rs")).toContain('pub const AUTO_CHECK_KEY: &str = "checkForUpdates";');
  });
});

describe("pure helpers", () => {
  it("progress and sizes", () => {
    expect(progressPercent(50, 200)).toBe(25);
    expect(progressPercent(999, 1000)).toBe(99);
    expect(progressPercent(2000, 1000)).toBe(100);
    expect(progressPercent(10, null)).toBeNull();
    expect(progressPercent(10, 0)).toBeNull();
    expect(megabytes(3 * 1024 * 1024 + 100_000)).toBe("3.1");
    expect(acceptsAutomatic(null)).toBe(true);
    expect(acceptsAutomatic({ kind: "checking" })).toBe(false);
    const dl = { kind: "downloading" as const, info: INFO, received: 0, total: null };
    expect(withProgress(dl, { received: 5, total: 10 })).toEqual({ ...dl, received: 5, total: 10 });
    expect(withProgress({ kind: "upToDate", version: "1" }, { received: 5, total: 10 })).toEqual({ kind: "upToDate", version: "1" });
  });
});

describe("UpdateFeature", () => {
  const desktop = { confirmUnsaved: () => Promise.resolve(true), multiWindow: true };

  it("an automatic update-available opens the prompt, never over another update dialog", () => {
    const { shell, listeners } = fakeShell();
    const f = new UpdateFeature(shell, desktop);
    const off = f.attach();
    listeners.available?.(INFO);
    expect(useUpdateStore.getState().phase).toEqual({ kind: "available", info: INFO, manual: false });
    useUpdateStore.setState({ phase: { kind: "checking" } });
    listeners.available?.({ ...INFO, version: "0.3.0" });
    expect(useUpdateStore.getState().phase).toEqual({ kind: "checking" });
    off();
    expect(listeners.available).toBeUndefined();
  });

  it("Check for Updates: the prompt, up to date, or the error", async () => {
    const { shell } = fakeShell();
    await new UpdateFeature(shell, desktop).checkNow();
    expect(useUpdateStore.getState().phase).toEqual({ kind: "available", info: INFO, manual: true });

    useUpdateStore.setState({ phase: null });
    await new UpdateFeature(fakeShell({ updateCheck: () => Promise.resolve(null) }).shell, desktop).checkNow();
    expect(useUpdateStore.getState().phase).toEqual({ kind: "upToDate", version: "0.1.0" });

    useUpdateStore.setState({ phase: null });
    await new UpdateFeature(fakeShell({ updateCheck: () => Promise.reject({ code: "io", message: "offline" }) }).shell, desktop).checkNow();
    expect(useUpdateStore.getState().phase).toEqual({ kind: "error", what: "check", message: "offline" });

    // Closed while checking: the late answer is dropped.
    useUpdateStore.setState({ phase: null });
    let answer: (v: UpdateInfo | null) => void = () => undefined;
    const slow = new UpdateFeature(fakeShell({ updateCheck: () => new Promise((r) => (answer = r)) }).shell, desktop);
    const pending = slow.checkNow();
    expect(useUpdateStore.getState().phase).toEqual({ kind: "checking" });
    slow.dismiss();
    answer(INFO);
    await pending;
    expect(useUpdateStore.getState().phase).toBeNull();
  });

  it("Install and Restart: downloads with progress (Later does nothing meanwhile), then asks the shell to restart", async () => {
    const { shell, calls } = fakeShell();
    const f = new UpdateFeature(shell, desktop);
    f.attach();
    const done = f.install(INFO);
    await Promise.resolve();
    expect(useUpdateStore.getState().phase).toMatchObject({ kind: "downloading", received: 50, total: 100 });
    f.dismiss();
    expect(useUpdateStore.getState().phase?.kind).toBe("downloading");
    await done;
    expect(calls).toEqual(["download", "restart"]);
    // The dialog closed so the save prompts can be answered.
    expect(useUpdateStore.getState().phase).toBeNull();
    expect(useUpdateStore.getState().staged).toMatchObject({ version: "0.2.0", downloaded: true });
  });

  it("an already downloaded update restarts at once; a failed download says why", async () => {
    const { shell, calls } = fakeShell();
    await new UpdateFeature(shell, desktop).install({ ...INFO, downloaded: true });
    expect(calls).toEqual(["restart"]);
    const bad = fakeShell({ updateDownload: () => Promise.reject({ code: "io", message: "signature mismatch" }) });
    await new UpdateFeature(bad.shell, desktop).install(INFO);
    expect(useUpdateStore.getState().phase).toEqual({ kind: "error", what: "download", message: "signature mismatch" });
    expect(bad.calls).toEqual([]);
  });

  it("Cancel in a save prompt keeps the update downloaded and says so", async () => {
    // Desktop: the shell runs the questions and reports the cancel.
    const { shell, listeners } = fakeShell();
    const f = new UpdateFeature(shell, desktop);
    f.attach();
    await f.install(INFO);
    listeners.aborted?.({ cancelled: true, message: null });
    expect(useUpdateStore.getState().phase).toMatchObject({ kind: "restartCancelled", info: { version: "0.2.0", downloaded: true } });
    listeners.aborted?.({ cancelled: false, message: "disk full" });
    expect(useUpdateStore.getState().phase).toEqual({ kind: "error", what: "install", message: "disk full" });
    // Browser: the guard runs here; Cancel never reaches the shell.
    const web = fakeShell();
    await new UpdateFeature(web.shell, { confirmUnsaved: () => Promise.resolve(false), multiWindow: false }).install(INFO);
    expect(web.calls).toEqual(["download"]);
    expect(useUpdateStore.getState().phase?.kind).toBe("restartCancelled");
  });

  it("Skip This Version remembers it in the shell", async () => {
    const { shell, calls } = fakeShell();
    useUpdateStore.setState({ phase: { kind: "available", info: INFO, manual: false } });
    await new UpdateFeature(shell, desktop).skip("0.2.0");
    expect(calls).toEqual(["skip 0.2.0"]);
    expect(useUpdateStore.getState().phase).toBeNull();
  });

  it("Help ▸ Check for Updates… is a command", () => {
    const ui = { renameActiveSheet() {}, showAbout() {}, customNumberFormat() {}, resize() {} };
    const checkNow = vi.fn();
    const cmds = buildCommands({ updater: { checkNow } } as unknown as AppController, ui);
    cmds.find((c) => c.id === "help.checkUpdates")?.run();
    expect(checkNow).toHaveBeenCalledOnce();
  });
});

describe("Tauri commands", () => {
  it("are registered, defined and granted", () => {
    const lib = read("src/lib.rs");
    const acl = read("app_commands.rs");
    const caps = read("capabilities/default.json");
    const rs = read("src/updater.rs");
    for (const m of UPDATE_SHELL_COMMANDS) {
      const name = COMMANDS[m];
      expect(lib, name).toMatch(new RegExp(`updater::${name},`));
      expect(acl, name).toContain(`"${name}"`);
      expect(caps, name).toContain(`"allow-${name.replace(/_/g, "-")}"`);
      expect(rs, name).toMatch(new RegExp(`#\\[tauri::command\\]\\s*pub async fn ${name}\\(`));
    }
    // The plugin's own JS commands stay unreachable: no updater:* permission anywhere.
    expect((JSON.parse(caps) as { permissions: string[] }).permissions.filter((p) => p.startsWith("updater:"))).toEqual([]);
    for (const [key, event] of [
      ["UPDATE_AVAILABLE", EVENTS.updateAvailable],
      ["UPDATE_PROGRESS", EVENTS.updateProgress],
      ["UPDATE_RESTART_ABORTED", EVENTS.updateRestartAborted],
    ] as const) {
      expect(rs).toContain(`pub const ${key}: &str = "${event}";`);
    }
  });

  it("invoke with the documented argument names and listen to the events", async () => {
    const calls: { cmd: string; args: InvokeArgs | undefined }[] = [];
    const listened: string[] = [];
    const client = new TauriClient({
      invoke: <T,>(cmd: string, args?: InvokeArgs): Promise<T> => {
        calls.push({ cmd, args });
        return Promise.resolve(null as T);
      },
      listen: <T,>(e: string, _h: (e: TauriEvent<T>) => void) => {
        listened.push(e);
        return Promise.resolve(() => undefined);
      },
    });
    await client.updateStatus();
    await client.updateCheck();
    await client.updateDownload();
    await client.updateRestart();
    await client.updateSkip("0.2.0");
    expect(calls).toEqual([
      { cmd: "update_status", args: undefined },
      { cmd: "update_check", args: undefined },
      { cmd: "update_download", args: undefined },
      { cmd: "update_restart", args: undefined },
      { cmd: "update_skip", args: { version: "0.2.0" } },
    ]);
    client.onUpdateAvailable(() => undefined);
    client.onUpdateProgress(() => undefined);
    client.onUpdateRestartAborted(() => undefined);
    expect(listened).toEqual(["update-available", "update-progress", "update-restart-aborted"]);
  });
});

describe("the browser mock's simulated server", () => {
  let store: Map<string, string>;
  beforeEach(() => {
    store = new Map();
    vi.stubGlobal("localStorage", {
      getItem: (k: string) => store.get(k) ?? null,
      setItem: (k: string, v: string) => void store.set(k, v),
      removeItem: (k: string) => void store.delete(k),
    });
  });
  afterEach(() => vi.unstubAllGlobals());

  const mock = () => {
    const m = new MockClient();
    m.updateHooks.available = { version: "0.2.0", notes: "n", date: null };
    m.updateHooks.autoDelayMs = 0;
    m.updateHooks.downloadStepMs = 0;
    return m;
  };
  const autoPrompt = async (m: MockClient) => {
    const seen: string[] = [];
    m.onUpdateAvailable((i) => seen.push(i.version));
    await new Promise((r) => setTimeout(r, 5));
    return seen;
  };

  it("announces once, honouring the pref and the skipped version", async () => {
    expect(await autoPrompt(mock())).toEqual(["0.2.0"]);
    store.set("tally.prefs", JSON.stringify({ checkForUpdates: false }));
    expect(await autoPrompt(mock())).toEqual([]);
    store.clear();
    await mock().updateSkip("0.2.0");
    expect(await autoPrompt(mock())).toEqual([]);
    // A manual check ignores the skip.
    expect((await mock().updateCheck())?.version).toBe("0.2.0");
    // Nothing newer: up to date.
    const same = mock();
    same.updateHooks.available = { version: "0.1.0", notes: "", date: null };
    expect(await same.updateCheck()).toBeNull();
  });

  it("downloads with progress, then counts the restart", async () => {
    const m = mock();
    await expect(m.updateRestart()).rejects.toMatchObject({ code: "invalid_argument" });
    await m.updateCheck();
    const progress: number[] = [];
    m.onUpdateProgress((p) => progress.push(p.received));
    const info = await m.updateDownload();
    expect(info.downloaded).toBe(true);
    expect(progress).toHaveLength(5);
    await m.updateRestart();
    expect(m.updateHooks.restartRequested).toBe(1);
    expect((await m.updateStatus()).staged?.version).toBe("0.2.0");
  });
});
