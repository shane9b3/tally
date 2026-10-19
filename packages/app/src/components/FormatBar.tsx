/**
 * The formatting strip under the toolbar: B I U S, font size, font and
 * fill colour, alignment, wrap, number format, $ / %, and "Colour by
 * value". Every toggle reflects the active cell's style; every action
 * applies to all selected ranges.
 */
import type { HAlign } from "@tally/bridge";
import { useState, type ReactNode } from "react";
import { cx, useController } from "../context";
import { FONT_SIZES, NUMBER_FORMATS, presetFor } from "../formats";
import { useActiveStyle } from "../hooks";
import { rgbaToHex } from "../palette";
import { useUiStore } from "../stores/uiStore";
import { ColorPicker } from "./ColorPicker";
import { AlignCenterIcon, AlignLeftIcon, AlignRightIcon, ChevronIcon, ChipsIcon, FillIcon, WrapIcon } from "./icons";
import { Popover } from "./Popover";
import styles from "./FormatBar.module.css";

interface ToolProps {
  command: string;
  label: string;
  children: ReactNode;
  active?: boolean;
  wide?: boolean;
}

/** An icon button bound to a CommandRegistry command. */
function Tool({ command, label, children, active, wide }: ToolProps) {
  const ctl = useController();
  const cmd = ctl.registry.get(command);
  const tip = [label, cmd?.shortcut].filter(Boolean).join("  ");
  return (
    <button
      type="button"
      className={cx(styles.tool, wide && styles.wide, active && styles.on)}
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

type Menu = { kind: "size" | "format" | "color" | "fill"; x: number; y: number } | null;

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
  const size = style?.fontSizePt ?? 10;
  const textColor = style?.color != null ? rgbaToHex(style.color) : "var(--tally-text)";
  const fillColor = style?.fill != null && (style.fill & 255) > 0 ? rgbaToHex(style.fill) : "transparent";
  const done = () => ctl.focusGrid();

  return (
    <div className={styles.bar} role="toolbar" aria-label="Format" data-testid="format-bar">
      <div className={styles.group}>
        <Tool command="format.bold" label="Bold" active={style?.bold ?? false}>
          <span className={styles.bold}>B</span>
        </Tool>
        <Tool command="format.italic" label="Italic" active={style?.italic ?? false}>
          <span className={styles.italic}>I</span>
        </Tool>
        <Tool command="format.underline" label="Underline" active={style?.underline ?? false}>
          <span className={styles.underline}>U</span>
        </Tool>
        <Tool command="format.strike" label="Strikethrough" active={style?.strike ?? false}>
          <span className={styles.strike}>S</span>
        </Tool>
      </div>
      <span className={styles.sep} />
      <button
        type="button"
        className={cx(styles.dropdown, styles.size)}
        aria-label="Font size"
        aria-haspopup="menu"
        title="Font size"
        data-testid="font-size"
        onMouseDown={(e) => e.preventDefault()}
        onClick={open("size")}
      >
        <span>{size}</span>
        <ChevronIcon />
      </button>
      <span className={styles.sep} />
      <div className={styles.group}>
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
      </div>
      <span className={styles.sep} />
      <div className={styles.group}>
        <Tool command="format.alignLeft" label="Align left" active={h === "left"}>
          <AlignLeftIcon />
        </Tool>
        <Tool command="format.alignCenter" label="Align centre" active={h === "center"}>
          <AlignCenterIcon />
        </Tool>
        <Tool command="format.alignRight" label="Align right" active={h === "right"}>
          <AlignRightIcon />
        </Tool>
        <Tool command="format.wrap" label="Wrap text" active={style?.wrap ?? false}>
          <WrapIcon />
        </Tool>
      </div>
      <span className={styles.sep} />
      <button
        type="button"
        className={cx(styles.dropdown, styles.format)}
        aria-label="Number format"
        aria-haspopup="menu"
        title="Number format"
        data-testid="number-format"
        onMouseDown={(e) => e.preventDefault()}
        onClick={open("format")}
      >
        <span>{formatLabel}</span>
        <ChevronIcon />
      </button>
      <div className={styles.group}>
        <Tool command="format.currency" label="Format as currency" active={preset?.id === "currency"}>
          <span className={styles.glyph}>$</span>
        </Tool>
        <Tool command="format.percent" label="Format as percent" active={preset?.id === "percent"}>
          <span className={styles.glyph}>%</span>
        </Tool>
      </div>
      <span className={styles.sep} />
      <Tool command="format.colorByValue" label="Colour by value: chip colours for each distinct value in the column" wide>
        <ChipsIcon />
        <span>Colour by value</span>
      </Tool>

      {menu?.kind === "size" ? (
        <Popover
          label="Font size"
          x={menu.x}
          y={menu.y}
          onClose={() => setMenu(null)}
          items={FONT_SIZES.map((pt) => ({
            id: String(pt),
            label: String(pt),
            checked: pt === size,
            onSelect: () => {
              void ctl.setFontSize(pt);
              done();
            },
          }))}
        />
      ) : null}
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
    </div>
  );
}
