/**
 * The formatting strip under the toolbar: B I U S, font name and size
 * (editable combo boxes, FontPickers.tsx), font and
 * fill colour, alignment, wrap, number format, $ / %, "Highlight cells"
 * (conditional formatting presets) and "Colour by value". Every toggle reflects the active cell's style; every action
 * applies to all selected ranges. Buttons can be hidden (right-click ▸
 * Visible Buttons, View ▸ Toolbars ▸ Customize Toolbars…; separators
 * collapse) and those that do not fit go into "»" (chrome/ChromeUi.tsx).
 */
import type { HAlign } from "@tally/bridge";
import { Fragment, useState, type ReactNode } from "react";
import { OverflowRow, useBarMenu } from "../chrome/ChromeUi";
import { FORMATTING_SECTIONS, slotsOf, visibleSections, type Slot } from "../chrome/chromeModel";
import { useW56Store } from "../data/store";
import { cx, useController } from "../context";
import { NUMBER_FORMATS, presetFor } from "../formats";
import { useActiveStyle } from "../hooks";
import { rgbaToHex } from "../palette";
import { useUiStore } from "../stores/uiStore";
import { ColorPicker } from "./ColorPicker";
import { BordersPicker } from "./BordersPicker";
import { FontNameBox, FontSizeBox, useFontInfo } from "./FontPickers";
import { AlignCenterIcon, AlignLeftIcon, AlignRightIcon, BordersIcon, ChevronIcon, ChipsIcon, FillIcon, HighlightIcon, MergeIcon, WrapIcon } from "./icons";
import { HIGHLIGHT_PRESETS } from "../v03";
import { useSelectionStore } from "../stores/selectionStore";
import { useDocStore } from "../stores/docStore";
import { Popover } from "./Popover";
import { arrowNav } from "./dialogA11y";
import { t } from "../i18n";
import styles from "./FormatBar.module.css";

interface ToolProps {
  command: string;
  label: string;
  children: ReactNode;
  active?: boolean;
  wide?: boolean;
  /** Icon-only form of a wide button (the label stays the tooltip and aria-label). */
  compact?: boolean;
}

/** An icon button bound to a CommandRegistry command. */
function Tool({ command, label, children, active, wide, compact }: ToolProps) {
  const ctl = useController();
  const cmd = ctl.registry.get(command);
  const tip = [label, cmd?.shortcut].filter(Boolean).join("  ");
  return (
    <button
      type="button"
      className={cx(styles.tool, wide && styles.wide, compact && styles.compact, active && styles.on)}
      aria-label={label}
      aria-pressed={active === undefined ? undefined : active}
      data-command={command}
      title={tip}
      onMouseDown={(e) => e.preventDefault() /* keep focus in the grid */}
      onClick={() => {
        if (ctl.registry.run(command)) ctl.focusGrid();
      }}
    >
      {children}
    </button>
  );
}

type Menu = { kind: "format" | "color" | "fill" | "borders" | "highlight"; x: number; y: number } | null;

