/** No user-visible English literal bypasses t() (scripts/scan-literals.mjs; allowlist inside). */
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { expect, it } from "vitest";

it("scan-literals finds no untranslated JSX text or string props", () => {
  const script = fileURLToPath(new URL("../scripts/scan-literals.mjs", import.meta.url));
  const r = spawnSync(process.execPath, [script], { encoding: "utf8" });
  expect(r.stdout.trim().split("\n").slice(0, -1)).toEqual([]);
});
