/**
 * Navigator (F5, §4 item 39): sheets, range names, database ranges, charts,
 * pictures, pivot tables, notes and scenarios from `navigator()`. Single
 * click selects a sheet; double-click (or Enter) jumps to the item;
 * scenarios have a Show button; pivots open the layout from the context
 * button. Refreshed on every engine result (`rev`).
 */
import type { NavigatorDto, NavItem } from "@tally/bridge";
import { useEffect, useState } from "react";
import { cx, useController } from "../context";
import { a1 } from "../dataOps";
import { t } from "../i18n";
import { useDocStore } from "../stores/docStore";
import { useW56Store } from "../data/store";
import w56 from "../data/W56.module.css";

const EMPTY: NavigatorDto = { sheets: [], names: [], databaseRanges: [], charts: [], images: [], pivots: [], notes: [], scenarios: [] };

const GROUPS: readonly { key: keyof NavigatorDto; tKey: string; label: string }[] = [
  { key: "sheets", tKey: "navigator.sheets", label: "Sheets" },
  { key: "names", tKey: "navigator.rangeNames", label: "Range names" },
  { key: "databaseRanges", tKey: "navigator.dataRanges", label: "Database ranges" },
  { key: "charts", tKey: "navigator.charts", label: "Charts" },
  { key: "images", tKey: "cmd.insert.image", label: "Pictures" },
  { key: "pivots", tKey: "menu.data.dataPilot", label: "DataPilot" },
  { key: "notes", tKey: "navigator.notes", label: "Notes" },
  { key: "scenarios", tKey: "cmd.tools.scenarios", label: "Scenarios" },
];

export function NavigatorPanel() {
  const ctl = useController();
  const doc = useDocStore((s) => s.info?.doc);
  const rev = useDocStore((s) => s.rev);
  const sheets = useDocStore((s) => s.info?.sheets ?? []);
  useW56Store((s) => s.i18nRev);
  const pivots = useW56Store((s) => s.pivots);
  const [nav, setNav] = useState<NavigatorDto>(EMPTY);
  const [open, setOpen] = useState<Record<string, boolean>>({ sheets: true, names: true, databaseRanges: true, charts: true, images: false, pivots: true, notes: true, scenarios: true });

  useEffect(() => {
    if (doc === undefined) return undefined;
    let live = true;
    const timer = setTimeout(() => {
      ctl.w56.navigator().then(
        (n) => live && setNav(n),
        () => live && setNav(EMPTY),
      );
    }, 80);
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [ctl, doc, rev]);

  const go = (it: NavItem) => ctl.w56.goToItem(it.sheet, it.range);
  const sheetName = (id: number | null) => sheets.find((s) => s.id === id)?.name ?? "";
  return (
    <aside className={w56.panel} aria-label={t("navigator.title", "Navigator")} data-testid="navigator-panel">
      <div className={w56.panelHead}>
        <span className={w56.grow}>{t("navigator.title", "Navigator")}</span>
        <button type="button" className={w56.iconBtn} aria-label={t("dialog.close", "Close")} onClick={() => useW56Store.getState().setPanel(null)}>
          ×
        </button>
      </div>
      <div className={w56.panelBody}>
        {GROUPS.map((g) => {
          const items = nav[g.key];
          const isOpen = open[g.key] ?? true;
          return (
            <div key={g.key} className={w56.navGroup} data-testid={`nav-${g.key}`}>
              <button type="button" className={w56.navHead} aria-expanded={isOpen} onClick={() => setOpen({ ...open, [g.key]: !isOpen })}>
                <span>{isOpen ? "▾" : "▸"}</span>
                <span className={w56.grow}>{t(g.tKey, g.label).replace(/…$/, "")}</span>
                <span className={w56.muted}>{items.length}</span>
              </button>
              {isOpen
                ? items.map((it, i) => (
                    <button
                      key={`${g.key}-${i}`}
                      type="button"
                      className={cx(w56.navItem)}
                      title={it.range ? `${sheetName(it.sheet)}.${a1(it.range)}` : undefined}
                      onClick={() => g.key === "sheets" && it.sheet !== null && ctl.showSheet(it.sheet)}
                      onDoubleClick={() => go(it)}
                      onKeyDown={(e) => e.key === "Enter" && go(it)}
                      data-testid={`nav-item-${g.key}-${i}`}
                    >
                      <span className={w56.grow}>{it.title || "(untitled)"}</span>
                      {g.key === "scenarios" && it.id !== null && it.sheet !== null ? (
                        <span
                          role="button"
                          tabIndex={0}
                          className={w56.muted}
                          onClick={(e) => {
                            e.stopPropagation();
                            if (it.sheet !== null && it.sheet !== ctl.sheet) ctl.showSheet(it.sheet, false);
                            void ctl.w56.showScenario(it.id as number);
                          }}
                        >
                          {t("w56shell.show", "Show")}
                        </span>
                      ) : g.key === "pivots" && it.id !== null ? (
                        <span
                          role="button"
                          tabIndex={0}
                          className={w56.muted}
                          onClick={(e) => {
                            e.stopPropagation();
                            const p = pivots.find((x) => x.id === it.id) ?? null;
                            void ctl.w56.openDataPilot(p);
                          }}
                        >
                          {t("pivot.layout", "Layout")}
                        </span>
                      ) : it.range ? (
                        <span className={w56.muted}>{a1(it.range)}</span>
                      ) : null}
                    </button>
                  ))
                : null}
            </div>
          );
        })}
      </div>
    </aside>
  );
}
