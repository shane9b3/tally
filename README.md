# Tally

Tally is a modern, fast spreadsheet for macOS, Windows and Linux. It aims for
OpenOffice Calc compatible behaviour (the same functions, `Err:5xx` codes and
OpenDocument files) with a clean, modern interface. "Tally" is a placeholder name.

Status: waves 1–5 are integrated; the desktop app runs the real engine. See
`docs/TODO.md` for what remains before 1.0.

## Features

- All 408 Calc functions (405 AOO 4.1 + IFERROR, IFNA, NUMBERVALUE,
  ISOWEEKNUM), AOO error codes, iterative calculation, precision as shown,
  de/fr/es function names and locale separators.
- Files: ODS/OTS, XLSX, CSV/TSV and HTML read + write; XLS, SXC read; dBase
  read + write; PDF export and printing with a print preview.
- Charts (column, bar, line, area, pie/donut, scatter, bubble, radar, stock;
  trend lines, error bars) and pictures, saved in ODS and XLSX.
- DataPilot (pivot tables) with GETPIVOTDATA, outline groups, standard and
  advanced filters, AutoFilter, subtotals, consolidate, text to columns,
  remove duplicates, scenarios, the linear solver, goal seek, multiple
  operations, detective and Mark Invalid Data.
- Named cell styles, AutoFormat, conditional formatting, validation, notes,
  hyperlinks, sheet protection, page setup, Window ▸ Split, view toggles
  (formulas, value highlighting, zero values, gridlines, headers).
- Function Wizard with argument fields and a live result, AutoInput,
  AutoCorrect, Navigator, Styles panel, templates, autosave and crash
  recovery, light and dark themes, English/German/French/Spanish UI, and the
  ⌘K assist.

## Stack

- **Engine:** Rust (`crates/tally-*`): formula parser, function library, number
  formats, dependency graph and recalculation, ODS and CSV IO.
- **Shell:** Tauri 2.12 (`apps/desktop`).
- **UI:** React 19 + TypeScript, with a canvas grid we write ourselves (`packages/*`).
- **Contracts:** Rust DTOs in `tally-api` exported to TypeScript with ts-rs.

Decisions and exact version pins: `docs/adr/0001-stack.md`.

## Prerequisites

- Rust 1.90+ (CI uses 1.94, see `rust-toolchain.toml`)
- Node 24 and pnpm (`corepack enable`). **Use pnpm, not npm.**
- Linux only: `libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev patchelf`

## Quick start

```sh
pnpm install                 # JS dependencies (exact versions, lockfile committed)
pnpm tauri dev               # build the Rust side and open the desktop app
```

`pnpm dev:web` serves the same UI in a browser against the in-memory
`MockClient` (no engine). `pnpm tauri build` makes `target/release/bundle/macos/Tally.app`
and a `.dmg` (unsigned).

## Install

Download the installer for your system from
[GitHub Releases](https://github.com/shane9b3/tally/releases/latest): a
`.dmg` for macOS (Apple Silicon `aarch64` or Intel `x64`), the
`-setup.exe` (or `.msi`) for Windows, the `.AppImage` (or `.deb`/`.rpm`)
for Linux. On macOS, drag Tally to Applications, then eject the Tally disk
image (otherwise it stays mounted and its copy can be launched by mistake).
The builds are not yet signed by Apple or Microsoft: on macOS
the first launch needs System Settings ▸ Privacy & Security ▸ Open Anyway,
on Windows SmartScreen's More info ▸ Run anyway (details in
[docs/release.md](docs/release.md#9-unsigned-builds-first-install-friction)).

Tally then updates itself: it checks GitHub once a day and asks before
installing a new version (Tools ▸ Options ▸ Online Update to turn the check
off; Check for Updates… in the Help menu, or the Tally menu on macOS).
Copies installed from the `.deb`/`.rpm` packages are updated by
downloading the new package. Releases are made as described in
[docs/release.md](docs/release.md).

## Build and test

```sh
pnpm install                 # JS dependencies (exact versions, lockfile committed)
cargo test --workspace --features tally-engine/io   # Rust tests (also regenerates TS bindings)
pnpm -r typecheck            # TypeScript
pnpm -r test                 # vitest
cargo xtask gen-bindings     # regenerate packages/bridge/src/generated explicitly
pnpm e2e                     # Playwright against the MockClient web build
```

## Repository layout

```
Cargo.toml, rust-toolchain.toml, deny.toml   Rust workspace, toolchain, licence policy
package.json, pnpm-workspace.yaml, .npmrc    pnpm workspace
crates/
  tally-types/      CellAddr, CellRange, Value, ErrorKind, CalcSettings (shared contract)
  tally-model/      WorkbookData interchange model used by IO and the engine (shared contract)
  tally-api/        DTOs (exported to TS) and the DocumentService trait (shared contract)
  tally-formula/    lexer, parser, dialects, printer, explain()             (WP2)
  tally-numfmt/     number formats, input recognition, dates                (WP4)
  tally-functions/  function registry, coercion, criteria, functions        (WP3)
  tally-engine/     storage, dependencies, recalc, ops, undo                (WP1)
  tally-io-ods/     ODS read/write                                          (WP5)
  tally-io-csv/     CSV import/export                                       (WP5)
  tally-assist/     rule-based natural language to formula (⌘K)            (WP8)
  tally-cli/        command-line eval/convert/recalc/diff                   (WP1)
xtask/              cargo xtask gen-bindings | function-coverage
packages/
  bridge/           @tally/bridge: generated types, EngineClient, ShellClient, MockClient, TauriClient
  tokens/           @tally/tokens: CSS design tokens + typed Theme reader
  grid/             @tally/grid: canvas grid                                (WP6)
  app/              @tally/app: React chrome and stores                     (WP7)
apps/desktop/       Tauri app                                               (WP7)
docs/               plan, ADRs, API and contributing guides
```

## Documentation

- `docs/phase-a.md`: Phase A scope, merge order and done checks
- `docs/architecture-plan.md`: full architecture and work packages
- `docs/adr/`: architecture decision records
- `docs/api.md`: Tauri commands ↔ `DocumentService` ↔ `EngineClient`
- `docs/contributing.md`: work package ownership and contribution rules

## Licence

Apache-2.0 (see `LICENSE`). Some algorithms are ported from Apache OpenOffice;
see `NOTICE`.
