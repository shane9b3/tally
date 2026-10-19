# Tally

Tally is a modern, fast spreadsheet for macOS, Windows and Linux. It aims for
OpenOffice Calc compatible behaviour (the same functions, `Err:5xx` codes and
OpenDocument files) with a clean, modern interface. "Tally" is a placeholder name.

Status: Phase A (first runnable build) in progress. See `docs/phase-a.md`.

## Stack

- **Engine:** Rust (`crates/tally-*`): formula parser, function library, number
  formats, dependency graph and recalculation, ODS and CSV IO.
- **Shell:** Tauri 2.12 (`apps/desktop`, arriving with WP7).
- **UI:** React 19 + TypeScript, with a canvas grid we write ourselves (`packages/*`).
- **Contracts:** Rust DTOs in `tally-api` exported to TypeScript with ts-rs.

Decisions and exact version pins: `docs/adr/0001-stack.md`.

## Prerequisites

- Rust 1.90+ (CI uses 1.94, see `rust-toolchain.toml`)
- Node 24 and pnpm (`corepack enable`). **Use pnpm, not npm.**
- Linux only: `libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev patchelf`

## Build and test

```sh
pnpm install                 # JS dependencies (exact versions, lockfile committed)
cargo test --workspace       # Rust tests (also regenerates TS bindings)
pnpm -r typecheck            # TypeScript
pnpm -r test                 # vitest
cargo xtask gen-bindings     # regenerate packages/bridge/src/generated explicitly
pnpm tauri dev               # run the desktop app (available once WP7 lands)
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
  bridge/           @tally/bridge: generated types, EngineClient, MockClient
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
