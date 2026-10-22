// @vitest-environment happy-dom
/**
 * Page setup ▸ Header & footer (AOO Format ▸ Page ▸ Header / Footer):
 * "Header on" / "Footer on" keep the texts and disable the other
 * controls, Spacing / Height / AutoFit height / side margins map onto the
 * page setup (Height is shown without the spacing, as in AOO), the
 * preview lays the areas out like the engine, and the TS layout mirror.
 */
import type { PageSetupDto } from "@tally/bridge";
import { defaultPageSetup } from "@tally/bridge/mock";
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { V3Dialog } from "../src/components/V3Dialogs";
import { ControllerContext } from "../src/context";
import { AppController } from "../src/controller";
import { WebMockClient } from "../src/dev/webMock";
import { COL_HEADER_H_MM, HF_LINE_MM, hfArea, hfLines, MIN_BODY_MM, pageBoxes, ROW_HEADER_W_MM } from "../src/print/hfLayout";
import { autoPageBreaks } from "../src/v03";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const TWO_LINES = { left: "", center: "The Redwood - 10/22/2026\nBlock 90 - North Side", right: "" };

describe("header/footer layout mirror", () => {
  it("autofits two lines plus spacing and pushes the body down", () => {
    const s: PageSetupDto = { ...defaultPageSetup(), header: TWO_LINES, footer: null, margins: { ...defaultPageSetup().margins, topMm: 10 } };
    expect(hfLines(TWO_LINES)).toBe(2);
    const b = pageBoxes(s, [210, 297]);
    expect(b.header).toEqual({ top: 10, height: 2 * HF_LINE_MM, left: 20, width: 170 });
    expect(b.bodyTop).toBeCloseTo(10 + 2 * HF_LINE_MM + 2.5, 6);
    expect(b.header!.top + b.header!.height).toBeLessThan(b.bodyTop);
    expect(b.footer).toBeNull();
    expect(b.bodyHeight).toBeCloseTo(297 - 10 - 20 - (2 * HF_LINE_MM + 2.5), 6);
  });

  it("keeps a one-line area at its set height; off gives the room back; footer ends at the bottom margin", () => {
    const s = defaultPageSetup();
    const b = pageBoxes(s, [210, 297]);
    expect(b.bodyTop).toBe(27.5);
    expect(b.header!.height).toBe(5);
    expect(b.footer!.top + b.footer!.height).toBeCloseTo(277, 6);
    expect(b.bodyHeight).toBeCloseTo(297 - 40 - 15, 6);
    const off = pageBoxes({ ...s, headerArea: { on: false, spacingMm: 2.5, autofit: true, leftMm: 0, rightMm: 0 } }, [210, 297]);
    expect(off.header).toBeNull();
    expect(off.bodyTop).toBe(20);
  });

  it("estimated page breaks use the reduced body", () => {
    const s: PageSetupDto = { ...defaultPageSetup(), header: TWO_LINES, footer: null, margins: { ...defaultPageSetup().margins, topMm: 10 } };
    const rows = { runs: [], dflt: 20 };
    const body = pageBoxes(s, [210, 297]).bodyHeight * (96 / 25.4);
    const auto = autoPageBreaks(s, { r0: 0, c0: 0, r1: 499, c1: 0 }, rows, { runs: [], dflt: 80 });
    expect(auto.rows[0]).toBe(Math.floor(body / 20));
  });
});

describe("clamping and page-break estimates (as the engine)", () => {
  it("oversized areas keep a 10 mm body and are reported", () => {
    const s: PageSetupDto = { ...defaultPageSetup(), margins: { ...defaultPageSetup().margins, headerMm: 300, footerMm: 200 } };
    const b = pageBoxes(s, [210, 297]);
    expect(b.fits).toBe(false);
    expect(b.bodyHeight).toBe(MIN_BODY_MM);
    expect(b.bodyTop).toBe(20 + 247);
    expect(b.footer!.height).toBe(0);
    expect(pageBoxes(defaultPageSetup(), [210, 297]).fits).toBe(true);
  });

  it("printed headers and repeated rows take room from the estimated pages", () => {
    const base: PageSetupDto = { ...defaultPageSetup(), header: null, footer: null };
    const rows = { runs: [], dflt: 20 };
    const cols = { runs: [], dflt: 80 };
    const area = { r0: 0, c0: 0, r1: 499, c1: 30 };
    const bodyPx = 257 * (96 / 25.4);
    expect(autoPageBreaks(base, area, rows, cols).rows[0]).toBe(Math.floor(bodyPx / 20));
    const withHeaders = autoPageBreaks({ ...base, printHeaders: true }, area, rows, cols);
    expect(withHeaders.rows[0]).toBe(Math.floor((bodyPx - COL_HEADER_H_MM * (96 / 25.4)) / 20));
    expect(withHeaders.cols[0]).toBe(Math.floor(((170 - ROW_HEADER_W_MM) * (96 / 25.4)) / 80));
    // Rows 1–2 repeated: page 1 as before, later pages hold 2 rows fewer.
    const rep = autoPageBreaks({ ...base, repeatRows: [0, 1] }, area, rows, cols).rows;
    const per = Math.floor(bodyPx / 20);
    expect(rep[0]).toBe(per);
    expect(rep[1]! - rep[0]!).toBe(per - 2);
  });
});

