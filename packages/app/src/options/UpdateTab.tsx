/**
 * Tools ▸ Options ▸ Online Update (AOO's page: Tools ▸ Options ▸ OpenOffice
 * ▸ Online Update): "Check for updates automatically" (pref
 * `checkForUpdates`, default on) and the installed version. A build that
 * does not update itself (dev builds, Linux .deb/.rpm installs) says why.
 */
import type { UpdateStatus } from "@tally/bridge";
import { useEffect, useState } from "react";
import dlg from "../components/Dialogs.module.css";
import { useController } from "../context";
import { Check } from "../data/W56Shell";
import { t } from "../i18n";

export function UpdateTab({ checked, onChange }: { checked: boolean; onChange: (v: boolean) => void }) {
  const ctl = useController();
  const [status, setStatus] = useState<UpdateStatus | null>(null);
  useEffect(() => {
    let live = true;
    void ctl.updater.status().then((s) => {
      if (live) setStatus(s);
    });
    return () => {
      live = false;
    };
  }, [ctl]);
  return (
    <div>
      <Check label={t("options.checkForUpdates", "Check for updates automatically")} checked={checked} onChange={onChange} testId="opt-check-updates" />
      <p className={dlg.note} data-testid="opt-update-version">
        {status ? t("options.installedVersion", "Installed version: Tally {version}", { version: status.currentVersion }) : null}
      </p>
      {status && !status.supported && status.reason ? (
        <p className={dlg.note} data-testid="opt-update-unsupported">
          {status.reason}
        </p>
      ) : null}
      <p className={dlg.note}>
        {t(
          "options.updateNote",
          "Once a day Tally asks GitHub whether a new version is out: one HTTPS request that sends nothing about you or your documents. Updates are signed and are never installed without your click. Check for Updates… (Help menu; Tally menu on macOS) checks now.",
        )}
      </p>
    </div>
  );
}
