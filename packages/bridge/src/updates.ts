/**
 * Auto-update: the shell commands and events of
 * `apps/desktop/src-tauri/src/updater.rs` (docs/release.md). The check,
 * download, signature check and install all happen in Rust; the UI only
 * asks and shows. `TauriClient` implements {@link UpdateShell}; the
 * browser build's `MockClient` simulates it (`updateHooks`) for e2e.
 */

/** An update offered to the user (`UpdateInfo` in updater.rs). */
export interface UpdateInfo {
  /** The new version (`0.2.0`). */
  version: string;
  /** The running version. */
  currentVersion: string;
  /** Release notes, plain text (untrusted: never render as HTML). */
  notes: string;
  /** Release date `YYYY-MM-DD`, when known. */
  date: string | null;
  /** Downloaded and verified this session: Restart installs it. */
  downloaded: boolean;
}

/** `update-progress`: bytes received (and the total when known). */
export interface UpdateProgress {
  received: number;
  total: number | null;
}

/** `update-restart-aborted`: the restart to update ended without restarting. */
export interface RestartAborted {
  /** Cancel in an unsaved-changes prompt; otherwise the install failed (`message`). */
  cancelled: boolean;
  message: string | null;
}

/** `update_status`. */
export interface UpdateStatus {
  /** The running version. */
  currentVersion: string;
  /** Whether this build checks for updates (release builds; not dev builds). */
  supported: boolean;
  /** Why not. */
  reason: string | null;
  /** An update downloaded this session, waiting for the restart. */
  staged: UpdateInfo | null;
}

/** Rejection code when this build or installation does not update itself (`ApiError::UNSUPPORTED`). */
export const UPDATES_UNAVAILABLE = "unsupported";

/** The updater's shell surface (optional: a client without it has no updates). */
export interface UpdateShell {
  /** `update_status`. */
  updateStatus?(): Promise<UpdateStatus>;
  /** `update_check`: a manual check; `null` when up to date (ignores a skipped version). */
  updateCheck?(): Promise<UpdateInfo | null>;
  /** `update_download`: downloads and verifies the offered update (progress through {@link onUpdateProgress}). */
  updateDownload?(): Promise<UpdateInfo>;
  /**
   * `update_restart`: asks every edited document (Save / Don't Save /
   * Cancel), then installs and relaunches. Resolves once the questions
   * started; Cancel arrives as {@link onUpdateRestartAborted}.
   */
  updateRestart?(): Promise<void>;
  /** `update_skip`: automatic checks stay silent for this version (and older). */
  updateSkip?(version: string): Promise<void>;
  /** `update-available`: an automatic check found an update (sent to one document only). */
  onUpdateAvailable?(cb: (info: UpdateInfo) => void): () => void;
  /** `update-progress`. */
  onUpdateProgress?(cb: (p: UpdateProgress) => void): () => void;
  /** `update-restart-aborted`. */
  onUpdateRestartAborted?(cb: (e: RestartAborted) => void): () => void;
}

/**
 * Semver order of two `x.y.z[-pre]` versions (a `v` prefix is allowed):
 * negative, zero or positive; `null` when either is not a version. The
 * browser mock's stand-in for the shell's comparison (`updater.rs`).
 */
export function compareVersions(a: string, b: string): number | null {
  const parse = (v: string) => /^v?(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?$/.exec(v.trim());
  const x = parse(a);
  const y = parse(b);
  if (!x || !y) return null;
  for (let i = 1; i <= 3; i++) {
    const d = Number(x[i]) - Number(y[i]);
    if (d !== 0) return d;
  }
  const px = x[4];
  const py = y[4];
  if (px === undefined || py === undefined) return px === py ? 0 : px === undefined ? 1 : -1;
  return px < py ? -1 : px > py ? 1 : 0;
}

/**
 * Whether a check announces `version` (`should_prompt` in updater.rs): a
 * manual check always does; an automatic one stays silent for the skipped
 * version and anything older.
 */
export function shouldPrompt(version: string, skipped: string | null, manual: boolean): boolean {
  if (manual || skipped === null) return true;
  const c = compareVersions(version, skipped);
  return c === null || c > 0;
}
