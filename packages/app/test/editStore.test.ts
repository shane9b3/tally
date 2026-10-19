import { beforeEach, describe, expect, it } from "vitest";
import { useEditStore } from "../src/stores/editStore";

const st = () => useEditStore.getState();

describe("EditSession history", () => {
  beforeEach(() => st().reset());

  it("coalesces typing into words and undoes/redoes editor-locally", () => {
    st().start({ r: 0, c: 0 }, "", "grid");
    for (const t of ["=", "=S", "=SU", "=SUM", "=SUM("]) st().change(t);
    st().change("=SUM(A");
    expect(st().undo()).toBe(true);
    expect(st().edit?.text).toBe("=SUM(");
    st().undo();
    expect(st().edit?.text).toBe("=SUM");
    st().undo();
    expect(st().edit?.text).toBe("=");
    expect(st().redo()).toBe(true);
    expect(st().edit?.text).toBe("=SUM");
    // Each programmatic change bumps seq (the grid editor then applies it).
    expect(st().edit?.seq).toBeGreaterThan(0);
  });

  it("programmatic replacements are their own step and remember the point span", () => {
    st().start({ r: 0, c: 0 }, "=", "grid");
    st().replace("=A1", 3, { start: 1, end: 3 });
    expect(st().pointSpan).toEqual({ start: 1, end: 3 });
    st().change("=A1+");
    expect(st().pointSpan).toBeNull();
    st().undo();
    expect(st().edit?.text).toBe("=A1");
    st().undo();
    expect(st().edit?.text).toBe("=");
    expect(st().undo()).toBe(false);
  });
});
