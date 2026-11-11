// `node --test .github/release/` (CI web job): the release helpers.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { changelogSection, latestJson, plan, REQUIRED } from "./release.mjs";

const SCRIPT = join(dirname(fileURLToPath(import.meta.url)), "release.mjs");

const CHANGELOG = `# Changelog

## [Unreleased]

- Work in progress

## [0.2.0] - 2026-12-01

### Added
- Auto-update.

## 0.1.10

- Older.
`;

test("the CHANGELOG section of a version, or nothing", () => {
  assert.equal(changelogSection(CHANGELOG, "0.2.0"), "### Added\n- Auto-update.");
  assert.equal(changelogSection(CHANGELOG, "0.1.10"), "- Older.");
  assert.equal(changelogSection(CHANGELOG, "0.1.1"), "");
  assert.equal(changelogSection(CHANGELOG, "0.2"), "");
  assert.equal(changelogSection("", "0.2.0"), "");
});

const BUNDLES = {
  "darwin-aarch64": ["b/macos/Tally.app.tar.gz", "b/macos/Tally.app.tar.gz.sig", "b/dmg/Tally_0.2.0_aarch64.dmg"],
  "darwin-x86_64": ["b/macos/Tally.app.tar.gz", "b/macos/Tally.app.tar.gz.sig", "b/dmg/Tally_0.2.0_x64.dmg"],
  "windows-x86_64": ["b/nsis/Tally_0.2.0_x64-setup.exe", "b/nsis/Tally_0.2.0_x64-setup.exe.sig", "b/msi/Tally_0.2.0_x64_en-US.msi", "b/msi/Tally_0.2.0_x64_en-US.msi.sig"],
  "linux-x86_64": ["b/appimage/Tally_0.2.0_amd64.AppImage", "b/appimage/Tally_0.2.0_amd64.AppImage.sig", "b/deb/Tally_0.2.0_amd64.deb", "b/rpm/Tally-0.2.0-1.x86_64.rpm"],
};

test("each platform publishes fixed names; the two macOS updater packages no longer collide", () => {
  const arm = plan("darwin-aarch64", "0.2.0", BUNDLES["darwin-aarch64"]);
  const intel = plan("darwin-x86_64", "0.2.0", BUNDLES["darwin-x86_64"]);
  assert.deepEqual(arm.copies.map((c) => c.name), ["Tally_0.2.0_aarch64.dmg", "Tally_0.2.0_aarch64.app.tar.gz", "Tally_0.2.0_aarch64.app.tar.gz.sig"]);
  assert.equal(intel.updater["darwin-x86_64"].name, "Tally_0.2.0_x64.app.tar.gz");
  const win = plan("windows-x86_64", "0.2.0", BUNDLES["windows-x86_64"]);
  assert.equal(win.updater["windows-x86_64"].name, "Tally_0.2.0_x64-setup.exe");
  assert.equal(win.updater["windows-x86_64-msi"].name, "Tally_0.2.0_x64_en-US.msi");
  const linux = plan("linux-x86_64", "0.2.0", BUNDLES["linux-x86_64"]);
  assert.deepEqual(Object.keys(linux.updater).sort(), ["linux-x86_64", "linux-x86_64-appimage"]);
  // .deb and .rpm are downloads only.
  assert.ok(linux.copies.some((c) => c.name.endsWith(".deb")) && !Object.values(linux.updater).some((e) => e.name.endsWith(".deb")));
});

test("an unsigned or ambiguous bundle fails the job", () => {
  assert.throws(() => plan("darwin-aarch64", "0.2.0", ["b/macos/Tally.app.tar.gz", "b/dmg/T.dmg"]), /no signature/);
  assert.throws(() => plan("darwin-aarch64", "0.2.0", [...BUNDLES["darwin-aarch64"], "b/dmg/other.dmg"]), /expected one macOS \.dmg, found 2/);
  assert.throws(() => plan("linux-x86_64", "0.2.0", ["b/appimage/T.AppImage", "b/appimage/T.AppImage.sig"]), /\.deb/);
  assert.throws(() => plan("freebsd-x86_64", "0.2.0", []), /unknown platform/);
});

const entries = Object.fromEntries(REQUIRED.map((p) => [p, { asset: `Tally_${p}.bin`, signature: `sig-${p}\n` }]));

test("latest.json covers every platform with release-asset URLs", () => {
  const doc = latestJson({ version: "0.2.0", tag: "v0.2.0", repo: "shane9b3/tally", notes: "Notes", pubDate: "2026-12-01T00:00:00Z", entries });
  assert.equal(doc.version, "0.2.0");
  assert.equal(doc.notes, "Notes");
  assert.deepEqual(Object.keys(doc.platforms), [...REQUIRED].sort());
  assert.equal(doc.platforms["darwin-aarch64"].url, "https://github.com/shane9b3/tally/releases/download/v0.2.0/Tally_darwin-aarch64.bin");
  assert.equal(doc.platforms["linux-x86_64"].signature, "sig-linux-x86_64");
  const { "windows-x86_64": _, ...partial } = entries;
  assert.throws(() => latestJson({ version: "0.2.0", tag: "v0.2.0", repo: "shane9b3/tally", notes: "", pubDate: "", entries: partial }), /miss windows-x86_64/);
  assert.throws(() => latestJson({ version: "v0.2.0", tag: "v0.2.0", repo: "shane9b3/tally", notes: "", pubDate: "", entries }), /not a version/);
  assert.throws(() => latestJson({ version: "0.2.0", tag: "v0.2.0", repo: "shane9b3/tally", notes: "", pubDate: "", entries: { ...entries, "linux-x86_64": { asset: "a", signature: " " } } }), /empty signature/);
});

test("the CLI: collect then latest", () => {
  const root = mkdtempSync(join(tmpdir(), "tally-release-"));
  try {
    for (const [platform, files] of Object.entries(BUNDLES)) {
      const bundle = join(root, platform);
      for (const f of files) {
        const p = join(bundle, f);
        mkdirSync(dirname(p), { recursive: true });
        writeFileSync(p, f.endsWith(".sig") ? `signature of ${platform}` : "bytes");
      }
      execFileSync(process.execPath, [SCRIPT, "collect", "--platform", platform, "--version", "0.2.0", "--bundle-dir", join(bundle, "b"), "--out", join(root, "out")]);
    }
    writeFileSync(join(root, "CHANGELOG.md"), CHANGELOG);
    execFileSync(process.execPath, [SCRIPT, "notes", "--version", "0.2.0", "--changelog", join(root, "CHANGELOG.md"), "--out", join(root, "notes.md")]);
    execFileSync(process.execPath, [SCRIPT, "latest", "--version", "0.2.0", "--tag", "v0.2.0", "--repo", "shane9b3/tally", "--notes", join(root, "notes.md"), "--fragments", join(root, "out"), "--out", join(root, "latest.json")]);
    const doc = JSON.parse(readFileSync(join(root, "latest.json"), "utf8"));
    assert.equal(doc.notes, "### Added\n- Auto-update.");
    assert.equal(doc.platforms["darwin-x86_64"].signature, "signature of darwin-x86_64");
    assert.equal(doc.platforms["windows-x86_64-msi"].url, "https://github.com/shane9b3/tally/releases/download/v0.2.0/Tally_0.2.0_x64_en-US.msi");
    assert.match(doc.pub_date, /^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$/);
    assert.equal(readFileSync(join(root, "out", "Tally_0.2.0_aarch64.app.tar.gz"), "utf8"), "bytes");
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
