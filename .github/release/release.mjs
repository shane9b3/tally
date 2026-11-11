#!/usr/bin/env node
// Release helpers for .github/workflows/release.yml (docs/release.md).
// Node only, no dependencies. Tests: `node --test .github/release/`.
//
//   notes   --version X --changelog CHANGELOG.md --out notes.md
//       The CHANGELOG.md section of version X ("## [X]" or "## X", up to the
//       next "## "), or an empty file when there is none (the workflow then
//       uses GitHub's generated notes).
//   collect --platform darwin-aarch64 --version X --bundle-dir DIR --out DIR
//       Copies this platform's installers and updater package (+ .sig) from
//       the Tauri bundle directory into --out under fixed names, and writes
//       fragment-<platform>.json: the updater entries for latest.json.
//   latest  --version X --tag vX --repo owner/name --notes notes.md --fragments DIR --out latest.json
//       Merges the fragments into the updater manifest; fails unless every
//       platform in REQUIRED is present.

import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { basename, join } from "node:path";
import { fileURLToPath } from "node:url";

/** The platforms every release must cover (updater keys). */
export const REQUIRED = ["darwin-aarch64", "darwin-x86_64", "windows-x86_64", "linux-x86_64"];

const VERSION_RE = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/;

/** The CHANGELOG.md section for `version` (trimmed), or "". */
export function changelogSection(text, version) {
  const lines = text.split(/\r?\n/);
  const esc = version.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const head = new RegExp(`^##\\s+\\[?v?${esc}\\]?(?:\\s|$)`);
  const start = lines.findIndex((l) => head.test(l));
  if (start < 0) return "";
  let end = lines.length;
  for (let i = start + 1; i < lines.length; i++) {
    if (/^##\s/.test(lines[i])) {
      end = i;
      break;
    }
  }
  return lines
    .slice(start + 1, end)
    .join("\n")
    .trim();
}

function walk(dir) {
  if (!existsSync(dir)) return [];
  const out = [];
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    const st = statSync(p);
    // `Tally.app` is a directory: never descend into bundles.
    if (st.isDirectory() && !name.endsWith(".app")) out.push(...walk(p));
    else if (st.isFile()) out.push(p);
  }
  return out;
}

/** The single file in `files` matching `re` (error if none or several). */
function one(files, re, what) {
  const hits = files.filter((f) => re.test(basename(f)));
  if (hits.length !== 1) throw new Error(`expected one ${what}, found ${hits.length}: ${hits.map((h) => basename(h)).join(", ") || "none"}`);
  return hits[0];
}

/**
 * What to publish for `platform` from the bundle directory's `files`:
 * `[{ from, name }]` copies and the updater entries `{ key: { from, name } }`
 * (the `.sig` next to `from` holds the signature).
 */
export function plan(platform, version, files) {
  const copies = [];
  const updater = {};
  const add = (from, name) => {
    copies.push({ from, name });
    return { from, name };
  };
  const withSig = (from, name, keys) => {
    const sig = `${from}.sig`;
    if (!files.includes(sig)) throw new Error(`no signature next to ${basename(from)} (is TAURI_SIGNING_PRIVATE_KEY set?)`);
    const entry = add(from, name);
    add(sig, `${name}.sig`);
    for (const k of keys) updater[k] = entry;
  };
  switch (platform) {
    case "darwin-aarch64":
    case "darwin-x86_64": {
      const arch = platform === "darwin-aarch64" ? "aarch64" : "x64";
      add(one(files, /\.dmg$/, "macOS .dmg"), `Tally_${version}_${arch}.dmg`);
      // Tauri names it `Tally.app.tar.gz` for both architectures: renamed here.
      withSig(one(files, /\.app\.tar\.gz$/, "macOS updater .app.tar.gz"), `Tally_${version}_${arch}.app.tar.gz`, [platform, `${platform}-app`]);
      break;
    }
    case "windows-x86_64": {
      const nsis = one(files, /-setup\.exe$/, "NSIS installer");
      withSig(nsis, `Tally_${version}_x64-setup.exe`, ["windows-x86_64", "windows-x86_64-nsis"]);
      const msi = one(files, /\.msi$/, "MSI installer");
      withSig(msi, `Tally_${version}_x64_en-US.msi`, ["windows-x86_64-msi"]);
      break;
    }
    case "linux-x86_64": {
      // The AppImage updates itself; .deb and .rpm are plain downloads (the
      // app does not offer updates when installed from them, updater.rs).
      withSig(one(files, /\.AppImage$/, "AppImage"), `Tally_${version}_amd64.AppImage`, ["linux-x86_64", "linux-x86_64-appimage"]);
      add(one(files, /\.deb$/, ".deb package"), `Tally_${version}_amd64.deb`);
      add(one(files, /\.rpm$/, ".rpm package"), `Tally-${version}-1.x86_64.rpm`);
      break;
    }
    default:
      throw new Error(`unknown platform ${platform}`);
  }
  return { copies, updater };
}

