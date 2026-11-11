/**
 * Auto-update UI (docs/release.md): the update prompt, Help ▸ Check for
 * Updates… and the restart. The checking, downloading, signature check
 * and install happen in the desktop shell (`updater.rs`, through
 * {@link UpdateShell}); this module only asks and shows.
 *
 * - An automatic check (shell timer, pref `checkForUpdates`) sends
 *   `update-available` to one document, the front one, which shows
 *   "Tally <new> is available — you have <current>" with the release notes
 *   (plain text) and Install and Restart / Later / Skip This Version.
 * - Check for Updates… asks now: the prompt, "up to date", or the error.
 * - Install and Restart downloads (progress, nothing else to click), then
 *   asks the shell to restart: it runs the unsaved-changes questions of
 *   every open document like Quit and installs once all answered. Cancel
 *   there keeps everything open and the update downloaded
 *   (`update-restart-aborted`, shown as "Restart cancelled").
 * - In the browser build (MockClient, one document, no window shell) the
 *   unsaved-changes guard runs here before the restart.
 */
import { isApiError, type RestartAborted, type UpdateInfo, type UpdateProgress, type UpdateShell, type UpdateStatus } from "@tally/bridge";
import { create } from "zustand";

function messageOf(e: unknown): string {
  if (isApiError(e)) return e.message;
  return e instanceof Error ? e.message : String(e);
}

/** What the update dialog shows. */
export type UpdatePhase =
  | { kind: "checking" }
  | { kind: "available"; info: UpdateInfo; manual: boolean }
  | { kind: "downloading"; info: UpdateInfo; received: number; total: number | null }
  | { kind: "upToDate"; version: string }
  | { kind: "error"; what: "check" | "download" | "install"; message: string }
  | { kind: "restartCancelled"; info: UpdateInfo | null };

export interface UpdateState {
  phase: UpdatePhase | null;
  /** The downloaded update waiting for the restart (offered again after a cancel). */
  staged: UpdateInfo | null;
  set(phase: UpdatePhase | null): void;
}

export const useUpdateStore = create<UpdateState>()((set) => ({
  phase: null,
  staged: null,
  set: (phase) => set({ phase }),
}));

/** Pure: whole percent downloaded, or `null` when the size is unknown. */
export function progressPercent(received: number, total: number | null): number | null {
  if (total === null || !(total > 0)) return null;
  return Math.max(0, Math.min(100, Math.floor((received * 100) / total)));
}

/** Pure: megabytes with one decimal (`12.3`). */
export function megabytes(bytes: number): string {
  return (Math.max(0, bytes) / (1024 * 1024)).toFixed(1);
}

/**
 * Pure: whether an automatic `update-available` may open the prompt: not
 * over another update dialog (a manual check, a download in progress).
 */
export function acceptsAutomatic(phase: UpdatePhase | null): boolean {
  return phase === null;
}

/** Pure: the progress state after an `update-progress` event (only while downloading). */
export function withProgress(phase: UpdatePhase | null, p: UpdateProgress): UpdatePhase | null {
  if (phase?.kind !== "downloading") return phase;
  return { ...phase, received: p.received, total: p.total };
}

/** What the feature needs from the controller. */
export interface UpdateHost {
  /** The browser build's unsaved-changes guard (Save / Don't Save / Cancel); `true` = go on. */
  confirmUnsaved(): Promise<boolean>;
  /** Desktop (the shell asks every document itself). */
  multiWindow: boolean;
}

/** The update actions of one document's controller. */
export class UpdateFeature {
  constructor(
    private readonly shell: UpdateShell,
    private readonly host: UpdateHost,
  ) {}

  private get store(): UpdateState {
    return useUpdateStore.getState();
  }

  /** Whether this client can update at all (the shell has the commands). */
  get available(): boolean {
    return typeof this.shell.updateCheck === "function";
  }

  /** Subscribes to the shell's update events; returns the cleanup. */
  attach(): () => void {
    const offs = [
      this.shell.onUpdateAvailable?.((info) => {
        if (acceptsAutomatic(this.store.phase)) this.store.set({ kind: "available", info, manual: false });
      }),
      this.shell.onUpdateProgress?.((p) => this.store.set(withProgress(this.store.phase, p))),
      this.shell.onUpdateRestartAborted?.((e) => this.restartAborted(e)),
    ];
    return () => offs.forEach((off) => off?.());
  }

  /** `update_status` (`null` without an updater). */
  async status(): Promise<UpdateStatus | null> {
    if (!this.shell.updateStatus) return null;
    try {
      return await this.shell.updateStatus();
    } catch {
      return null;
    }
  }

  /** Whether the dialog still shows "Checking for updates…". */
  private checking(): boolean {
    return useUpdateStore.getState().phase?.kind === "checking";
  }

  /** Help ▸ Check for Updates…: the prompt, "up to date" or the error. */
  checkNow = async (): Promise<void> => {
    const check = this.shell.updateCheck;
    if (!check) {
      this.store.set({ kind: "error", what: "check", message: "" });
      return;
    }
    const busy = this.store.phase?.kind;
    if (busy === "downloading" || busy === "checking") return;
    this.store.set({ kind: "checking" });
    try {
      const info = await check.call(this.shell);
      // Closed meanwhile: the answer is not wanted any more.
      if (!this.checking()) return;
      if (info) {
        this.store.set({ kind: "available", info, manual: true });
      } else {
        const status = await this.status();
        this.store.set({ kind: "upToDate", version: status?.currentVersion ?? "" });
      }
    } catch (e) {
      if (this.checking()) this.store.set({ kind: "error", what: "check", message: messageOf(e) });
    }
  };

  /** Later (and closing any of the informational states). */
  dismiss = (): void => {
    if (this.store.phase?.kind === "downloading") return;
    this.store.set(null);
  };

  /** Skip This Version: automatic checks stay silent until a newer one. */
  skip = async (version: string): Promise<void> => {
    this.store.set(null);
    try {
      await this.shell.updateSkip?.(version);
    } catch (e) {
      console.warn("tally: update_skip", e);
    }
  };

  /** Install and Restart: download (with progress) unless done already, then restart. */
  install = async (info: UpdateInfo): Promise<void> => {
    let ready = info;
    if (!info.downloaded) {
      const download = this.shell.updateDownload;
      if (!download) return;
      this.store.set({ kind: "downloading", info, received: 0, total: null });
      try {
        ready = await download.call(this.shell);
      } catch (e) {
        this.store.set({ kind: "error", what: "download", message: messageOf(e) });
        return;
      }
    }
    useUpdateStore.setState({ staged: ready });
    await this.restart(ready);
  };

  /**
   * The restart: the dialog closes so the unsaved-changes questions can be
   * answered; the shell asks every edited document, installs and relaunches.
   */
  private async restart(info: UpdateInfo): Promise<void> {
    this.store.set(null);
    if (!this.host.multiWindow && !(await this.host.confirmUnsaved())) {
      this.restartAborted({ cancelled: true, message: null }, info);
      return;
    }
    try {
      await this.shell.updateRestart?.();
    } catch (e) {
      this.store.set({ kind: "error", what: "install", message: messageOf(e) });
    }
  }

  private restartAborted(e: RestartAborted, info: UpdateInfo | null = this.store.staged): void {
    if (e.cancelled) this.store.set({ kind: "restartCancelled", info });
    else this.store.set({ kind: "error", what: "install", message: e.message ?? "" });
  }
}
