# Releasing Tally and the auto-updater

Tally is distributed through the GitHub Releases of the public repository
<https://github.com/shane9b3/tally>. A release is built by GitHub Actions
(`.github/workflows/release.yml`) when a `v*` tag is pushed. Installed
copies find new versions through the static manifest
`https://github.com/shane9b3/tally/releases/latest/download/latest.json`,
which that workflow uploads last.

> **Status (2026-11-11):** the workflow has never run. Everything below
> that involves GitHub Actions, a real release or a real update is written
> from the documentation of the actions and the updater plugin, not from a
> run. Do the first release as a test (section 6) before announcing it.

## 1. How the pieces fit

| Piece | Where |
|---|---|
| Updater plugin | `tauri-plugin-updater` =2.14.1 (workspace `Cargo.toml`), driven from Rust only: `apps/desktop/src-tauri/src/updater.rs`. No `updater:*` permission is granted to any webview; the CSP stays IPC-only. |
| Endpoint and public key | `apps/desktop/src-tauri/tauri.conf.json` ▸ `plugins.updater` (`endpoints`, `pubkey`, `requireSignedVersion: true`, Windows `installMode: passive`). |
| Release-only build settings | `apps/desktop/src-tauri/tauri.release.conf.json`, passed with `--config` by the workflow: `bundle.createUpdaterArtifacts: true` (signed updater packages) and macOS `signingIdentity: "-"` (ad-hoc signing). Kept out of `tauri.conf.json` so a local build needs no key (section 8). |
| Version | `cargo xtask bump-version <x.y.z>` writes it everywhere; `cargo xtask bump-version --check [x.y.z]` (CI, and the release workflow against the tag) checks that they agree. |
| Release notes | `CHANGELOG.md`, the section `## [x.y.z]`; GitHub's generated notes when there is none. The release body becomes `notes` in `latest.json`, which the app shows as plain text. |
| Workflow helpers | `.github/release/release.mjs` (notes, artifact collection, `latest.json`), tested by `node --test .github/release/release.test.mjs` in CI. |
| The app's side | Automatic check 5 s after launch and every 24 h while running (Options ▸ Online Update ▸ "Check for updates automatically", default on); Help ▸ Check for Updates… (the Tally menu on macOS). See `docs/api.md` "Auto-update" and `docs/security.md` "Auto-update". |

## 2. One-time setup (owner)

1. **Signing key.** The minisign key pair exists. The private key lives at
   `~/.tauri/tally-updater.key` on the owner's Mac and in the repository
   secret `TAURI_SIGNING_PRIVATE_KEY` (Settings ▸ Secrets and variables ▸
   Actions). Its password is empty; the workflow sets
   `TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ""`. The public key is committed in
   `tauri.conf.json`. **Back the private key up offline** (section 7).
2. **Actions permissions.** Settings ▸ Actions ▸ General: allow GitHub
   Actions; "Workflow permissions" may stay read-only (the workflow asks
   for `contents: write` per job).
3. **Tag protection (recommended).** Settings ▸ Rules ▸ add a tag ruleset
   for `v*` so only you can create release tags.

## 3. Making a release, step by step

```sh
git switch main && git pull
cargo xtask bump-version 0.2.0        # Cargo.toml, Cargo.lock, tauri.conf.json, every package.json
cargo xtask bump-version --check 0.2.0
```

