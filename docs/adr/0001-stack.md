# ADR-0001: Stack and baseline decisions

Status: accepted, 2026-10-19

## Decisions

- **Stack:** Rust engine crates (`tally-*`), Tauri 2.12 shell, React 19 + TypeScript UI, and a canvas grid we write ourselves.
  - The engine is the product, so we build it. IronCalc and formualizer are read-only reference oracles; no code is copied from them.
  - No LibreOffice (MPL) code is copied. Apache OpenOffice (Apache-2.0) algorithms may be ported with attribution.
- **Name:** "Tally" is a placeholder. Crates use the prefix `tally-`, npm packages the scope `@tally/`. No "OpenOffice" or "Calc" branding (Apache trademarks).
- **Behaviour baseline:** OpenOffice-compatible semantics (Err:5xx codes, OpenFormula storage, `Sheet.A1` references, the AOO function set) with modern defaults where AOO's are traps:
  - Criteria mode defaults to wildcards; regex and literal selectable in settings.
  - en-US UI argument separator is `,`; `;` is always accepted on input. ODS storage always uses `;` (OpenFormula).
  - Input accepts both `Sheet.A1` and `Sheet!A1`; formulas print in the user's UI dialect (default CalcA1 with `,`).
- **Grid size:** 1,048,576 rows × 16,384 columns.
- **Text in arithmetic:** converts only when unambiguous (ConvertUnambiguous).
- **Calendar:** proleptic Gregorian, no Julian switch. Default epoch 1899-12-30; 1900 and 1904 selectable.
- **Macros:** never executed; preserved as opaque blobs and shown with a banner.
- **Builds:** unsigned for now. Intel macOS and universal builds come from CI (local Homebrew Rust has arm64 std only).
- **Web:** not a product goal; `tally-wasm` must keep compiling in CI from v0.2.
- **Assist (⌘K):** v0.1 ships only the `AssistProvider` trait and the rule-based provider. No LLM code, no network access by default. v0.1 inserts plain ranges; header-name references come in v0.3.
- **Share:** export (ODS/CSV now, XLSX/PDF later) plus the macOS share sheet. The op log is designed so a CRDT could be layered later; we do not build for it now.
- **Toolchain:** stable versions only, exact pins, committed `Cargo.lock` and `pnpm-lock.yaml`. Workspace `rust-version = "1.90"` (Tauri 2.12 requirement). Never bare `cargo add tauri` (pulls the 3.0 alpha).
- **Security posture:** untrusted file parsing happens only in Rust with size/depth/ratio limits; the webview never receives raw file bytes; no fs/shell plugins exposed to JS; strict CSP; devtools off in release. See docs/architecture-plan.md §5.4.

## Pinned versions (Phase A)

package.json (`.npmrc` has `save-exact=true`): typescript 5.9.3, vite 7.3.7, @vitejs/plugin-react 5.1.4, react 19.3.1, react-dom 19.3.1, @types/react 19.3.0, @types/react-dom 19.3.0, zustand 5.0.15, cmdk 1.1.1, @fontsource-variable/inter 5.3.0, @tauri-apps/api 2.12.4, @tauri-apps/cli 2.12.3, vitest 3.2.7, @playwright/test 1.64.1.

Cargo `[workspace.dependencies]`: tauri 2.12.4, tauri-build 2.7.3, tauri-plugin-dialog 2.8.3, serde 1.0.229 (derive, rc), serde_json 1.0.154, ts-rs 12.0.1, quick-xml 0.42.0, zip 9.0.5 (default-features off, deflate), csv 1.4.0, encoding_rs 0.8.42, chrono 0.4.45 (std, clock), slotmap 1.1.1, smallvec 1.16.3 (not the 2.0 beta), regex 1.13.1, thiserror 2.0.21; dev: proptest 1.12.0, insta 1.49.0, criterion 0.8.2, toml 1.1.8.
