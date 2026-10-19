# Contributing

## Work package ownership

Phase A is split into work packages (WP0–WP8, see `docs/phase-a.md` and
`docs/architecture-plan.md` §7). The rule that keeps parallel work mergeable:

- **Each WP owns a disjoint set of files.** Only the owning WP edits them. If you
  need a change in someone else's files, ask the owner or open a PR they review.
- **Shared contracts change only via lead review.** These are:
  - `crates/tally-types` (addresses, values, errors, settings),
  - `crates/tally-model` (the `WorkbookData` interchange model),
  - `crates/tally-api/src/dto.rs` and `crates/tally-api/src/service.rs`
    (DTOs and the `DocumentService` trait),
  - `packages/bridge/src/client.ts` (the `EngineClient` interface) and
    `packages/bridge/src/generated/` (never edited by hand).

  Additive changes (a new optional field, a new `Op` variant, a new method) are
  fine with lead review. Breaking changes need sign-off from every WP that
  consumes the contract.
- Root configs (`Cargo.toml`, `package.json`, `pnpm-workspace.yaml`, `deny.toml`,
  `.github/`) belong to WP0/the lead. Adding a dependency means adding an exact
  pin to `[workspace.dependencies]` (Rust) or an exact version (pnpm) in a
  lead-reviewed PR.

| WP | Owns |
|---|---|
| WP0 | root configs, `crates/tally-types`, `crates/tally-model`, `crates/tally-api/src/{dto,lib}.rs`, `packages/bridge/src/{client.ts,index.ts,generated/}`, `.github/`, `docs/adr/` |
| WP1 | `crates/tally-engine`, `crates/tally-api/src/service.rs` impl, `crates/tally-cli` |
| WP2 | `crates/tally-formula` |
| WP3 | `crates/tally-functions`, `tests/conformance/**` |
| WP4 | `crates/tally-numfmt` |
| WP5 | `crates/tally-io-ods`, `crates/tally-io-csv`, `tests/fixtures/**` |
| WP6 | `packages/grid`, `packages/formula-ts`, `packages/tokens` |
| WP7 | `apps/desktop/**`, `packages/app/**`, `packages/bridge/src/{tauri.ts,mock.ts}` |
| WP8 | `crates/tally-assist` |

## Before you push

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo xtask gen-bindings --check   # after touching tally-api DTOs: run without --check and commit
pnpm -r typecheck
pnpm -r test
```

## Rules of the road

- Use **pnpm**, never npm. Versions are exact (`save-exact`); commit `pnpm-lock.yaml`
  and `Cargo.lock`.
- Never run a bare `cargo add tauri` (it resolves to the 3.0 alpha); use the
  workspace pin.
- No `unsafe`. No `unwrap()` in library code that handles data; tests may unwrap.
- Licensing: Apache-2.0. You may port algorithms from Apache OpenOffice
  (Apache-2.0) with a comment naming the original source file. Never copy code
  from LibreOffice (MPL-2.0), IronCalc or formualizer; they are reference oracles only.
- No "OpenOffice" or "Calc" branding in the product (Apache trademarks).