Edit `CHANGELOG.md`: rename `## [Unreleased]` to `## [0.2.0] - 2026-12-01`
(today's date) and add a new empty `## [Unreleased]` above it. Write the
notes for people who use Tally: they appear in the app's update prompt.

```sh
cargo test -p xtask                  # quick sanity
git add Cargo.toml Cargo.lock apps/desktop/src-tauri/tauri.conf.json \
  apps/desktop/package.json packages/*/package.json CHANGELOG.md
git commit -m "Release 0.2.0"
git push
git tag -a v0.2.0 -m "Tally 0.2.0"
git push origin v0.2.0               # starts the release workflow
```

To rebuild an existing tag (e.g. after a runner failure) use Actions ▸
Release ▸ "Run workflow" with the tag name. That works only while the
release is still a draft: a published release is never rebuilt (bump the
version instead).

## 4. What the workflow does

1. **verify** (read-only): the tag must look like `vX.Y.Z[-pre]` and
   `cargo xtask bump-version --check X.Y.Z` must pass, so the tag, the
   Cargo workspace, `Cargo.lock`, `tauri.conf.json` and every
   `package.json` agree. Fails fast otherwise.
2. **draft**: creates the GitHub Release as a **draft** titled
   "Tally X.Y.Z" with the `CHANGELOG.md` section as its body (or GitHub's
   generated notes). A draft left by a failed run is reused; a published
   release with that tag stops the run.
3. **build** (four jobs in parallel): `darwin-aarch64` and `darwin-x86_64`
   (both on `macos-15`, the Intel one cross-compiled), `windows-x86_64`
   (`windows-latest`), `linux-x86_64` (`ubuntu-22.04`, for an AppImage that
   runs on older glibc). Each runs `tauri-apps/tauri-action` (build only)
   with `--config src-tauri/tauri.release.conf.json` and the signing key,
   then `release.mjs collect` copies the results under fixed names and the
   job uploads them to the draft:
   - macOS: `Tally_X.Y.Z_aarch64.dmg` / `Tally_X.Y.Z_x64.dmg`, and the
     updater packages `Tally_X.Y.Z_<arch>.app.tar.gz` + `.sig` (Tauri names
     both `Tally.app.tar.gz`, hence the renaming);
   - Windows: `Tally_X.Y.Z_x64-setup.exe` (NSIS, the updater's package) and
     `Tally_X.Y.Z_x64_en-US.msi`, each with `.sig`;
   - Linux: `Tally_X.Y.Z_amd64.AppImage` + `.sig` (updatable),
     `Tally_X.Y.Z_amd64.deb`, `Tally-X.Y.Z-1.x86_64.rpm` (plain downloads:
     copies installed from them do not offer in-app updates).
   Each job also leaves a `fragment-<platform>.json` workflow artifact with
   its `latest.json` entries.
4. **publish** (only when all four builds succeeded): merges the fragments
   into `latest.json` (`version`, `notes` = the release body, `pub_date`,
   and `platforms` for `darwin-aarch64`, `darwin-x86_64`, `windows-x86_64`
   (+ `-nsis`, `-msi`), `linux-x86_64` (+ `-appimage`); it fails if any of
   the four is missing), uploads it, then takes the release out of draft and
   marks it latest. Until this step `releases/latest/download/latest.json`
   still serves the previous release, so a half-built release is never
   offered.

Permissions: the workflow default is none; `verify` reads, the other jobs
get `contents: write`. The signing key is given only to the build step and
the token only to the upload steps (not to `pnpm install` or build
scripts). There is no `pull_request` trigger, so code from a fork never
runs with the secret.

## 5. Checking a release

- Actions ▸ Release: all six jobs green.
- The release page lists 4 installers for macOS/Windows (2 dmg, exe, msi),
  3 for Linux, 4 `.sig` files and `latest.json`.
- `curl -sL https://github.com/shane9b3/tally/releases/latest/download/latest.json`
  shows the new `version` and a `platforms` entry with a non-empty
  `signature` and an `https://github.com/shane9b3/tally/releases/download/vX.Y.Z/…`
  `url` for each platform; each URL downloads (`curl -sIL <url>` → 200).
- Install the dmg on a Mac (first-install friction: section 9) and check
  Tools ▸ Options ▸ Online Update ▸ "Installed version".

## 6. Testing the updater end to end (two versions)

You need two published releases, the older one installed. Do this once
before relying on updates, and after any change to the updater.

1. Release `vA` (e.g. `v0.1.1`) as in section 3. Install it from its dmg
   into `/Applications` (section 9 for the first launch).
2. Release `vB` (e.g. `v0.1.2`), with a `CHANGELOG.md` section, so the
   prompt has notes to show. Wait for **publish** to finish.
3. Start `vA`. Within about 5 seconds the front window shows "Tally vB is
   available — you have vA" with the notes. (Or Help ▸ Check for
   Updates… / Tally ▸ Check for Updates… at once.)
4. Run the manual QA sheet `docs/qa/updater-manual.md` (unsaved documents,
   cancel, skip, offline, Windows/Linux where you can).
5. After Install and Restart, Options ▸ Online Update shows vB, and
   `codesign -dv /Applications/Tally.app` still reports an ad-hoc
   signature.

Test releases can be deleted afterwards, but keep the newest real release
published: `releases/latest` must always point at a release with a
`latest.json`, or every installed copy reports "could not check".

Debug builds (`pnpm tauri dev`) never check for updates
(`update_status.reason`); the browser build (`pnpm dev:web`) simulates
the server: `?update=0.2.0` offers a release, `?updateError=offline`
fails the check (`MockClient.updateHooks`, used by
`tests/e2e/updater.spec.ts`).

## 7. Key custody

- The **private key** signs every updater package. It lives at
  `~/.tauri/tally-updater.key` (owner's Mac) and in the Actions secret
  `TAURI_SIGNING_PRIVATE_KEY`. Keep an offline backup (password manager or
  an encrypted USB key). It never goes into the repository.
- **If it is lost**, no new release can be signed for the key the
  installed copies trust: they reject every future update (signature
  error) and stay on their version until the user downloads a new version
  by hand from the Releases page. Announce that in the release notes of the
  first release with a new key.
- **If it leaks**, someone could sign a malicious update, but installed
  copies only fetch from this repository's releases over HTTPS, so they
  would also need control of the repository (or its releases). Rotate at
  once and review the repository's access and recent releases.
- **Rotating** (old key still available): generate a new pair
  (`pnpm --filter tally-desktop tauri signer generate -w ~/.tauri/tally-updater-2.key`),
  put the new **public** key into `tauri.conf.json`, and make one release
  signed with the **old** private key (the CLI warns that the key does not
  match the configured public key; that is expected here). Installed copies
  accept it (old key) and from then on trust the new key. Then replace the
  secret with the new private key and sign every later release with it.
  Copies that skip that transition release can no longer update in place.

## 8. Local builds

`pnpm --filter tally-desktop tauri build` (the README's release build)
does **not** create updater artifacts and needs no key:
`createUpdaterArtifacts` is only in `tauri.release.conf.json`. The built
app still contains the updater, the endpoint and the public key, so a
local release build checks GitHub like a downloaded one (turn it off in
Options ▸ Online Update if that is unwanted).

A signed updater bundle locally (to test the packaging):

```sh
TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/tally-updater.key)" TAURI_SIGNING_PRIVATE_KEY_PASSWORD="" \
  pnpm --filter tally-desktop tauri build --config src-tauri/tauri.release.conf.json
```

Without the variable this command fails ("A public key has been found,
but no private key"), which is why the overlay is not the default.

## 9. Unsigned builds: first-install friction

There is no Apple Developer ID or Windows code-signing certificate yet.

- **macOS:** the app is ad-hoc signed (`signingIdentity: "-"`), so Apple
  Silicon runs it, but it is not notarized. The first launch of a
  downloaded copy is blocked ("Apple could not verify 'Tally' is free of
  malware…"). Open it anyway with System Settings ▸ Privacy & Security ▸
  "Open Anyway" (macOS 15+), or remove the quarantine flag:
  `xattr -dr com.apple.quarantine /Applications/Tally.app`. Updates
  installed by the app are downloaded by Tally itself, carry no quarantine
  flag and do not ask again. When Tally sits where the user cannot write,
  the installer asks for an administrator password.
- **Windows:** SmartScreen warns about the unsigned installer ("Windows
  protected your PC" ▸ More info ▸ Run anyway). The NSIS installer installs
  per user (no administrator rights); in-app updates run it in passive mode
  (progress window only) and it relaunches Tally.
- **Linux:** mark the AppImage executable (`chmod +x`). The AppImage
  updates itself in place; `.deb`/`.rpm` installs are updated through the
  package manager or a new download.

Developer ID signing + notarization and Windows Authenticode (or Azure
Trusted Signing) remain on the 1.0 list (docs/TODO.md).

## 10. Troubleshooting

- **verify fails:** the tag and the files disagree. Run
  `cargo xtask bump-version --check`, fix, commit, delete and re-push the tag
  (`git tag -d vX.Y.Z && git push origin :refs/tags/vX.Y.Z`) while the
  release is still a draft.
- **A build job fails:** fix, then Actions ▸ Release ▸ Run workflow with
  the same tag (the draft is reused; uploads use `--clobber`). If the fix
  needs a code change, delete the draft and the tag and tag again.
- **"no signature next to …"** in the collect step: the secret is missing
  or empty.
- **The app says "could not check":** `releases/latest` has no
  `latest.json` (a release published by hand?), or the machine is offline
  / behind a proxy that blocks github.com (the system proxy is used).
- **Signature errors in the app:** the release was signed with a key other
  than the one in the installed app's `tauri.conf.json` (section 7).