export function FormatBar() {
  const ctl = useController();
  const style = useActiveStyle();
  const [menu, setMenu] = useState<Menu>(null);
  const open = (kind: NonNullable<Menu>["kind"]) => (e: React.MouseEvent<HTMLButtonElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    setMenu(menu?.kind === kind ? null : { kind, x: r.left, y: r.bottom + 6 });
  };
  const h: HAlign = style?.hAlign ?? "general";
  const preset = presetFor(style?.numberFormat);
  const formatLabel = preset ? preset.label : style ? "Custom" : "General";
  const textColor = style?.color != null ? rgbaToHex(style.color) : "var(--tally-text)";
  const fillColor = style?.fill != null && (style.fill & 255) > 0 ? rgbaToHex(style.fill) : "transparent";
  const done = () => ctl.focusGrid();
  // Re-render on selection / layout changes for the merge toggle.
  useSelectionStore((s) => s.selection);
  useDocStore((s) => s.rev);
  const merged = ctl.selectionIsMerged();
  const hidden = useW56Store((s) => s.prefs.chrome.hidden.formatting);
  const fontInfo = useFontInfo();
  const barMenu = useBarMenu("formatting");

  // The two wide buttons drop their labels when the strip is short of room (before "»").
  const highlightButton = (compact: boolean) => (
    <button
      type="button"
      className={cx(styles.tool, styles.wide, compact && styles.compact)}
      aria-label="Highlight cells"
      aria-haspopup="menu"
      title="Highlight cells: conditional formatting presets"
      data-testid="highlight-cells"
      onMouseDown={(e) => e.preventDefault()}
      onClick={open("highlight")}
    >
      <HighlightIcon />
      {compact ? null : <span>Highlight</span>}
      <ChevronIcon />
    </button>
  );
  const colorByValueButton = (compact: boolean) => (
    <Tool command="format.colorByValue" label="Colour by value: chip colours for each distinct value in the column" wide compact={compact}>
      <ChipsIcon />
      {compact ? null : <span>Colour by value</span>}
    </Tool>
  );
  const items: Record<string, ReactNode> = {
    fontName: <FontNameBox onDone={done} info={fontInfo} />,
    fontSize: <FontSizeBox onDone={done} info={fontInfo} />,
    bold: (
      <Tool command="format.bold" label="Bold" active={style?.bold ?? false}>
        <span className={styles.bold}>B</span>
      </Tool>
    ),
    italic: (
      <Tool command="format.italic" label="Italic" active={style?.italic ?? false}>
        <span className={styles.italic}>I</span>
      </Tool>
    ),
    underline: (
      <Tool command="format.underline" label="Underline" active={style?.underline ?? false}>
        <span className={styles.underline}>U</span>
      </Tool>
    ),
    strike: (
      <Tool command="format.strike" label="Strikethrough" active={style?.strike ?? false}>
        <span className={styles.strike}>S</span>
      </Tool>
    ),
    fontColor: (
      <button
        type="button"
        className={cx(styles.tool, styles.colorTool)}
        aria-label="Font colour"
        aria-haspopup="dialog"
        title="Font colour"
        data-testid="font-color"
        onMouseDown={(e) => e.preventDefault()}
        onClick={open("color")}
      >
        <span className={styles.letterA}>A</span>
        <span className={styles.colorBar} style={{ background: textColor }} />
      </button>
    ),
    fillColor: (
      <button
        type="button"
        className={cx(styles.tool, styles.colorTool)}
        aria-label="Fill colour"
        aria-haspopup="dialog"
        title="Fill colour"
        data-testid="fill-color"
        onMouseDown={(e) => e.preventDefault()}
        onClick={open("fill")}
      >
        <FillIcon />
        <span className={cx(styles.colorBar, fillColor === "transparent" && styles.emptyBar)} style={{ background: fillColor }} />
      </button>
    ),
    alignLeft: (
      <Tool command="format.alignLeft" label="Align left" active={h === "left"}>
        <AlignLeftIcon />
      </Tool>
    ),
    alignCenter: (
      <Tool command="format.alignCenter" label="Align centre" active={h === "center"}>
        <AlignCenterIcon />
      </Tool>
    ),
    alignRight: (
      <Tool command="format.alignRight" label="Align right" active={h === "right"}>
        <AlignRightIcon />
      </Tool>
    ),
    wrap: (
      <Tool command="format.wrap" label="Wrap text" active={style?.wrap ?? false}>
        <WrapIcon />
      </Tool>
    ),
    merge: (
      <Tool command="format.toggleMerge" label={merged ? "Unmerge cells" : "Merge cells"} active={merged}>
        <MergeIcon />
      </Tool>
    ),
    borders: (
      <button
        type="button"
        className={styles.tool}
        aria-label="Borders"
        aria-haspopup="dialog"
        title="Borders"
        data-testid="borders"
        onMouseDown={(e) => e.preventDefault()}
        onClick={open("borders")}
      >
        <BordersIcon />
      </button>
    ),
    numberFormat: (
      <button
        type="button"
        className={cx(styles.dropdown, styles.format)}
        aria-label={`${t("toolbar.numberFormat", "Number format")}: ${formatLabel}`}
        aria-haspopup="menu"
        title="Number format"
        data-testid="number-format"
        onMouseDown={(e) => e.preventDefault()}
        onClick={open("format")}
      >
        <span>{formatLabel}</span>
        <ChevronIcon />
      </button>
    ),
    currency: (
      <Tool command="format.currency" label="Format as currency" active={preset?.id === "currency"}>
        <span className={styles.glyph}>$</span>
      </Tool>
    ),
    percent: (
      <Tool command="format.percent" label="Format as percent" active={preset?.id === "percent"}>
        <span className={styles.glyph}>%</span>
      </Tool>
    ),
    highlight: highlightButton(false),
    colorByValue: colorByValueButton(false),
  };
  const slots = slotsOf(visibleSections(FORMATTING_SECTIONS, hidden));
  const renderSlot = (s: Slot, compact: boolean): ReactNode => {
    if (s.kind === "sep") return <span className={styles.sep} />;
    if (compact && s.el === "highlight") return highlightButton(true);
    if (compact && s.el === "colorByValue") return colorByValueButton(true);
    if (typeof s.el === "string") return items[s.el];
    return (
      <div className={styles.group}>
        {s.el.map((id) => (
          <Fragment key={id}>{items[id]}</Fragment>
        ))}
      </div>
    );
  };

  return (
    <div className={styles.bar} role="toolbar" aria-label="Format" data-testid="format-bar" onContextMenu={barMenu.onContextMenu} onKeyDown={(e) => void arrowNav(e)}>
      <OverflowRow slots={slots} renderSlot={renderSlot} compactable={(s) => s.kind === "el" && (s.el === "highlight" || s.el === "colorByValue")} testId="format-row" className={styles.row} />
      {menu?.kind === "format" ? (
        <Popover
          label="Number format"
          testId="number-format-menu"
          x={menu.x}
          y={menu.y}
          onClose={() => setMenu(null)}
          items={[
            ...NUMBER_FORMATS.map((f) => ({
              id: f.id,
              label: f.label,
              hint: f.sample,
              checked: preset?.id === f.id,
              onSelect: () => {
                void ctl.setNumberFormat(f.code, f.label);
                done();
              },
            })),
            {
              id: "custom",
              label: "Custom…",
              separator: true,
              checked: !preset && style !== undefined,
              onSelect: () => useUiStore.getState().openDialog({ kind: "numberFormat", code: style?.numberFormat ?? "General" }),
            },
          ]}
        />
      ) : null}
      {menu?.kind === "highlight" ? (
        <Popover
          label="Highlight cells"
          testId="highlight-menu"
          x={menu.x}
          y={menu.y}
          onClose={() => setMenu(null)}
          items={[
            ...HIGHLIGHT_PRESETS.map((p) => ({
              id: p.id,
              label: p.label,
              separator: p.separator ?? false,
              onSelect: () => {
                ctl.v3.highlight(p.id);
                if (p.values === 0) done();
              },
            })),
            {
              id: "manage",
              label: "Manage rules…",
              separator: true,
              onSelect: () => useUiStore.getState().openDialog({ kind: "cfManager" }),
            },
          ]}
        />
      ) : null}
      {menu?.kind === "borders" ? (
        <BordersPicker
          x={menu.x}
          y={menu.y}
          onClose={() => setMenu(null)}
          onPick={(preset, line) => {
            void ctl.applyBorders(preset, line);
            done();
          }}
        />
      ) : null}
      {menu?.kind === "color" || menu?.kind === "fill" ? (
        <ColorPicker
          x={menu.x}
          y={menu.y}
          label={menu.kind === "color" ? "Font colour" : "Fill colour"}
          value={menu.kind === "color" ? (style?.color ?? null) : (style?.fill ?? null)}
          allowNone={menu.kind === "fill"}
          onClose={() => setMenu(null)}
          onPick={(rgba) => {
            void (menu.kind === "color" ? ctl.setTextColor(rgba) : ctl.setFillColor(rgba));
            done();
          }}
        />
      ) : null}
      {barMenu.menu}
    </div>
  );
}
