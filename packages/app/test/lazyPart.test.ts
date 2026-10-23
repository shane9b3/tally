// @vitest-environment happy-dom
/**
 * `lazyPart` / `LazySlot`: a code-split part whose import() fails shows a
 * toast, runs `onFail` and leaves its siblings alone; its next mount loads it
 * again instead of replaying the cached rejection.
 */
import { act, createElement, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { LazySlot, lazyPart } from "../src/lazyPart";
import { useUiStore } from "../src/stores/uiStore";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const Hello = ({ who }: { who: string }) => createElement("p", { "data-testid": "part" }, `hello ${who}`);

async function flush(): Promise<void> {
  for (let i = 0; i < 5; i++) await act(async () => new Promise((r) => setTimeout(r, 0)));
}

describe("lazyPart + LazySlot", () => {
  afterEach(() => {
    useUiStore.getState().reset();
    vi.restoreAllMocks();
  });

  it("a failed load toasts, calls onFail, keeps the siblings, and the next mount retries", async () => {
    vi.spyOn(console, "error").mockImplementation(() => undefined);
    let attempts = 0;
    const load = vi.fn(async () => {
      attempts++;
      if (attempts === 1) throw new Error("chunk fetch failed");
      return Hello;
    });
    const Part = lazyPart(load);
    const onFail = vi.fn();
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root = createRoot(host);
    const view = (open: boolean): ReactNode =>
      createElement("div", null, createElement("span", { "data-testid": "grid" }, "grid"), open ? createElement(LazySlot, { onFail }, createElement(Part, { who: "tally" })) : null);

    await act(async () => root.render(view(true)));
    await flush();
    expect(onFail).toHaveBeenCalledTimes(1);
    expect(host.querySelector("[data-testid=grid]")).not.toBeNull();
    expect(host.querySelector("[data-testid=part]")).toBeNull();
    const toasts = useUiStore.getState().toasts;
    expect(toasts.some((x) => x.kind === "error")).toBe(true);
    expect(load).toHaveBeenCalledTimes(1);

    // Closed (onFail reset the flag), then opened again: a fresh load, which works.
    await act(async () => root.render(view(false)));
    await act(async () => root.render(view(true)));
    await flush();
    expect(load).toHaveBeenCalledTimes(2);
    expect(host.querySelector("[data-testid=part]")?.textContent).toBe("hello tally");
    expect(onFail).toHaveBeenCalledTimes(1);

    // Loaded once: later mounts reuse it.
    await act(async () => root.render(view(false)));
    await act(async () => root.render(view(true)));
    expect(host.querySelector("[data-testid=part]")?.textContent).toBe("hello tally");
    expect(load).toHaveBeenCalledTimes(2);
    await act(async () => root.unmount());
    host.remove();
  });
});
