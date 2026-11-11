/**
 * The update dialog (updater.ts): "Tally <new> is available — you have
 * <current>" with the release notes as plain text and Skip This Version /
 * Later / Install and Restart; the download's progress (nothing to click:
 * nothing destructive happens meanwhile); "up to date"; errors; and
 * "Restart cancelled". Built like SavePrompt: shared dialog styles, focus
 * trap, Escape = Later (not while downloading). The release notes are
 * untrusted text from the release: rendered as a text node, never as HTML.
 */
import type { ReactNode } from "react";
import { useEffect, useRef } from "react";
import { cx, useController } from "../context";
import { t } from "../i18n";
import { megabytes, progressPercent, useUpdateStore, type UpdatePhase } from "../updater";
import { trapTab } from "./trapTab";
import dlg from "./Dialogs.module.css";
import styles from "./UpdatePrompt.module.css";

function titleOf(p: UpdatePhase): string {
  switch (p.kind) {
    case "checking":
      return t("update.checking", "Checking for updates…");
    case "available":
      return t("update.available", "Tally {version} is available — you have {current}", { version: p.info.version, current: p.info.currentVersion });
    case "downloading":
      return t("update.downloading", "Downloading Tally {version}…", { version: p.info.version });
    case "upToDate":
      return t("update.upToDate", "Tally is up to date");
    case "error":
      return p.what === "check"
        ? t("update.checkFailed", "Tally could not check for updates")
        : p.what === "download"
          ? t("update.downloadFailed", "The update could not be downloaded")
          : t("update.installFailed", "The update could not be installed");
    case "restartCancelled":
      return t("update.restartCancelled", "Restart cancelled");
  }
}

export function UpdatePrompt() {
  const phase = useUpdateStore((s) => s.phase);
  const ctl = useController();
  const formRef = useRef<HTMLFormElement>(null);
  const defaultRef = useRef<HTMLButtonElement>(null);
  const kind = phase?.kind ?? null;

  useEffect(() => {
    if (kind === null) return;
    // The default button, or the dialog itself while nothing can be clicked.
    if (defaultRef.current) defaultRef.current.focus();
    else formRef.current?.focus();
  }, [kind]);

  if (!phase) return null;
  const up = ctl.updater;
  const busy = phase.kind === "downloading";
  let body: ReactNode = null;
  let actions: ReactNode = null;
  let submit: (() => void) | null = null;

  if (phase.kind === "available" || (phase.kind === "restartCancelled" && phase.info)) {
    const info = phase.kind === "available" ? phase.info : phase.info!;
    submit = () => void up.install(info);
    body =
      phase.kind === "available" ? (
        <>
          <p id="update-prompt-detail" className={dlg.lead}>
            {t("update.availableLead", "Install it now? Tally restarts to finish, and asks about unsaved documents first.")}
            {info.date ? ` ${t("update.released", "Released {date}.", { date: info.date })}` : ""}
          </p>
          <div className={styles.notesLabel} id="update-notes-label">
            {t("update.notes", "Release notes")}
          </div>
          <div className={styles.notes} role="region" aria-labelledby="update-notes-label" tabIndex={0} data-testid="update-notes">
            {info.notes || t("update.noNotes", "No release notes were published for this version.")}
          </div>
        </>
      ) : (
        <p id="update-prompt-detail" className={dlg.lead}>
          {t("update.restartCancelledLead", "Tally {version} is downloaded but not installed, and every document is still open. Choose Install and Restart when you are ready. If you quit Tally instead, the update is not installed; it is offered again later.", { version: info.version })}
        </p>
      );
    actions = (
      <>
        {phase.kind === "available" ? (
          <div className={dlg.extra}>
            <button type="button" className={dlg.secondary} onClick={() => void up.skip(info.version)} data-testid="update-skip">
              {t("update.skip", "Skip This Version")}
            </button>
          </div>
        ) : null}
        <button type="button" className={dlg.secondary} onClick={up.dismiss} data-testid="update-later">
          {t("update.later", "Later")}
        </button>
        <button ref={defaultRef} type="submit" className={dlg.primary} data-testid="update-install">
          {t("update.install", "Install and Restart")}
        </button>
      </>
    );
  } else if (phase.kind === "downloading") {
    const pct = progressPercent(phase.received, phase.total);
    body = (
      <>
        <p id="update-prompt-detail" className={dlg.lead}>
          {t("update.downloadingLead", "Its signature is checked before anything is installed. Tally then asks about unsaved documents and restarts.")}
        </p>
        <progress className={styles.progress} max={100} value={pct ?? undefined} aria-label={t("update.progressLabel", "Download progress")} data-testid="update-progress" />
        <p className={styles.status} aria-live="polite">
          {pct !== null ? t("update.progress", "{percent}% downloaded", { percent: pct }) : t("update.progressBytes", "{mb} MB downloaded", { mb: megabytes(phase.received) })}
        </p>
      </>
    );
  } else if (phase.kind === "checking") {
    body = <progress className={styles.progress} aria-label={t("update.checking", "Checking for updates…")} />;
    actions = (
      <button ref={defaultRef} type="button" className={dlg.secondary} onClick={up.dismiss} data-testid="update-close">
        {t("dialog.cancel", "Cancel")}
      </button>
    );
  } else {
    // up to date, error, cancelled without a known update
    const lead =
      phase.kind === "upToDate"
        ? t("update.upToDateLead", "You have the newest version, {version}.", { version: phase.version })
        : phase.kind === "error"
          ? phase.what === "install"
            ? t("update.installFailedLead", "Nothing was closed. Your documents are still open; try again later.")
            : t("update.failedLead", "Nothing was changed. Check your internet connection and try again later.")
          : t("update.restartCancelledShort", "Nothing was closed and nothing was installed.");
    body = (
      <>
        <p id="update-prompt-detail" className={dlg.lead}>
          {lead}
        </p>
        {phase.kind === "error" && phase.message ? (
          <p className={styles.detail} data-testid="update-error">
            {phase.message}
          </p>
        ) : null}
      </>
    );
    actions = (
      <button ref={defaultRef} type="submit" className={dlg.primary} data-testid="update-ok">
        {t("dialog.ok", "OK")}
      </button>
    );
    submit = up.dismiss;
  }

  return (
    <div className={cx(dlg.overlay, styles.overlay)}>
      <form
        ref={formRef}
        className={cx(dlg.card, dlg.wide, styles.card)}
        role={phase.kind === "error" ? "alertdialog" : "dialog"}
        aria-modal="true"
        aria-labelledby="update-prompt-title"
        aria-describedby={phase.kind === "checking" ? undefined : "update-prompt-detail"}
        aria-busy={busy || phase.kind === "checking"}
        tabIndex={-1}
        data-testid="update-prompt"
        data-phase={phase.kind}
        onKeyDown={(e) => {
          e.stopPropagation();
          if (e.key === "Escape") {
            e.preventDefault();
            up.dismiss();
            return;
          }
          if (formRef.current) trapTab(formRef.current, e, document.activeElement);
        }}
        onSubmit={(e) => {
          e.preventDefault();
          submit?.();
        }}
      >
        <h2 id="update-prompt-title" className={dlg.title}>
          {titleOf(phase)}
        </h2>
        {body}
        {actions ? <div className={dlg.actions}>{actions}</div> : null}
      </form>
    </div>
  );
}