describe("Page setup ▸ Header & footer", () => {
  let host: HTMLDivElement;
  let root: Root;
  let ctl: AppController;
  let save: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
    ctl = new AppController(new WebMockClient(), new WebMockClient(), { nativeMenu: false });
    save = vi.fn(async () => {});
    (ctl.v3 as unknown as { savePageSetup: typeof save }).savePageSetup = save;
  });

  afterEach(() => {
    act(() => root.unmount());
    host.remove();
  });

  const q = <T extends HTMLElement>(id: string) => host.querySelector<T>(`[data-testid="${id}"]`);
  const open = (setup: PageSetupDto) => {
    act(() => root.render(createElement(ControllerContext.Provider, { value: ctl }, createElement(V3Dialog, { dialog: { kind: "pageSetup", setup } }))));
    act(() => q("tab-hf")!.click());
  };
  const type = (id: string, value: string) => {
    const el = q<HTMLInputElement | HTMLTextAreaElement>(id)!;
    const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    const setter = Object.getOwnPropertyDescriptor(proto, "value")!.set!;
    act(() => {
      setter.call(el, value);
      el.dispatchEvent(new Event("input", { bubbles: true }));
    });
  };
  const submit = (): PageSetupDto => {
    act(() => q("dialog-ok")!.click());
    expect(save).toHaveBeenCalledTimes(1);
    return save.mock.calls[0]![0] as PageSetupDto;
  };

  it("shows AOO's controls; Header on unchecked disables them and keeps the text", () => {
    open(defaultPageSetup());
    for (const id of ["header-on", "header-left-margin", "header-right-margin", "header-spacing", "header-height", "header-autofit", "footer-on", "footer-spacing", "footer-height", "footer-autofit"]) expect(q(id), id).not.toBeNull();
    expect(host.textContent).toContain("Header on");
    expect(host.textContent).toContain("Footer on");
    expect(host.textContent).toContain("AutoFit height");
    expect(q<HTMLInputElement>("header-on")!.checked).toBe(true);
    expect(q<HTMLInputElement>("header-autofit")!.checked).toBe(true);
    // Height is shown without the spacing: 7.5 − 2.5 mm.
    expect(q<HTMLInputElement>("header-height")!.value).toBe("5");
    expect(q<HTMLInputElement>("header-spacing")!.value).toBe("2.5");
    expect(q<HTMLFieldSetElement>("header-fields")!.disabled).toBe(false);
    expect(q("preview-header")).not.toBeNull();
    act(() => q("header-on")!.click());
    expect(q<HTMLInputElement>("header-on")!.checked).toBe(false);
    expect(q<HTMLFieldSetElement>("header-fields")!.disabled).toBe(true);
    expect(q<HTMLFieldSetElement>("footer-fields")!.disabled).toBe(false);
    expect(q("preview-header")).toBeNull();
    // The text stays in the (disabled) field.
    expect(q<HTMLTextAreaElement>("header-center")!.value).toBe("&[SHEET]");
    const out = submit();
    expect(out.headerArea).toMatchObject({ on: false, spacingMm: 2.5, autofit: true });
    expect(out.header).toEqual({ left: "", center: "&[SHEET]", right: "" });
    // The footer was left alone (an older setup without areas: on).
    expect(hfArea(out, "footer").on).toBe(true);
  });

  it("Spacing keeps the height, Height adds the spacing; AutoFit and margins are saved in one change", () => {
    open(defaultPageSetup());
    type("footer-spacing", "5");
    expect(q<HTMLInputElement>("footer-height")!.value).toBe("5");
    type("footer-height", "8");
    act(() => q("footer-autofit")!.click());
    type("footer-left-margin", "3");
    type("footer-right-margin", "1.5");
    const out = submit();
    expect(out.margins.footerMm).toBe(13);
    expect(out.footerArea).toEqual({ on: true, spacingMm: 5, autofit: false, leftMm: 3, rightMm: 1.5 });
    expect(out.margins.headerMm).toBe(7.5);
  });

  it("Footer on without a text gives an empty footer; clearing a text keeps the footer on", () => {
    open({ ...defaultPageSetup(), footer: null });
    expect(q<HTMLInputElement>("footer-on")!.checked).toBe(false);
    expect(q<HTMLFieldSetElement>("footer-fields")!.disabled).toBe(true);
    act(() => q("footer-on")!.click());
    expect(q<HTMLFieldSetElement>("footer-fields")!.disabled).toBe(false);
    type("header-center", "");
    // On and empty: it still takes its room, and the dialog says so.
    expect(q("header-empty-note")!.textContent).toContain("7.5 mm");
    const out = submit();
    expect(out.footer).toEqual({ left: "", center: "", right: "" });
    expect(out.footerArea?.on).toBe(true);
    expect(out.header).toEqual({ left: "", center: "", right: "" });
  });

  it("the preview places a two-line header above the body; a region keeps its line breaks", () => {
    open({ ...defaultPageSetup(), header: TWO_LINES, margins: { ...defaultPageSetup().margins, topMm: 10 } });
    expect(q<HTMLTextAreaElement>("header-center")!.value).toBe(TWO_LINES.center);
    type("header-right", "a\nb");
    expect(q<HTMLTextAreaElement>("header-right")!.value).toBe("a\nb");
    const head = q("preview-header")!;
    const body = q("preview-body")!;
    const px = (v: string) => Number.parseFloat(v);
    expect(px(head.style.top) + px(head.style.height)).toBeLessThan(px(body.style.top));
    expect(px(head.style.top)).toBeGreaterThan(0);
  });

  it("refuses areas that do not fit the page", () => {
    open(defaultPageSetup());
    expect(q("page-hf-too-tall")).toBeNull();
    type("header-height", "200");
    expect(q("page-hf-too-tall")).toBeNull();
    type("footer-height", "50");
    expect(q("page-hf-too-tall")).not.toBeNull();
    expect(q<HTMLButtonElement>("dialog-ok")!.disabled).toBe(true);
    type("header-height", "20");
    expect(q("page-hf-too-tall")).toBeNull();
    expect(q<HTMLButtonElement>("dialog-ok")!.disabled).toBe(false);
  });
});
