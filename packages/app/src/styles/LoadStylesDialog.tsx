/**
 * Load Styles (Styles panel ▸ Load Styles…, Format ▸ Load Styles…): copies
 * the cell styles and / or the page format of a template or a file into
 * the document. Modelled on AOO's load-template dialog
 * (`SfxNewFileDialog` with `SFXWB_LOAD_TEMPLATE`: the template list, a
 * "From File…" button and one checkbox per style family plus Overwrite)
 * with Calc's two families, the options of `XStyleLoader::loadStylesFromURL`.
 */
import type { LoadStylesOptions, TemplateInfo } from "@tally/bridge";
import { useEffect, useState } from "react";
import dlg from "../components/Dialogs.module.css";
import { useController } from "../context";
import { t } from "../i18n";
import { useW56Store } from "../data/store";
import { Check, W56Shell, closeW56 } from "../data/W56Shell";
import w56 from "../data/W56.module.css";
import { loadStylesValid } from "./loadStyles";

/** The last choices (AOO's dialog starts with the first family checked). */
let lastOptions: LoadStylesOptions = { cellStyles: true, pageStyles: false, overwrite: false };

export function LoadStylesDialog() {
  useW56Store((s) => s.i18nRev);
  const ctl = useController();
  const [templates, setTemplates] = useState<TemplateInfo[]>([]);
  const [picked, setPicked] = useState<string | null>(null);
  const [opts, setOptsState] = useState<LoadStylesOptions>(lastOptions);
  const setOpts = (o: LoadStylesOptions) => {
    lastOptions = o;
    setOptsState(o);
  };
  useEffect(() => {
    let live = true;
    void Promise.all([ctl.shell.listTemplates().catch(() => [] as TemplateInfo[]), ctl.w56.listUserTemplates()]).then(([builtin, user]) => {
      if (live) setTemplates([...builtin, ...user]);
    });
    return () => {
      live = false;
    };
  }, [ctl]);
  const valid = loadStylesValid(opts, picked);
  const fromFile = ctl.w56.canLoadStylesFromFile();
  const load = (id: string) => void ctl.w56.loadStyles({ template: id }, opts);
  return (
    <W56Shell
      title={t("loadStyles.title", "Load Styles")}
      ok={t("dialog.ok", "OK")}
      testId="load-styles"
      valid={valid}
      onSubmit={() => {
        if (picked) load(picked);
      }}
      extra={
        fromFile ? (
          <button
            type="button"
            className={dlg.secondary}
            disabled={!loadStylesValid(opts, "file")}
            onClick={() => {
              closeW56();
              void ctl.w56.loadStyles({ file: true }, opts);
            }}
            data-testid="load-styles-file"
          >
            {t("loadStyles.fromFile", "From File…")}
          </button>
        ) : undefined
      }
    >
      <p className={dlg.lead}>{t("loadStyles.lead", "Copies the styles of a template or another document into this one.")}</p>
      <div className={w56.list} role="listbox" aria-label={t("templates.title", "Templates")} style={{ maxHeight: 220 }} data-testid="load-styles-list">
        {templates.map((x) => (
          <button
            key={x.id}
            type="button"
            role="option"
            aria-selected={x.id === picked}
            className={x.id === picked ? w56.listItemOn : w56.listItem}
            title={x.description || undefined}
            onClick={() => setPicked(x.id)}
            onDoubleClick={() => {
              if (!loadStylesValid(opts, x.id)) return;
              closeW56();
              load(x.id);
            }}
            data-testid={`load-styles-template-${x.id}`}
          >
            {x.title}
          </button>
        ))}
        {templates.length === 0 ? <div className={w56.muted}>{t("loadStyles.noTemplates", "No templates")}</div> : null}
      </div>
      <div className={w56.section}>{t("loadStyles.families", "Styles to load")}</div>
      <div className={dlg.optionGrid}>
        <Check label={t("loadStyles.cellStyles", "Cell Styles")} checked={opts.cellStyles} onChange={(v) => setOpts({ ...opts, cellStyles: v })} testId="load-styles-cell" />
        <Check label={t("loadStyles.pageStyles", "Page Styles")} checked={opts.pageStyles} onChange={(v) => setOpts({ ...opts, pageStyles: v })} testId="load-styles-page" />
        <Check label={t("loadStyles.overwrite", "Overwrite")} checked={opts.overwrite} onChange={(v) => setOpts({ ...opts, overwrite: v })} testId="load-styles-overwrite" />
      </div>
      <p className={dlg.note}>{t("loadStyles.note", "Without Overwrite only styles with new names are added. The page format (first sheet of the source) replaces every sheet's only with Overwrite; print ranges stay.")}</p>
    </W56Shell>
  );
}