/** Builds latest.json from the fragments' entries. */
export function latestJson({ version, tag, repo, notes, pubDate, entries }) {
  if (!VERSION_RE.test(version)) throw new Error(`not a version: ${version}`);
  if (!/^[\w.-]+\/[\w.-]+$/.test(repo)) throw new Error(`not a repository: ${repo}`);
  const missing = REQUIRED.filter((p) => !entries[p]);
  if (missing.length > 0) throw new Error(`latest.json would miss ${missing.join(", ")}`);
  const platforms = {};
  for (const key of Object.keys(entries).sort()) {
    const { asset, signature } = entries[key];
    if (typeof signature !== "string" || signature.trim() === "") throw new Error(`${key}: empty signature`);
    platforms[key] = {
      signature: signature.trim(),
      url: `https://github.com/${repo}/releases/download/${encodeURIComponent(tag)}/${encodeURIComponent(asset)}`,
    };
  }
  return { version, notes, pub_date: pubDate, platforms };
}

function args(argv) {
  const out = {};
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (!a.startsWith("--")) throw new Error(`unexpected argument ${a}`);
    const v = argv[i + 1];
    if (v === undefined) throw new Error(`${a} needs a value`);
    out[a.slice(2)] = v;
    i++;
  }
  return out;
}

function need(o, ...keys) {
  for (const k of keys) if (o[k] === undefined) throw new Error(`--${k} is required`);
}

function main(argv) {
  const [cmd, ...rest] = argv;
  const o = args(rest);
  if (cmd === "notes") {
    need(o, "version", "changelog", "out");
    const text = existsSync(o.changelog) ? readFileSync(o.changelog, "utf8") : "";
    const section = changelogSection(text, o.version);
    writeFileSync(o.out, section ? `${section}\n` : "");
    console.log(section ? `notes: CHANGELOG.md section for ${o.version}` : `notes: no CHANGELOG.md section for ${o.version}`);
  } else if (cmd === "collect") {
    need(o, "platform", "version", "bundle-dir", "out");
    if (!VERSION_RE.test(o.version)) throw new Error(`not a version: ${o.version}`);
    const { copies, updater } = plan(o.platform, o.version, walk(o["bundle-dir"]));
    mkdirSync(o.out, { recursive: true });
    for (const c of copies) {
      copyFileSync(c.from, join(o.out, c.name));
      console.log(`${basename(c.from)} -> ${c.name}`);
    }
    const fragment = {};
    for (const [key, e] of Object.entries(updater)) fragment[key] = { asset: e.name, signature: readFileSync(`${e.from}.sig`, "utf8") };
    writeFileSync(join(o.out, `fragment-${o.platform}.json`), `${JSON.stringify(fragment, null, 2)}\n`);
  } else if (cmd === "latest") {
    need(o, "version", "tag", "repo", "notes", "fragments", "out");
    const entries = {};
    for (const f of readdirSync(o.fragments).filter((n) => /^fragment-.*\.json$/.test(n))) {
      Object.assign(entries, JSON.parse(readFileSync(join(o.fragments, f), "utf8")));
    }
    const notes = existsSync(o.notes) ? readFileSync(o.notes, "utf8").trim() : "";
    const doc = latestJson({ version: o.version, tag: o.tag, repo: o.repo, notes, pubDate: new Date().toISOString().replace(/\.\d{3}Z$/, "Z"), entries });
    writeFileSync(o.out, `${JSON.stringify(doc, null, 2)}\n`);
    console.log(`latest.json: ${Object.keys(doc.platforms).join(", ")}`);
  } else {
    throw new Error("usage: release.mjs notes|collect|latest --option value …");
  }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  try {
    main(process.argv.slice(2));
  } catch (e) {
    console.error(`release.mjs: ${e instanceof Error ? e.message : String(e)}`);
    process.exit(1);
  }
}
