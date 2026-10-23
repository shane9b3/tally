// @vitest-environment happy-dom
/**
 * `arrowNav`: selection follows the focus only in single-select widgets whose
 * click merely selects; multi-select lists, `data-select-on-focus="false"`
 * groups and `select: false` callers only move the focus; a popup inside a
 * toolbar never gets the toolbar's navigation.
 */
import { afterEach, describe, expect, it } from "vitest";
import { arrowNav } from "../src/components/dialogA11y";

// happy-dom has no layout: give every element a box so the visibility filters keep it.
HTMLElement.prototype.getClientRects = function () {
  return [{ width: 1, height: 1 }] as unknown as DOMRectList;
};

function press(el: HTMLElement, key: string, opts: { select?: boolean } = {}): boolean {
  let prevented = false;
  return (
    arrowNav(
      { key, target: el, altKey: false, metaKey: false, ctrlKey: false, defaultPrevented: false, preventDefault: () => void (prevented = true), stopPropagation: () => undefined },
      opts,
    ) && prevented
  );
}

function build(html: string): { root: HTMLElement; clicks: string[] } {
  const root = document.createElement("div");
  root.innerHTML = html;
  document.body.appendChild(root);
  const clicks: string[] = [];
  root.querySelectorAll<HTMLElement>("button").forEach((b) => b.addEventListener("click", () => clicks.push(b.id)));
  return { root, clicks };
}

afterEach(() => {
  document.body.innerHTML = "";
});

describe("arrowNav", () => {
  it("single-select list: the selection follows the focus", () => {
    const { root, clicks } = build('<div role="listbox"><button id="a" role="option"></button><button id="b" role="option"></button></div>');
    expect(press(root.querySelector("#a")!, "ArrowDown")).toBe(true);
    expect(document.activeElement?.id).toBe("b");
    expect(clicks).toEqual(["b"]);
  });

  it("multi-select list (Show Sheet): arrows only move the focus", () => {
    const { root, clicks } = build('<div role="listbox" aria-multiselectable="true"><button id="a" role="option"></button><button id="b" role="option"></button><button id="c" role="option"></button></div>');
    press(root.querySelector("#a")!, "ArrowDown");
    press(root.querySelector("#b")!, "End");
    expect(document.activeElement?.id).toBe("c");
    expect(clicks).toEqual([]);
  });

  it('data-select-on-focus="false" radio group (chart type): arrows only move the focus', () => {
    const { root, clicks } = build('<div role="radiogroup" data-select-on-focus="false"><button id="a" role="radio"></button><button id="b" role="radio"></button></div>');
    press(root.querySelector("#a")!, "ArrowRight");
    expect(document.activeElement?.id).toBe("b");
    expect(clicks).toEqual([]);
    // A plain radio group still selects.
    const other = build('<div role="radiogroup"><button id="x" role="radio"></button><button id="y" role="radio"></button></div>');
    press(other.root.querySelector("#x")!, "ArrowRight");
    expect(other.clicks).toEqual(["y"]);
  });

  it("select: false (Styles panel) never clicks", () => {
    const { root, clicks } = build('<div role="listbox"><button id="a" role="option"></button><button id="b" role="option"></button></div>');
    press(root.querySelector("#a")!, "ArrowDown", { select: false });
    expect(clicks).toEqual([]);
  });

  it("a picker (role=dialog) inside a toolbar does not get the toolbar's arrows", () => {
    const { root } = build('<div role="toolbar"><button id="t1"></button><button id="t2"></button><div role="dialog"><button id="p1"></button><button id="p2"></button></div></div>');
    expect(press(root.querySelector("#p1")!, "ArrowRight")).toBe(false);
    // The toolbar's own buttons skip the picker's.
    root.querySelector<HTMLElement>("#t2")!.focus();
    press(root.querySelector("#t2")!, "ArrowRight");
    expect(document.activeElement?.id).toBe("t1");
  });
});
