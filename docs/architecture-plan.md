# Implementation Plan: Modern Calc Reimplementation (codename TBD, placeholder crate prefix `calc-`, npm scope `@calc/`)

## 0. What I checked on this machine

- `/Volumes/FILES/Projects/OSX_Office` exists and is empty. It is not a git repo.
- Toolchain: cargo/rustc 1.94.0 from Homebrew, with no rustup. **Only the `aarch64-apple-darwin` std is installed** (`rustlib/` contains just that target), so you cannot build x86_64-apple-darwin or universal macOS binaries on this machine without installing rustup. Node v24.13, pnpm 11.13.1.
- **npm's cache is broken on this machine.** `npm view` failed with EACCES because `~/.npm` contains root-owned files. Use pnpm, or fix it with `sudo chown -R 501:20 ~/.npm`.
- **Tauri trap:** the latest Tauri on crates.io is `3.0.0-alpha.5`, and the plugins are also on 3.x alphas. Stable is **tauri 2.12.4** and `@tauri-apps/cli` 2.12.3. Pin to `tauri = "2.12"` and the matching plugin 2.x versions. Never use bare `cargo add tauri`, because it can pull the alpha.
- Crates that are available and relevant: quick-xml 0.42, zip 9.0, calamine 0.37 (MIT, reads xlsx/xls/ods), rust_xlsxwriter 0.100 (write-only, mature), umya-spreadsheet 3.1 (read/write), csv 1.4, chrono 0.4.45, ts-rs 12.0 (stable), specta/tauri-specta 2.0.0-rc.25 (still RC), logos 0.16, petgraph 0.8, proptest 1.12, insta 1.49, criterion 0.8, statrs 0.19, rayon 1.12, slotmap 1.1, compact_str 0.10, regex 1.13 (linear time), fancy-regex 0.19 (backtracking), icu_decimal 2.3, wasm-bindgen 0.2.129. Native GUI options: egui 0.37, iced 0.14, slint 1.18, gpui 0.2.2.
- **Existing Rust spreadsheet engines:** `ironcalc` / `ironcalc_base` 0.9 and `formualizer` 0.11 are both MIT OR Apache-2.0. These matter for the build-vs-adopt decision (section 1.4).
- npm packages: react 19.3.1, vite 8.3.4, zustand 5.0.15, cmdk 1.1.1, @fontsource-variable/inter 5.3.0, vitest 5.0.5, @playwright/test 1.64.1, typescript 7.0.2 (the new native compiler, so check that typescript-eslint supports it; otherwise pin 5.9), tailwindcss 4.x, echarts 6.1.0, uplot 1.6.32, electron 44.9, @glideapps/glide-data-grid 6.0.3.

---

## 1. Stack decision

### 1.1 Scoring (1 = poor, 5 = excellent)

| Criterion | (a) Rust engine + Tauri 2 + React/TS | (b) Rust + native Rust GUI (egui/iced/Slint/gpui) | (c) C#/.NET + Avalonia | (d) TS + Electron | (e) Kotlin Compose MP |
|---|---|---|---|---|---|
| Native arm64 + x64 on mac/Win/Linux | 5 | 5 | 5 | 5 | 4 (bundled JVM; jpackage per arch) |
| Matching the Notion/Linear mockup | 5 (CSS is the natural medium for this look) | 2 (egui looks like egui; iced/Slint mean hand-building every widget; gpui looks best but its API is unstable and it's tied to Zed) | 4 | 5 | 4 |
| Large-grid performance (100k+ rows, virtualized) | 4 (canvas grid with a Rust engine; IPC is the main cost; WebKitGTK on Linux is the weak spot) | 5 | 4–5 (Skia custom control) | 3–4 (an engine in JS is fine for display but slower at bulk recalc, with GC pauses and more memory) | 4 |
| Long-term maintainability | 4 (two languages, but a very clean boundary) | 3 (young ecosystems; IME, accessibility and text input are immature) | 4 | 4 (one language) | 3 |
| Binary size | 5 (about 10–25 MB) | 5 (about 10–20 MB) | 3 (40–80 MB self-contained, less with AOT) | 2 (120–200 MB) | 2 (about 100 MB with JRE) |
| Implementing 400+ functions with tests | 5 (Rust unit/property/snapshot tests, fast CI) | 5 | 4 | 4 | 4 |
| Sharing one calc engine (desktop, web, CLI, server) | 5 (native + WASM + CLI) | 5 | 3 (WASM via Blazor is heavy) | 4 | 4 (KMP targets) |
| Effort to a working v0.1 | 4 | 2 | 3 (dotnet not installed; team skill unknown) | 5 | 3 (JDK and Gradle not installed) |
| **Total (/40)** | **37** | **32** | **31** | **33** | **28** |

### 1.2 Recommendation: (a), with five changes to your leaning

Your leaning is right. A Rust engine crate with no UI dependencies is the most valuable asset in this project. It is testable in isolation, it compiles to WASM, it can back a CLI for conformance testing, and the 400+ functions get Rust's correctness tooling. A web frontend is clearly the cheapest way to hit the mockup: pill buttons, chips, the command palette, the sidebar, Inter typography and dark mode are all trivial in CSS and expensive in every native toolkit.

Changes and challenges:

1. **Build the grid yourself, framework-agnostic, on canvas.** Do not use glide-data-grid. It is a record-oriented data grid, not a spreadsheet surface: it does not handle merges, text overflow into empty neighbours, per-edge borders or the fill handle the way Calc does. The grid is the heart of the product, so own it. Write it as a plain TypeScript class (`@calc/grid`) with a thin React wrapper, so React re-renders never sit on the scroll or paint path.
2. **Put a transport-agnostic `calc-api` facade between the engine and Tauri.** Tauri commands are just one adapter; a `wasm-bindgen` adapter and a CLI are others. This hedges the biggest real risk of (a): Tauri uses three different webviews (WKWebView, WebView2, WebKitGTK), and Linux WebKitGTK has historically been the weakest on canvas performance, GPU acceleration and IME. If that becomes a blocker, moving the shell to Electron is shell-only work, because the engine moves to WASM or napi and the UI is unchanged.
3. **Engine location in v0.1: native, in the Tauri backend process.** The alternative (WASM inside a Web Worker) avoids IPC and gives a web build for free, but native gives file IO without exposing an fs API to JavaScript, multithreading later, no 4 GB wasm32 memory ceiling, and roughly 1.5–2x speed. From v0.2, keep a WASM build compiling in CI and use it to drive Playwright end-to-end tests in a plain browser.
4. **React over Svelte/Solid.** The chrome is not performance-critical once the grid is imperative. React has the best ecosystem for this UI (cmdk, Radix primitives) and the deepest hiring pool. Use zustand for state.
5. **Build vs adopt the engine: build our own, after a time-boxed 2-day IronCalc spike.** IronCalc is MIT/Apache Rust, compiles to WASM, has a few hundred functions, and is Excel-semantics first. Reasons to build:
   - OpenOffice semantics are pervasive: `Err:5xx` codes, `;` argument separators, `Sheet1.A1` references, regex-or-wildcard criteria modes, the text-to-number conversion setting, ODF OpenFormula storage, and `$Sheet` absolute sheet references.
   - We need control over columnar storage for a million rows and over the dependency-graph design.
   - The engine is the product.

   Spike decision criteria: can it be bent to ODF semantics without a hard fork, does its storage scale to 1M×50, and is its dependency model incremental? If two or more fail, build. Either way, use IronCalc and formualizer as differential-test oracles and reference code (the licences allow this with attribution).

**Licensing note:** Apache OpenOffice is Apache-2.0. The function algorithms in `sc/source/core/tool/interpr*.cxx` and `scaddins/` can legally be ported with attribution, which is a big accelerator for Financial, Statistical and Add-in. Do **not** copy LibreOffice code (MPL-2.0). Read its behaviour; don't paste its source. Recommended project licence: Apache-2.0. Do not use "OpenOffice" or "Calc" in the product name (Apache trademarks).

### 1.3 Window chrome

- macOS: `titleBarStyle: "Overlay"` and `hiddenTitle: true`, with traffic lights inset into the top toolbar and a drag region on the toolbar background.
- Windows and Linux in v0.1: native decorations. A custom Windows title bar with Snap Layouts support comes in v0.3+. Keep native decorations on Linux long-term, since Wayland compositors differ.

---

## 2. Repository layout (monorepo)

```
OSX_Office/
  Cargo.toml                 # [workspace]; shared [workspace.dependencies] with pinned versions
  rust-toolchain.toml        # channel = "1.94" (used by CI rustup; Homebrew ignores it)
  package.json, pnpm-workspace.yaml, .npmrc
  deny.toml                  # cargo-deny: licences (reject GPL/MPL in deps), advisories
  crates/
    calc-types/              # CellAddr, CellRange, SheetId, Value, ErrorKind, Matrix, CalcSettings. Only dep: serde (optional feature)
    calc-formula/            # lexer, parser (Pratt), AST, dialects (CalcA1, ExcelA1, OpenFormula, OOXML), printer, ref relativization, explain()
      tests/fixtures/tokens/*.json   # shared lexer vectors (also consumed by the TS lexer)
    calc-numfmt/             # format-code parser/formatter, input recognizer, date serials, locale tables
    calc-functions/          # registry, arg coercion, criteria, function impls
      src/registry.rs, args.rs, criteria.rs, coerce.rs
      src/categories/{math,statistical,text,logical,datetime,lookup,financial,information,database,array,addin}/  # one file per function family
    calc-model/              # plain serde interchange model (WorkbookData) used by IO crates; no engine dependency
    calc-engine/             # workbook, sheet storage, styles, deps graph, recalc, evaluator, ops/transactions, undo, clipboard
      tests/conformance.rs   # runs tests/conformance/**.toml end to end
    calc-io-ods/  calc-io-xlsx/  calc-io-csv/   # file bytes <-> calc-model::WorkbookData
    calc-api/                # DTOs (ts-rs export), DocumentService facade, transport-agnostic
    calc-assist/             # natural-language-to-formula rules + AssistProvider trait
    calc-wasm/               # wasm-bindgen wrapper of calc-api (from v0.2)
    calc-cli/                # `calc eval|convert|recalc|diff`, used by the conformance oracle and power users
  apps/desktop/
    src-tauri/               # Tauri 2: main.rs, commands.rs, state.rs, events.rs, menu.rs, dialogs.rs, capabilities/default.json, tauri.conf.json
    index.html, src/main.tsx # mounts @calc/app with TauriClient
  packages/
    grid/                    # @calc/grid: canvas renderer, virtualization, selection, hit-testing, editor host
    app/                     # @calc/app: React chrome (toolbar, formula bar, name box, tabs, status bar, sidebar, palette, glance panel), stores
    bridge/                  # @calc/bridge: generated TS types, EngineClient interface, TauriClient, MockClient, (WasmClient later)
    formula-ts/              # TS lexer for formula highlighting/point-mode (validated against the shared fixtures)
    tokens/                  # design tokens: CSS variables (light/dark) + TS Theme reader for canvas
  tests/
    conformance/<category>/<FUNC>.toml  # function test vectors (owned per category)
    fixtures/{ods,xlsx,csv}/ # sample files incl. round-trip and malicious samples (zip bombs, XXE, traversal)
    e2e/                     # Playwright specs
  fuzz/                      # cargo-fuzz targets: formula parse, numfmt parse, ods read, xlsx read, csv read
  xtask/                     # cargo xtask: gen-bindings, function-coverage, oracle (LibreOffice/AOO headless), bench
  docs/
    adr/0001-stack.md ...    # architecture decision records
    architecture.md, formula-syntax.md, file-formats.md, security.md, contributing-functions.md
    functions/coverage.md    # generated: AOO master list vs implemented
    functions/master-list.toml  # every AOO 4.1 function: name, category, ODF name, OOXML name, arity
  .github/workflows/ ci.yml, release.yml, fuzz.yml (nightly schedule), bindings-check
```

**Dependency direction (no cycles):** `calc-types` → `calc-formula`, `calc-numfmt` → `calc-functions` → `calc-engine` → `calc-api` → {`src-tauri`, `calc-wasm`, `calc-cli`}. The IO crates depend only on `calc-model` (+ `calc-types`). `calc-engine` depends on `calc-model` to provide `Workbook::from_data` and `to_data`. `calc-assist` depends on `calc-formula` and a read-only engine view trait.

### CI matrix (GitHub Actions)

| Job | Runners | Steps |
|---|---|---|
| rust-check | ubuntu-24.04 | fmt, clippy `-D warnings`, cargo-deny, ts-bindings freshness (`xtask gen-bindings --check`), function-coverage report |
| rust-test | macos-15 (arm64), macos-15-intel or cross-compile x86_64 on arm64 and run under Rosetta, windows-latest (x64), windows-11-arm, ubuntu-24.04, ubuntu-24.04-arm | `cargo test --workspace` incl. conformance |
| wasm-check (v0.2+) | ubuntu | `cargo build -p calc-wasm --target wasm32-unknown-unknown` |
| web | ubuntu | pnpm install, typecheck, eslint, vitest (grid + app + formula-ts lexer fixtures) |
| e2e | ubuntu (browser + MockClient/WasmClient); macOS/Windows/Linux smoke with tauri-driver where supported | Playwright |
| bundle (release.yml, tags + nightly) | mac arm64 + x64 (or universal-apple-darwin), win x64/arm64, linux x64/arm64 | `tauri build` → dmg, msi/nsis, AppImage/deb/rpm; signing/notarization secrets later |
| fuzz (scheduled) | ubuntu nightly toolchain | cargo-fuzz 10 min per target |
| bench (scheduled) | fixed runner | criterion, fail on >15% regression |

The Linux runner needs `libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev patchelf`. Watch GitHub's Intel macOS runner deprecation; cross-compiling x86_64 on arm64 runners is the durable fallback.

---

## 3. Calc engine design

### 3.1 Limits and addressing

- 0-based `row: u32` and `col: u32` internally. Limits: `MAX_ROWS = 1_048_576`, `MAX_COLS = 16_384`, matching XLSX and modern LibreOffice. AOO 4.1 is 1,048,576 × 1,024, so a superset is safe.
- `SheetId` is a stable id that is never reused and is independent of tab order. Formulas reference sheets by id, so renaming or reordering never rewrites ASTs.

### 3.2 Cell storage (sparse, columnar)

- `Sheet { columns: Vec<Column> (lazily grown), row_dims: DimRuns, col_dims: DimRuns, merges, frozen, ... }`.
- `Column` holds **block-run storage**, modelled on the mdds `multi_type_vector` design that LibreOffice uses:
  - a sorted `Vec<Block>` where each `Block { start_row, kind }` with `kind ∈ { Empty(len), Numbers(Vec<f64>), Texts(Vec<StrId>), Bools(BitVec), Errors(Vec<ErrorKind>), Formulas(Vec<FormulaCellId>) }`;
  - contiguous numeric runs make SUM over A1:A1000000 a tight loop over `&[f64]`, and empty space costs nothing;
  - v0.1 may implement `Column` as `BTreeMap<u32, CellContent>` **behind a `ColumnStore` trait** with the same API (`get`, `set`, `iter_range`, `next_non_empty`, `insert_rows`, `delete_rows`), then swap to blocks in v0.3 with benchmarks as the gate.
- Formula cells live in a `SlotMap<FormulaCellId, FormulaCell { addr, expr: Arc<CompiledFormula>, cached: Value, state: Clean|Dirty|Evaluating, flags (volatile, array_anchor, spill) }>`.
- Shared/identical formulas (common after fill-down) share one `Arc<CompiledFormula>` because references are stored relative. This is exactly what makes copy and fill cheap.
- Strings are interned in a workbook-level `StringPool` (`StrId(u32)`, `Arc<str>`), with case-folded keys cached for lookup functions.
- Styles: a workbook-level `StyleTable` (deduplicated, `StyleId(u32)`). Each column has run-length `(row_start, row_end, StyleId)` ranges, plus column and row default styles. Number format is part of the style (`NumFmtId` into a format table of parsed codes).
- `DimRuns` stores column widths and row heights as run-length encoded `(start, end, size, hidden, manual)`. Sizes are stored in **1/100 mm** like ODF and converted to pixels at the API edge.

### 3.3 Value types

```rust
pub enum Value { Empty, Number(f64), Text(Arc<str>), Bool(bool), Error(ErrorKind), Array(Arc<Matrix>) }
pub struct Matrix { pub rows: u32, pub cols: u32, pub data: Vec<Value> } // row-major
```

- Numbers are `f64`, like OpenOffice. Use no decimal type. Equality and comparison use an OOo-style `approx_equal` (relative epsilon about 2^-48, mimicking `rtl::math::approxEqual`). Display rounds to 15 significant digits. A "precision as shown" setting exists, off by default.
- `Bool` stays distinct, because ODF has `office:value-type="boolean"` and OOXML has `t="b"`. It coerces to 1/0 in arithmetic.
- Evaluation-only intermediate: `enum EvalValue { Scalar(Value), Range(ResolvedRange), Array(Arc<Matrix>) }`, so functions can iterate references sparsely without materializing them.

### 3.4 Error values (OpenOffice codes)

```rust
#[repr(u16)]
pub enum ErrorKind {
  InvalidChar = 501, InvalidArgument = 502, Num = 503 /* #NUM! */, ParamList = 504,
  Pair = 507, Bracket = 508, MissingOperator = 509, MissingVariable = 510, MissingVariable2 = 511,
  FormulaOverflow = 512, StringOverflow = 513, InternalOverflow = 514, InternalSyntax = 516..=518/520 (variants),
  Value = 519 /* #VALUE! */, Null = 521 /* #NULL! */, Circular = 522, NoConvergence = 523,
  Ref = 524 /* #REF! */, Name = 525 /* #NAME? */, Div0 = 532 /* #DIV/0! */,
  NA = 32767 /* #N/A, OOo NOTAVAILABLE 0x7fff */,
}
```

- `display()` returns `#NUM!`, `#VALUE!`, `#NULL!`, `#REF!`, `#NAME?`, `#DIV/0!` and `#N/A` for those codes, and `Err:NNN` for the rest.
- `ERROR.TYPE` maps to Excel numbering (1=#NULL!, 2=#DIV/0!, 3=#VALUE!, 4=#REF!, 5=#NAME?, 6=#NUM!, 7=#N/A).
- `ERRORTYPE` (OOo) returns the 5xx code.
- XLSX export maps non-Excel codes to the nearest Excel error (Err:502 becomes #VALUE!, 522 becomes #VALUE! or a cached value).

### 3.5 Formula language (`calc-formula`)

**Hand-written lexer plus Pratt parser.** We need dialect switching, exact error spans mapped to Err:501/508/509, partial parsing for highlighting and autocomplete, and good error recovery, so don't use parser combinators.

**Dialects:**

| | CalcA1 (UI, AOO default) | ExcelA1 (UI option, OOXML storage) | OpenFormula (ODS storage `of:=`) |
|---|---|---|---|
| Sheet ref | `Sheet1.A1`, `$Sheet1.$A$1`, `'My Sheet'.A1` | `Sheet1!A1`, `'My Sheet'!A1` | `[.A1]`, `[$Sheet1.A1:.B2]`, `[.A1:.A3]` |
| Arg separator | `;` (locale-configurable, `,` allowed) | `,` | `;` |
| Array constant | `{1;2|3;4}` (column `;`, row `|`; configurable) | `{1,2;3,4}` | `{1;2|3;4}` |
| Union / intersection | `~` / `!` | `,` / space | `~` / `!` |
| Legacy | | `_xlfn.` / `_xlws.` prefixes | read `oooc:=` (OOo 2.x) |

- Operators, by precedence: `:`; `!` (intersection); `~` (union); prefix `-`/`+`; `%` postfix; `^`; `* /`; `+ -`; `&`; comparisons `= <> < <= > >=`.
- Also parsed: whole-column `A:A`, whole-row `1:1`, named ranges (workbook and sheet scope), `#REF!` literals inside formulas, missing args `=IF(A1;;1)`, and ODF `[.#REF!]`.
- **AST stores relative references** as offsets from the formula's own cell (OOo's `ScSingleRefData` approach):
  ```rust
  pub struct RefCoord { pub v: i32, pub abs: bool }  // abs → absolute index; !abs → offset from origin
  pub struct CellRefExpr { pub sheet: SheetRef, pub row: RefCoord, pub col: RefCoord }
  pub enum SheetRef { Current, Id { id: SheetId, abs: bool }, Unresolved(Arc<str>), Deleted }
  pub enum RefExpr { Cell(CellRefExpr), Area(CellRefExpr, CellRefExpr), Cols{..}, Rows{..}, Invalid }
  pub enum Expr { Num(f64), Str(Arc<str>), Bool(bool), Err(ErrorKind), ArrayLit(Matrix of literal exprs),
      Ref(RefExpr), Name(NameId|Unresolved), Call { func: FuncId|Unknown(Arc<str>), args: Vec<Expr> },
      Unary(UnOp, Box<Expr>), Binary(BinOp, Box<Expr>, Box<Expr>), Postfix(Percent, Box<Expr>),
      Paren(Box<Expr>), Missing }
  ```
  - Copy/paste and fill reuse the same AST; only the origin changes, so relative/absolute behaviour is automatic.
  - Insert/delete rows and columns rewrites ASTs through a single `adjust_refs(op)` visitor. References into deleted areas become `RefExpr::Invalid`, which prints as `#REF!`.
  - Moving cells (cut/paste) needs a separate `move_refs` visitor.
- **Printer:** `print(expr, origin, dialect, resolver)` returns canonical text. Display can optionally prettify (`*` as `×`, `<>` as `≠`) as in the mockup's "=D6 × E6". That must be **display-only and off for editing**; the stored text stays standard.
- `explain(expr, origin, labels)` produces the plain-English hint ("Quantity × Price"). It uses header labels found above or to the left of referenced cells in the current data region.
- **Array formulas:** `{=...}` / Ctrl+Shift+Enter cells are stored as an anchor with an extent; the other cells in the range point to the anchor. Dynamic-array spill (Excel 365 style) is post-1.0 and out of AOO scope.
- **Limits:** nesting depth ≤ 256 (deeper gives Err:512), formula length ≤ 8192 characters, and the parser recursion is guarded to avoid stack overflow on hostile input.

### 3.6 Dependency graph and recalculation

- **Precedents:** each formula cell's compiled formula lists its static references (cells, areas, names).
- **Reverse index (dependents):**
  - single cells go in `HashMap<CellAddr, SmallVec<[FormulaCellId; 2]>>`;
  - area references go in a per-sheet **area-listener index**: per column, an interval structure `(row_start, row_end) → FormulaCellId`, with whole-column references in a separate list;
  - this avoids expanding `A1:A1048576` into a million edges (the same idea as OOo's broadcaster area slots);
  - names keep a name → dependents map, and redefining a name dirties its dependents.
- **On edit:** mark the edited cell, then a BFS through the reverse index sets `state = Dirty` on dependents (formulas already dirty stop the walk). Volatile cells (NOW, TODAY, RAND, RANDBETWEEN, OFFSET, INDIRECT, INFO, CELL with some args) are dirtied on every recalc.
- **Evaluation:** demand-driven over the dirty set, with an **explicit work stack** instead of Rust recursion, so 100k-long chains don't blow the stack. `cell_value(addr)` on a Dirty precedent pushes it; meeting an `Evaluating` cell means a cycle.
- **Cycles:**
  - with iterations off (the default), every cell in the strongly connected component (Tarjan over the dirty subgraph) gets Err:522;
  - with iterations on (OOo defaults: Steps = 100, Minimum change = 0.001), iterate the component until it converges, or give Err:523.
- **Dynamic references** (INDIRECT, OFFSET, INDEX returning references) are recorded at evaluation time as dynamic dependencies, replaced on each evaluation.
- **Modes:** AutoCalculate on/off, F9 (recalc dirty), Ctrl+Shift+F9 (hard recalc: rebuild the graph and recompute everything).
- **On load:** use cached values from the file and offer a "Recalculation on file load: Never / Always / Prompt" setting like LibreOffice (default: recalc if the file came from a different generator, otherwise trust the cache).
- **Threading:** single-threaded in v0.1. The engine lives behind a `Mutex` per document in the Tauri state. From v0.5, run it as an actor thread with progress and cancel for long recalcs; v0.7 adds rayon over independent dirty components. *Built (2026-11-10, docs/recalc-parallel.md):* a parallel pre-pass evaluates dirty formulas on a capped rayon pool in dataflow order (no level barriers; a chain runs as one task) with a read-only context that reads only final values and bails on anything else (RAND, MULTIPLE.OPERATIONS, dirty formulas not yet final); the unchanged sequential pass then takes the pre-computed results, so results, errors, cycles and reports are identical to single-threaded. Feature `parallel` (default on; off for wasm32), app pref "Enable multi-threaded calculation", `TALLY_CALC_THREADING` / `TALLY_CALC_THREADS`.
- **Performance targets** (criterion benchmarks in `calc-engine/benches`):
  - full recalc of 100k simple formulas: under 300 ms;
  - edit with 10k dependents: under 30 ms;
  - SUM over a 1M-row numeric column: under 5 ms;
  - open a 20 MB ODS: under 3 s.

### 3.7 Function registry

Functions depend only on `calc-types` + `calc-numfmt` + a trait. They never depend on the engine.

```rust
pub type EvalFn = fn(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> Value;

pub struct FunctionDef {
    pub name: &'static str,          // canonical English, e.g. "SUMIF"
    pub odf_name: &'static str,      // stored name in ODF, e.g. "SUMIF", "COM.MICROSOFT.IFS", "ORG.OPENOFFICE.WEEKS"
    pub ooxml_name: Option<&'static str>, // e.g. "_xlfn.IFS"
    pub category: Category,          // Database, DateTime, Financial, Information, Logical, Math, Array, Statistical, Spreadsheet, Text, AddIn
    pub min_args: u8, pub max_args: u8, // 255 = variadic
    pub params: &'static [Param],    // last param may be `repeat: true`
    pub returns: ReturnKind,         // Scalar | Array | Reference
    pub flags: FnFlags,              // VOLATILE, LAZY_ARGS, RETURNS_REF, NEEDS_ORIGIN, ARRAY_CONTEXT_FORCED
    pub eval: EvalFn,
    pub help: &'static str,          // short description; long docs in docs/functions
}
pub struct Param { pub name: &'static str, pub kind: ParamKind, pub optional: bool, pub repeat: bool }
pub enum ParamKind {
    Number, Integer, Text, Bool, Any,
    NumberSeq,     // SUM-style: refs/arrays flattened, text & bools inside refs ignored, direct args coerced
    Reference,     // must be a reference (ROW, COLUMN, OFFSET, ISREF, SUMIF ranges)
    Array,         // matrix accepted (MMULT, SUMPRODUCT, ForceArray in OOo terms)
    Criteria,      // COUNTIF-family criterion
    Lazy,          // IF/IFERROR/CHOOSE/IFS/SWITCH branches: evaluated on demand
}

pub trait ArgList {
    fn len(&self) -> usize;
    fn is_missing(&self, i: usize) -> bool;
    fn eval(&mut self, i: usize, ctx: &mut dyn EvalContext) -> EvalValue; // engine evaluates on demand
}

pub trait EvalContext {
    fn origin(&self) -> CellAddr;
    fn settings(&self) -> &CalcSettings;              // epoch, text→number mode, criteria mode, case sensitivity, whole-cell match, iterations
    fn cell_value(&mut self, addr: CellAddr) -> Value; // evaluates dirty precedents
    fn for_each_in_range(&mut self, r: &CellRange, skip_empty: bool,
        f: &mut dyn FnMut(CellAddr, &Value) -> std::ops::ControlFlow<()>);
    fn used_bounds(&self, r: &CellRange) -> Option<CellRange>; // clamp A:A to the used area
    fn parse_reference_text(&self, text: &str, a1: bool) -> Option<CellRange>; // INDIRECT/ADDRESS
    fn record_dynamic_dep(&mut self, r: &CellRange);
    fn now_serial(&self) -> f64;
    fn rand(&mut self) -> f64;
    fn sheet_index(&self, id: SheetId) -> Option<u32>;
    fn sheet_count(&self) -> u32;
    fn cell_format_info(&self, addr: CellAddr) -> CellFormatInfo; // CELL("format"), ISFORMULA, FORMULA()
}
```

Coercion rules live in one place, `calc-functions/src/coerce.rs`, so no function re-implements them:

- **Scalar parameters:** `Empty` becomes 0 or "", `Bool` becomes 1/0, `Text` becomes a number according to `settings.text_to_number`:
  - `GenerateError` (#VALUE!), `TreatAsZero`, `ConvertUnambiguous` (default; ISO dates and plain numbers only), `ConvertLocale`;
  - AOO 4.1 behaviour is closest to `ConvertLocale`. This is an open question (Q3).
- **NumberSeq:** numbers inside references are taken; text, bools and empties inside references are skipped; direct scalar args are coerced; errors propagate immediately.
- **Array lifting is done in the evaluator, not in each function** (OOo "matrix jump" / implicit intersection):
  - a scalar parameter given an array in array-formula context maps elementwise and returns a Matrix;
  - in a normal cell, a range given to a scalar parameter uses implicit intersection with the formula's row or column, otherwise #VALUE!.
- **Criteria** (`criteria.rs`) parse `"<5"`, `">=x"`, `"=abc"`, `"<>Paid"` and bare values. Matching follows `settings.criteria_mode`:
  - `Wildcards` (`*`, `?`, `~`; LibreOffice default), `Regex` (AOO default; use the `regex` crate, which is linear-time and DoS-safe, with documented syntax differences from ICU), or `Literal`;
  - also honours "search criteria = and <> must apply to whole cells" and case sensitivity.
  - It is shared by COUNTIF, SUMIF, AVERAGEIF, COUNTIFS, SUMIFS, AVERAGEIFS, MATCH(type 0), VLOOKUP, HLOOKUP, LOOKUP, SEARCH and the D* database functions.
- **Registration is explicit:** each category module exposes `pub fn register(r: &mut Registry)`, and `calc-functions::registry()` builds a `OnceLock<Registry>`. No `inventory` linker tricks, which keeps it WASM-friendly and deterministic. Lookup is by uppercase canonical name, ODF name and OOXML name. Localized names come later via per-locale tables.
- **Merge-conflict avoidance:** one file per function family (for example `math/rounding.rs` holds ROUND/ROUNDUP/ROUNDDOWN/CEILING/FLOOR/MROUND/TRUNC/INT/EVEN/ODD). Each category's `mod.rs` registration list is owned by that category's engineer.
- **Test vectors:** `tests/conformance/<category>/<FUNC>.toml`:
  ```toml
  [[case]]
  cells = { A1 = 1, A2 = "x", A3 = true, A4 = "=1/0" }
  formula = "=SUM(A1:A3)"
  expect = 1
  [[case]]
  formula = "=ROUND(-2.5;0)"
  expect = -3
  [[case]]
  formula = "=SUM(A1:A4)"
  expect = "#DIV/0!"
  [[case]]
  formula = "=STDEV(1;2;3)"
  expect = 1.0
  tol = 1e-12
  settings = { criteria_mode = "wildcards" }
  ```
  - The runner in `calc-engine/tests/conformance.rs` runs every case end to end (parse, evaluate, compare). Unit tests inside `calc-functions` can use a tiny `MockEvalContext`.
  - **Oracle:** `xtask oracle` writes cases into an ODS, runs `soffice --headless --convert-to` (LibreOffice and/or AOO), and diffs. Known AOO/LibreOffice deviations are recorded per case (`aoo_expect`, `lo_expect`). Primary target: AOO 4.1 behaviour unless it is clearly a bug.
  - `xtask function-coverage` compares the registry to `docs/functions/master-list.toml`, and CI publishes the percentage.

### 3.8 Number formatting (`calc-numfmt`)

- OOo format-code grammar:
  - up to 4 `;`-separated sections, conditions `[<0]`, colours `[RED]`, `[COLOR12]`, currency/locale `[$€-407]`, `[NatNum1]` (parse and ignore initially);
  - `0 # ? , .` digit placeholders, thousands scaling `#,##0,`, percent, scientific `0.00E+00`, fractions `# ?/?` and `# ??/16`, text `@`, literals `"..."` and `\x`, fill `*` and spacing `_`;
  - dates `YY YYYY M MM MMM MMMM D DD NN NNN NNNN Q QQ WW`, times `H HH M MM S SS [HH] [MM] AM/PM 00.000`, era `E`/`G`; `General` and `Standard`.
- **Excel ↔ OOo code translation** for XLSX: `yyyy`, `dddd`, `[h]` and the `General` keyword.
- Format codes are parsed once into a `FormatAst` and cached by `NumFmtId`.
- `format(value, &FormatAst, &Locale, width_hint: Option<u16>) -> Formatted { text, color: Option<Rgb>, align_hint }`.
- **General/Standard depends on column width.** It shows up to about 10 significant digits and shrinks to fit or switches to scientific notation. So `CellView` carries the raw number for General-formatted cells and the grid does the final fit-to-width; fixed formats show `###` when they don't fit.
- **Input recognizer:** `recognize(input, &Locale) -> Recognized { value: Value, implied_format: Option<FormatCode> }`.
  - Typing `15%` gives 0.15 with `0%`; `$1,200.50` gives currency format; `2026-10-19` or `10/19/2026` give a date with format; `TRUE` gives Bool; `'0123` gives text; `=...` is a formula (handled upstream).
- **Dates:** serial `f64`, epoch selectable among **1899-12-30 (default, OOo)**, 1900-01-01 (StarCalc 1.0) and 1904-01-01 (Mac).
  - Own proleptic-Gregorian `days_from_civil` / `civil_from_days`. Use chrono only for the clock and time zone in NOW().
  - Negative serials (pre-1899 dates) are supported, as OOo does.
  - OOo switches to the Julian calendar before 1582-10-15. Decide whether to replicate this (open question Q7).
  - XLSX import: Excel's 1900 leap-year bug means serials ≥ 61 coincide with ours and serials 1–60 shift by one. Handle on import/export and document it.
- **Locale:** v0.1 ships `en-US` only, but the `Locale` struct exists from day one (decimal and grouping separators, list/argument separator, date order, currency, day/month names, function-name table id).
  - Ship a small internal table for about 20 locales in v0.4. Bring in ICU4X (`icu_decimal`, `icu_collator` for sort collation) only when needed.
  - Formulas are always stored canonically (English names, OpenFormula) and rendered per locale.

---

## 4. Frontend design

### 4.1 Data flow and IPC

- **The engine is the single source of truth.** The UI holds a view cache plus UI-only state (selection, scroll, edit session, panel visibility).
- **Viewport tile cache:** tiles of 64 rows × 32 columns keyed by `(sheet, tileRow, tileCol)` with a version.
  - On scroll, the grid requests missing visible tiles plus a one-tile prefetch ring.
  - One `get_cells` call for the visible range is about 2–3k cells, roughly 100–200 KB of JSON, which is fine for Tauri IPC. Batch up to 8 tiles per invoke.
  - If profiling shows a need (v0.4+), move to a binary encoding via `tauri::ipc::Response` raw bytes or `Channel`.
- **Mutations:** `apply(Transaction)` returns an `ApplyResult` listing changed ranges, coalesced into rectangles and including recalculated dependents. The UI invalidates intersecting tiles and refetches visible ones. Changes the UI did not initiate (volatile recalc, async load) arrive via the `doc-changed` event with the same payload.
- **Styles** are fetched by id and cached (`get_styles(ids)`); cells carry only `StyleId`. Formatted display text is computed in Rust, so all formatting logic lives in one place.
- **Layout:** `get_sheet_layout` returns column and row runs. The grid builds prefix-sum arrays (a Fenwick tree for rows) for pixel ↔ index mapping. Hidden (filtered) rows have height 0.

### 4.2 Grid (`@calc/grid`)

- **Layers:**
  - main canvas: fills, text, borders and gridlines, drawn per frame only for visible cells (about 2–3k cells, so a 60 fps budget is easy);
  - overlay canvas: selection, active-cell border, fill handle, reference highlights while editing, copy marching ants;
  - DOM layer: in-cell editor, column/row resize hit zones, tooltips.
- **Headers:** column and row headers are drawn on the same canvas, with frozen panes as clipped sub-viewports (four quadrants).
- Device-pixel-ratio aware, with 0.5 px snapping for crisp 1 px borders.
- **Text:** Inter Variable bundled through `@fontsource-variable/inter`. Wait for `document.fonts.load()` before the first paint. Cache `measureText` widths per (font, string).
  - Text overflows into empty right or left neighbours, so tile fetches include neighbouring columns.
  - Wrap and shrink-to-fit come later.
- **Chips** ("Paid"/"Due"/"Overdue" pills) are a **cell display style**: `StyleDto.display = { kind: "chip", bg, fg }`.
  - They are driven by conditional formatting rules or validation-list item colours (v0.2/v0.3).
  - They persist to ODS as an extension attribute in our own namespace (`calcx:display="chip"`) and degrade to a background fill in XLSX and in other apps.
- **Selection model:**
  ```ts
  { ranges: Range[]; activeRangeIndex: number; active: Cell; anchor: Cell }
  ```
  - Shift extends, Ctrl/Cmd adds ranges, header clicks select whole rows or columns, Ctrl+A selects the current region then everything.
- **Keyboard:**
  - arrows, Tab/Shift+Tab, Enter/Shift+Enter (moving within the selection), Home/End, Ctrl+arrows (jump to data edge using a `next_non_empty` engine query), PageUp/PageDown, Alt+PageUp/PageDown, Ctrl+PageUp/PageDown (sheets);
  - F2 edits, Esc cancels, Delete clears, Ctrl+Z/Y, Cmd mapping on macOS.
  - All bindings are defined in one keymap table in `@calc/app`; the grid exposes actions.
- **Editing and IME:** keep a hidden but focused `<textarea>` positioned at the active cell at all times, so typing and IME composition start naturally. On first input it becomes the visible editor.
  - Formula mode tokenizes with `@calc/formula-ts`, giving coloured references and **point mode** (arrow keys or clicks insert or replace a reference).
  - The TS lexer must pass the same JSON fixtures as the Rust lexer.
  - The formula bar and in-cell editor share one `EditSession` store, so the text stays in sync.
- **Accessibility:** a canvas is invisible to screen readers. Mirror the active cell, selection and headers into an offscreen ARIA grid fragment and announce changes through an `aria-live` region. Basic support in v0.2; a full pass in v0.7.

### 4.3 Clipboard

- Use DOM `copy`/`cut`/`paste` events with `clipboardData`. They work in all three webviews without permission prompts.
- **Copy:** the engine returns `{ tsv, html, token }`. We write `text/plain` (TSV) and `text/html` (a table with inline styles) and embed `data-calc-clip="<token>"` in the HTML.
- **Paste:** if the token matches the engine's last internal copy snapshot, do a high-fidelity internal paste (formulas re-anchored, so relative references adjust, plus styles). Otherwise parse HTML or TSV through the input recognizer.
- Paste Special (values only, formats, transpose, skip empty) comes in v0.2.

### 4.4 Undo/redo: in the engine

- Every mutation is an `Op` inside a `Transaction`. The engine applies it and records inverse ops (or, for heavy operations like sort and delete-rows, a compact snapshot of affected blocks).
- One stack per document, depth 100 (the OOo default). The transaction carries `selection_before` and `selection_after`, so undo restores the cursor.
- Reasons it belongs in the engine: it is the only place that knows recalc side effects and reference adjustments; it is testable without UI; and it is the same op log that autosave and recovery (and possibly collaboration someday) build on.
- UI-only actions (scroll, zoom, panel toggles) are not undoable.

### 4.5 State management

- zustand stores in `@calc/app`:
  - `docStore`: workbook info, sheets, active sheet, version, dirty, undo labels;
  - `selectionStore`;
  - `editStore` (the EditSession);
  - `uiStore`: sidebar, glance panel, palette, theme, zoom;
  - `viewCache`: lives in `@calc/grid` and is not React state.
- All engine access goes through `EngineClient` (from `@calc/bridge`). Components never call `invoke` directly. `MockClient`, an in-memory TS fake with about 20 functions, lets the UI and grid be built and tested before the engine exists.

### 4.6 Theming

- Design tokens as CSS custom properties in `@calc/tokens`: neutral greys, accent, chip palette (green/amber/red with light and dark variants), radii (pill = 999px, card = 10px), spacing scale, Inter font stack, subtle borders (`rgba(0,0,0,0.06)`).
- Light and dark sets switch on `[data-theme]` with `prefers-color-scheme` as the default.
- The grid reads tokens into a typed `Theme` object on theme change, because canvas cannot use CSS variables directly.
- Tailwind 4 is optional for the chrome. Plain CSS modules plus tokens is enough and keeps the dependency count low; I recommend CSS modules.

### 4.7 Command palette (⌘K) and natural-language-to-formula

- UI: cmdk. Sources are merged into one ranked list:
  1. commands from a central `CommandRegistry` (every toolbar and menu action has an id, title, keywords, shortcut and `run()`);
  2. function search (from `list_functions`);
  3. **assist suggestions** from `assist_suggest`.
- **`calc-assist` (Rust, offline, rule-based, the default):**
  - **Context:** active sheet, selection, and the detected **data region** (contiguous block around the selection whose first row is text headers). Columns are typed (number, date, text, low-cardinality category with its distinct values).
  - **Intent grammar** (token patterns plus a synonym lexicon):
    - aggregation: total, sum, add up → SUM; average, mean → AVERAGE; how many, count, number of → COUNT/COUNTA; highest, max, largest → MAX; lowest, min → MIN;
    - condition: "where/with/for <col> is/= <val>", "<col> over/above/greater than <n>", "in <month/year>", "un<value>" / "not <value>" (for example "unpaid" matches the distinct value "Paid" in the Status column, giving `<>Paid`);
    - target column: a fuzzy match (Jaro-Winkler) on headers; otherwise the first numeric column, or "Total"/"Amount" by preference.
  - **Output:** `Suggestion { title, formula, explanation, preview: Value, confidence, kind: InsertFormula | Command(Filter/Chart/Format) }`.
    - Example: "total of unpaid orders" becomes `=SUMIF(D2:D51;"<>Paid";F2:F51)`, explained as "Sum of Total where Status is not Paid", with preview $12,480.
    - When defined names exist, the suggestion uses them (`=SUMIF(Status;"<>Paid";Total)`).
    - The mockup's `"≠Paid"` is **not valid criteria syntax**. Display ≠ only as prettification; insert `<>`.
  - **Every suggestion is validated** by parsing with `calc-formula` and evaluated in a sandbox without committing, to fill the preview, before it is shown.
  - Suggested actions (Filter by Status, Chart Total by Status, Format as currency) map to `CommandRegistry` ids.
- **Optional LLM provider (opt-in only):**
  ```rust
  pub trait AssistProvider: Send + Sync {
      fn id(&self) -> &str;
      fn suggest(&self, req: &AssistRequest) -> Result<Vec<Suggestion>, AssistError>;
  }
  ```
  - `RuleProvider` is always present. `HttpLlmProvider` sits behind a cargo feature `llm` **and** a runtime setting that is off by default.
  - The API key lives in the OS keychain (`keyring` crate).
  - A consent dialog shows exactly what will be sent: headers plus up to N sample rows, never the whole file.
  - The LLM's formulas go through the same parse/validate/preview gate.
  - Network calls come only from Rust; the webview CSP forbids `connect-src` to remote hosts.

---

## 5. File formats and security

### 5.1 ODS (`calc-io-ods`), our own implementation

calamine reads ODS values but does not round-trip styles, merges, column widths, settings or named ranges, so we write our own.

- **Read:** zip → `META-INF/manifest.xml` → `content.xml` (streamed with `quick-xml`), `styles.xml`, `settings.xml` (active sheet, cursor, freeze panes, zoom) and `meta.xml`.
  - Parsed elements: `table:table`, `table:table-row` (with `table:number-rows-repeated`, and **beware huge repeat counts**), `table:table-cell` (`office:value-type`, `office:value`, `office:date-value`, `office:boolean-value`, `table:formula` in `of:=` or legacy `oooc:=`), covered-table-cell and `table:number-columns-spanned` (merges), `table:named-expressions`, `table:content-validations`, `calcext:conditional-formats` and `style:map`, `number:*-style` (converted to format codes), column and row styles.
  - Cached values are used if the recalc-on-load policy says so.
- **Write:** `mimetype` must be the first entry, **stored uncompressed**, then manifest, content, styles, meta, settings. Generate automatic styles from the `StyleTable`. Use run-length `*-repeated` for empty spans.
- Charts (`Object N/content.xml` with `chart:chart`) come in v0.4. Embedded images (`Pictures/`) are kept as opaque blobs.
- **Fidelity tests:** round-trip fixtures (ours → LibreOffice → ours) diffed at the `WorkbookData` level.

### 5.2 XLSX (`calc-io-xlsx`)

- **Write:** `rust_xlsxwriter` (mature, fast, handles styles, merges, freeze panes, validation, conditional formats and charts). Formulas are printed in the ExcelA1/OOXML dialect with `_xlfn.` prefixes.
- **Read:**
  - v0.2: our own quick-xml reader for sheet XML and sharedStrings (values, formulas, merges, column widths), because shared formulas (`t="shared"`) and array formulas need AST-level re-anchoring that calamine's string output doesn't give;
  - v0.3: styles.xml (fonts, fills, borders, numFmts).
  - Alternatively start with calamine for values plus formula strings in v0.2 to save time, then replace it. umya-spreadsheet is a fallback reference but is heavy and historically slow on large files.
- Handle the 1904 date system flag, the 1900 leap-year bug, `#N/A` and other error cells, and booleans.
- XLS (BIFF8): calamine values-only import in v0.4.

### 5.3 CSV (`calc-io-csv`)

- **Import:** the `csv` crate. Encoding detection with `chardetng` + `encoding_rs`. A dialog for separator, quote, encoding, "detect special numbers", and per-column type and locale. Values go through the input recognizer.
- **Export:** separator and quote options; formatted or raw values.
- **Formula-injection guard (default ON):** text cells starting with `= + - @ \t \r` are prefixed with `'`, with a toggle and a documented rationale. Numbers are never altered.
- TSV is the clipboard format.

### 5.4 Security requirements (put these in `docs/security.md` and write tests for each)

1. **Zip bombs:**
   - cap the entry count (10k), per-entry uncompressed bytes (default 512 MB for content.xml, 64 MB for others) and total uncompressed size (2 GB, configurable);
   - **enforce with `Read::take`**, because declared sizes in zip headers can lie;
   - cap the compression ratio (for example 200:1, enforced only above 10 MB);
   - stream XML; never buffer whole archives twice.
2. **XML:** reject any `<!DOCTYPE>` outright (ODF/OOXML never need one). That prevents XXE and billion-laughs attacks. quick-xml does not expand external entities; verify it, unescape only predefined entities, and cap element depth (256) and attribute counts.
3. **Logical bombs:** `table:number-rows-repeated="1048576"` on a non-empty row, or column-repeat × row-repeat explosions. Clamp to sheet limits, store repeated content as runs (never materialize), and cap total materialized cells (for example 50M) with a clear error.
4. **Path traversal:** we never extract to disk. Entries are read by name into memory; names containing `..`, absolute paths or backslashes are ignored. On write, entry names are fixed constants.
5. **Active content:** never execute ODS Basic macros (the `Basic/` directory) or XLSM VBA. Preserve or drop them with a banner. External links, DDE, `WEBSERVICE` and `FILTERXML` are disabled by default (they return an error plus a prompt). `HYPERLINK` opens only on explicit click, through Rust, and only for http/https/mailto.
6. **Untrusted parsing happens in Rust only.** The webview receives typed DTOs and never raw file contents (except images, which are passed as size-limited blobs the webview decodes in its sandbox).
7. **Tauri hardening:**
   - minimal capabilities: no fs or shell plugins exposed to JS, and the dialog plugin is invoked from Rust commands, so JS never receives arbitrary file-read power;
   - only our commands are allowed;
   - strict CSP (`default-src 'self'; connect-src ipc: http://ipc.localhost`);
   - devtools off in release builds.
8. **Engine DoS:** parser depth and length limits; a regex size limit through `RegexBuilder::size_limit`, with no fancy-regex in criteria; a maximum array size (for example 10M elements, else Err:512/#VALUE!); a recalc watchdog that can cancel.
9. **Fuzzing:** cargo-fuzz targets for the formula parser, format-code parser, ODS reader, XLSX reader and CSV reader, run nightly in CI (cargo-fuzz needs a nightly toolchain, which is CI-only here). Malicious sample corpus in `tests/fixtures/malicious/`.

---

## 6. Milestones

Function counts per category are approximate for AOO 4.1; `master-list.toml` is the source of truth.

| Category (AOO name) | Approx. count |
|---|---|
| Mathematical | 70 |
| Statistical | 95 |
| Text | 35 |
| Logical | 8 |
| Date&Time | 25 core + 15 add-in |
| Spreadsheet (Lookup/Ref) | 30 |
| Financial | 50 core + 25 add-in |
| Information | 22 |
| Database | 12 |
| Array | 15 |
| Add-in (Analysis: complex IM*, Bessel, CONVERT, base conversion, EDATE, NETWORKDAYS...) | 70 |
| **Total** | **≈ 420–450** |

### v0.1: demo-able core (about 8 weeks, 5–6 engineers)

- Tauri app builds and runs on macOS arm64 locally; CI builds macOS arm64/x64, Windows x64 and Linux x64 (unsigned).
- Mockup shell: macOS overlay title bar, toolbar pills (Undo, Redo, Bold, Italic, $ Currency, % Percent, Σ Add up; Sort, Filter, Chart and Share present but disabled with a "coming soon" tooltip), name box and formula bar with the plain-English hint, sheet tabs with + / rename / delete / reorder, status bar with Sum/Count/Average and zoom, left sidebar with recent files and two bundled templates (Invoices, Budget) as ODS. Light theme, plus a dark theme if time allows.
- Grid: virtualized canvas over 1,048,576 rows at 60 fps, selection (multi-range), keyboard navigation, in-cell editor and formula bar with reference highlighting and point mode, column/row resize.
- Engine: block or BTreeMap storage behind `ColumnStore`, values, formulas, incremental recalc, cycle detection (Err:522), undo/redo, internal + TSV copy/paste with relative references.
- Formula: CalcA1 + OpenFormula dialects; ExcelA1 parsing for XLSX paste and later.
- **Function batch 0 (about 60):**
  - Math: SUM, PRODUCT, ROUND, ROUNDUP, ROUNDDOWN, INT, TRUNC, ABS, MOD, POWER, SQRT, SIGN, PI, EXP, LN, LOG, LOG10, RAND, RANDBETWEEN, SUMPRODUCT, SUMIF, SUMIFS, COUNTIF, COUNTIFS, CEILING, FLOOR;
  - Statistical: AVERAGE, AVERAGEIF, COUNT, COUNTA, COUNTBLANK, MIN, MAX, MEDIAN;
  - Logical: IF, AND, OR, NOT, TRUE, FALSE, IFERROR, IFNA;
  - Text: CONCATENATE, LEFT, RIGHT, MID, LEN, UPPER, LOWER, PROPER, TRIM, TEXT, VALUE, FIND, SEARCH, SUBSTITUTE, REPT;
  - Date: TODAY, NOW, DATE, YEAR, MONTH, DAY, WEEKDAY, DATEVALUE;
  - Lookup: VLOOKUP, HLOOKUP, INDEX, MATCH, ROW, COLUMN, ROWS, COLUMNS;
  - Information: ISBLANK, ISNUMBER, ISTEXT, ISERROR, ISERR, ISNA, NA, ISLOGICAL.
- Number formats: General/Standard, fixed decimals, thousands, percent, currency, the common date and time codes, and the input recognizer for en-US.
- Files: ODS open/save (values, formulas, bold/italic, number formats, column widths, multiple sheets); CSV import/export with the injection guard.
- ⌘K palette: all commands, plus rule-based assist for "sum/average/count/max/min of <col> [where <col> is/is not <val>]" producing SUM, SUMIF, COUNTIF and AVERAGEIF.
- Insert/delete rows and columns **with reference adjustment** (essential for a credible demo; stretch goal, else first item of v0.2).

### v0.2: everyday spreadsheet

Parallel tracks:

- **Engine:** insert/delete reference adjustment (if not done), merge cells, freeze panes, named ranges (Insert Names / Define dialog), sort (multi-key, header detection), AutoFilter (hidden rows plus filter UI), find & replace (values/formulas, regex, whole cell), move/cut-paste reference adjustment.
- **UI:** fill handle (series detection: numbers, dates, weekday and month lists, copy formulas), paste special, borders/fill/font colour/alignment/wrap pickers, chips via a simple "colour by value" rule, dark theme, the ARIA shadow grid, the WASM harness and Playwright.
- **IO:** XLSX import (values, formulas incl. shared/array, merges, widths) and export via rust_xlsxwriter.
- **Functions batch 1:** finish Logical and Information, finish Text (about 35: CHAR, CODE, CLEAN, EXACT, FIXED, DOLLAR, T, LEFTB..., REPLACE, TEXTJOIN-equivalents where AOO has them, UNICHAR...), Math to about 80% (SUMSQ, GCD, LCM, FACT, COMBIN, MROUND, ODD, EVEN, QUOTIENT, trigonometric/hyperbolic, DEGREES/RADIANS, SUBTOTAL, AGGREGATE if targeted).

### v0.3: data quality and analysis

- Conditional formatting (value rules, colour scales, data bars, icon sets; chips as a CF style).
- Data validation (lists rendered as chips with a dropdown, numeric/date rules, input help, error alerts).
- Array formulas (Ctrl+Shift+Enter, MMULT, TRANSPOSE, MINVERSE, MDETERM, FREQUENCY, LINEST and the other Array-category functions).
- "At a glance" right panel: KPIs (row count, sums of numeric columns, count by status) plus a small SVG bar chart.
- Full format dialog with the custom format-code editor and preview; cell styles (Default, Heading, Result...).
- **Functions batch 2:** Date&Time complete (incl. the add-in: EDATE, EOMONTH, NETWORKDAYS, WORKDAY, WEEKNUM, YEARFRAC, DATEDIF, DAYS360, EASTERSUNDAY, ISLEAPYEAR, WEEKS, MONTHS, YEARS...); Spreadsheet/Lookup complete (OFFSET, INDIRECT, ADDRESS, CHOOSE, LOOKUP, AREAS, HYPERLINK, GETPIVOTDATA stub, SHEET, SHEETS, STYLE, DDE disabled, ERRORTYPE...); Statistical batch A (descriptive: STDEV*, VAR*, LARGE, SMALL, RANK, PERCENTILE, QUARTILE, MODE, AVEDEV, DEVSQ, GEOMEAN, HARMEAN, TRIMMEAN, CORREL, COVAR, FORECAST, SLOPE, INTERCEPT, RSQ, STEYX, PEARSON).
- XLSX styles import. Custom Windows title bar.

### v0.4: charts, finance, locales

- **Charts:** bar, column, line, area, pie, scatter and combo; ODS chart object read/write; XLSX chart export via rust_xlsxwriter. Rendering choice: echarts 6 (Apache-2.0, about 1 MB, tree-shakable) for full charts, our own SVG renderer for the glance panel.
- **Functions batch 3:** Financial complete (PV, FV, PMT, NPER, RATE, IPMT, PPMT, NPV, XNPV, IRR, XIRR, MIRR, SLN, SYD, DB, DDB, VDB, CUMIPMT, CUMPRINC, EFFECT, NOMINAL, bond functions PRICE, YIELD, DURATION, MDURATION, ACCRINT, COUP*, TBILL*, DISC, INTRATE, RECEIVED...), porting AOO algorithms with attribution. Statistical batch B (distributions: NORMDIST, NORMINV, NORMSDIST, NORMSINV, TDIST, TINV, CHIDIST, CHIINV, FDIST, FINV, BETADIST, BETAINV, GAMMADIST, GAMMAINV, GAMMALN, BINOMDIST, POISSON, EXPONDIST, WEIBULL, LOGNORMDIST, LOGINV, HYPGEOMDIST, NEGBINOMDIST, CRITBINOM, TTEST, FTEST, CHITEST, ZTEST, CONFIDENCE, plus the OOo-specific B, GAUSS, PHI...). Use statrs only for special functions where precision matches; prefer porting AOO's own implementations so results match bit for bit.
- Database functions (DSUM, DCOUNT, DCOUNTA, DAVERAGE, DGET, DMAX, DMIN, DPRODUCT, DSTDEV, DSTDEVP, DVAR, DVARP).
- Iterative calculation; function wizard and autocomplete; localization (de, fr, es, ja: separators, function names, UI strings); XLS import.

### v0.5: DataPilot and advanced data

- Pivot tables (DataPilot): engine `pivot` module, field layout UI, ODS `table:data-pilot-tables` read/write, XLSX pivot export best effort.
- Standard filter, advanced filter, subtotals, consolidate, text-to-columns, remove duplicates (a modern extra), group and outline.
- **Functions batch 4:** the Add-in category (complex IM* family, BESSEL*, CONVERT, BIN2DEC/HEX/OCT etc., ERF, ERFC, DELTA, GESTEP, FACTDOUBLE, SERIESSUM, MULTINOMIAL, SQRTPI...) and the remaining Math/Statistical (GROWTH, TREND, LOGEST, PROB, SKEW, KURT, STANDARDIZE, PERMUT, PERMUTATIONA, ...).
- The engine moves to an actor thread with cancellable recalc.

### v0.6: document features

- Print and page setup (page styles, headers/footers, print ranges, page breaks, scaling), PDF export (Rust PDF writer, or the webview print path for v0.6 then native later).
- Comments/notes, hyperlinks, images and shapes (basic), sheet and cell protection, goal seek, scenarios, autosave and crash recovery from the op log.

### v0.7 to v1.0: hardening and parity

- Multithreaded recalc (done 2026-11-10, docs/recalc-parallel.md), large-file streaming (100 MB ODS), memory optimisation (block storage everywhere).
- Full accessibility pass, code signing and notarization, auto-update (tauri-plugin-updater), crash reporting (opt-in).
- Solver (linear first).
- **v1.0 exit criteria:**
  - 100% of the master list implemented, or explicitly marked unsupported (DDE, macro-only functions);
  - ≥ 99% of conformance cases pass against the AOO oracle, with documented deviations;
  - round-trip fidelity tests on a 200-file real-world ODS/XLSX corpus;
  - performance targets met on all three OSes.

**Parallelism after v0.1:** function batches are embarrassingly parallel by category, with one engineer per category and disjoint files and TOML vectors. Charts, pivot, IO-XLSX, CF/validation and print are mostly independent tracks once the engine's op and query APIs exist.

---

## 7. v0.1 work packages

### WP0: Contracts and scaffolding (lead, days 1–3, blocks everyone)

- **Scope:** Cargo workspace and pnpm workspace; empty crates and packages with the module skeletons below; `calc-types` and the `calc-api` DTOs (the types below, compiled and exported to TS via ts-rs into `packages/bridge/src/generated/`); the `EngineClient` TS interface; `MockClient` stub; ADR-0001; CI `ci.yml`; `docs/functions/master-list.toml` seeded from AOO's function list.
- **Owns:** root configs, `crates/calc-types`, `crates/calc-api/src/dto.rs`, `packages/bridge/src/{client.ts,generated/}`, `.github/`, `docs/adr/`.
- **Freeze rule:** after day 3, changes to shared contracts go through a lead-reviewed PR. Additive changes are okay; breaking changes need everyone's sign-off.

**Shared Rust contracts (`calc-types`):**

```rust
pub type SheetId = u32;
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Serialize, Deserialize)]
pub struct CellAddr { pub sheet: SheetId, pub row: u32, pub col: u32 }
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Serialize, Deserialize)]
pub struct CellRange { pub sheet: SheetId, pub r0: u32, pub c0: u32, pub r1: u32, pub c1: u32 } // inclusive
pub const MAX_ROWS: u32 = 1_048_576; pub const MAX_COLS: u32 = 16_384;
pub enum Value { Empty, Number(f64), Text(Arc<str>), Bool(bool), Error(ErrorKind), Array(Arc<Matrix>) }
pub enum ErrorKind { /* see 3.4 */ }
impl ErrorKind { pub fn code(self) -> u16; pub fn display(self) -> Cow<'static, str>; pub fn parse(s: &str) -> Option<Self>; }
pub struct CalcSettings { pub date_epoch: DateEpoch, pub text_to_number: TextToNumber, pub criteria_mode: CriteriaMode,
  pub case_sensitive: bool, pub whole_cell_match: bool, pub iterations: Option<IterSettings>, pub precision_as_shown: bool }
pub fn col_to_letters(col: u32) -> String; pub fn letters_to_col(s: &str) -> Option<u32>;
```

**Interchange model (`calc-model`, also WP0, used by WP1 and WP5):**

```rust
pub struct WorkbookData { pub sheets: Vec<SheetData>, pub styles: Vec<StyleData>, pub names: Vec<NameData>, pub settings: DocSettings, pub meta: DocMeta }
pub struct SheetData { pub name: String, pub cells: Vec<CellData> /* row-major sorted */, pub col_widths: Vec<DimRun>, pub row_heights: Vec<DimRun>,
  pub merges: Vec<CellRangeLocal>, pub frozen: Option<(u32,u32)>, pub tab_color: Option<u32>, pub hidden: bool }
pub struct CellData { pub row: u32, pub col: u32, pub content: CellContentData, pub style: Option<u32> }
pub enum CellContentData { Value(Value), Formula { text: String /* OpenFormula dialect, e.g. "of:=SUM([.A1:.A3])" */, cached: Option<Value> } }
pub struct StyleData { pub font: FontData, pub fill: Option<u32 /* rgba */>, pub align: AlignData, pub borders: BordersData, pub number_format: Option<String /* OOo format code */>, pub display: Option<DisplayData> }
pub struct DimRun { pub start: u32, pub end: u32, pub size_hmm: u32 /* 1/100 mm */, pub hidden: bool }
```

**API DTOs (`calc-api`, exported to TS by ts-rs):**

```rust
pub struct DocId(pub u32);
pub struct WorkbookInfo { pub doc: DocId, pub title: String, pub path: Option<String>, pub sheets: Vec<SheetInfo>, pub active_sheet: SheetId, pub dirty: bool, pub version: u64 }
pub struct SheetInfo { pub id: SheetId, pub name: String, pub tab_color: Option<String>, pub hidden: bool }
pub struct SheetLayout { pub sheet: SheetId, pub used: Option<RangeDto>, pub default_col_width_px: f32, pub default_row_height_px: f32,
  pub col_runs: Vec<DimRunPx>, pub row_runs: Vec<DimRunPx>, pub frozen: Option<CellPos>, pub merges: Vec<RangeDto> }
pub struct DimRunPx { pub start: u32, pub end: u32, pub size: f32, pub hidden: bool }
pub struct RangeDto { pub r0: u32, pub c0: u32, pub r1: u32, pub c1: u32 }
pub struct CellPos { pub r: u32, pub c: u32 }
pub struct CellBlock { pub sheet: SheetId, pub range: RangeDto, pub version: u64, pub cells: Vec<CellView> } // only non-empty or styled cells
pub struct CellView { pub r: u32, pub c: u32, pub text: String, pub kind: ValueKind, pub num: Option<f64> /* for General fit-to-width */,
  pub style: u32, pub color: Option<u32> /* from [RED] etc. */, pub flags: u8 /* 1=formula 2=error 4=array 8=comment */ }
pub enum ValueKind { Empty, Number, Text, Bool, Error }
pub struct StyleDto { pub id: u32, pub bold: bool, pub italic: bool, pub underline: bool, pub strike: bool, pub font_family: Option<String>, pub font_size_pt: f32,
  pub color: Option<u32>, pub fill: Option<u32>, pub h_align: HAlign, pub v_align: VAlign, pub wrap: bool, pub indent: u8,
  pub borders: BordersDto, pub number_format: String, pub display: Option<DisplayDto /* Chip{bg,fg} */> }
pub enum Op {
  SetCellInput { sheet: SheetId, r: u32, c: u32, input: String },        // raw user text; engine recognizes formula/number/date
  SetRangeInputs { sheet: SheetId, r: u32, c: u32, rows: Vec<Vec<String>> },
  Clear { sheet: SheetId, ranges: Vec<RangeDto>, what: ClearWhat /* Contents|Formats|All */ },
  ApplyStyle { sheet: SheetId, ranges: Vec<RangeDto>, patch: StylePatch },  // all fields Option<..>; toggle semantics resolved by UI
  SetNumberFormat { sheet: SheetId, ranges: Vec<RangeDto>, code: String },
  InsertRows { sheet: SheetId, at: u32, count: u32 }, DeleteRows { .. }, InsertCols { .. }, DeleteCols { .. },
  SetColWidth { sheet: SheetId, start: u32, end: u32, px: f32 }, SetRowHeight { .. },
  AddSheet { name: Option<String>, index: Option<u32> }, RenameSheet { sheet: SheetId, name: String },
  DeleteSheet { sheet: SheetId }, MoveSheet { sheet: SheetId, to: u32 },
  Paste { sheet: SheetId, target: RangeDto, payload: PastePayload },     // Internal{token} | Tsv(String) | Html(String)
}
pub struct Transaction { pub ops: Vec<Op>, pub label: Option<String>, pub selection_before: Option<SelectionDto>, pub selection_after: Option<SelectionDto> }
pub struct ApplyResult { pub version: u64, pub changed: Vec<SheetRanges>, pub layout_changed: Vec<SheetId>, pub sheets_changed: bool,
  pub errors: Vec<OpError>, pub undo: UndoState, pub selection: Option<SelectionDto>, pub info: Option<WorkbookInfo> /* when sheets_changed */ }
pub struct UndoState { pub can_undo: bool, pub can_redo: bool, pub undo_label: Option<String>, pub redo_label: Option<String> }
pub struct SelectionDto { pub sheet: SheetId, pub ranges: Vec<RangeDto>, pub active: CellPos }
pub struct SelectionStats { pub sum: Option<f64>, pub count: u64, pub count_numbers: u64, pub average: Option<f64>, pub min: Option<f64>, pub max: Option<f64>, pub formatted_sum: Option<String> }
pub struct CellEditInfo { pub input: String /* text to show in the editor, formula in UI dialect */, pub hint: Option<String> /* plain-English */ }
pub struct FunctionInfo { pub name: String, pub category: String, pub signature: String, pub description: String }
pub struct Suggestion { pub title: String, pub formula: Option<String>, pub explanation: String, pub preview: Option<String>, pub confidence: f32, pub action: Option<String> /* command id */ }
pub struct ClipboardOut { pub tsv: String, pub html: String, pub token: String }
pub enum FileFormat { Ods, Xlsx, Csv }
```

**DocumentService (Rust facade in `calc-api`, implemented by WP1, wrapped by WP7):**

```rust
pub trait DocumentService {
  fn new_workbook(&mut self, template: Option<&str>) -> Result<WorkbookInfo>;
  fn open(&mut self, path: &Path) -> Result<WorkbookInfo>;
  fn save(&mut self, doc: DocId, path: Option<&Path>, format: FileFormat) -> Result<WorkbookInfo>;
  fn close(&mut self, doc: DocId);
  fn sheet_layout(&self, doc: DocId, sheet: SheetId) -> Result<SheetLayout>;
  fn cells(&self, doc: DocId, sheet: SheetId, ranges: &[RangeDto]) -> Result<Vec<CellBlock>>;
  fn styles(&self, doc: DocId, ids: &[u32]) -> Result<Vec<StyleDto>>;
  fn cell_edit_info(&self, doc: DocId, sheet: SheetId, r: u32, c: u32) -> Result<CellEditInfo>;
  fn apply(&mut self, doc: DocId, tx: Transaction) -> Result<ApplyResult>;
  fn undo(&mut self, doc: DocId) -> Result<ApplyResult>; fn redo(&mut self, doc: DocId) -> Result<ApplyResult>;
  fn selection_stats(&self, doc: DocId, sel: &SelectionDto) -> Result<SelectionStats>;
  fn copy(&mut self, doc: DocId, sel: &SelectionDto) -> Result<ClipboardOut>;
  fn navigate_edge(&self, doc: DocId, sheet: SheetId, from: CellPos, dir: Direction) -> Result<CellPos>; // Ctrl+arrow
  fn resolve_name_box(&self, doc: DocId, text: &str) -> Result<SelectionDto>;  // "B12", "A1:C5", "Sheet2.A1", named range
  fn list_functions(&self) -> Vec<FunctionInfo>;
  fn assist_suggest(&self, doc: DocId, query: &str, sel: &SelectionDto) -> Result<Vec<Suggestion>>;
}
```

**Tauri commands** (snake_case names; each is a 1:1 async wrapper over `DocumentService`; errors are `{ code: string, message: string }`):

`wb_new(template?)`, `wb_open_dialog()` (Rust shows the dialog and returns `WorkbookInfo | null`), `wb_open(path)`, `wb_save(doc, saveAs: bool, format?)`, `wb_close(doc)`, `sheet_layout(doc, sheet)`, `get_cells(doc, sheet, ranges)`, `get_styles(doc, ids)`, `cell_edit_info(doc, sheet, r, c)`, `apply(doc, tx)`, `undo(doc)`, `redo(doc)`, `selection_stats(doc, sel)`, `clipboard_copy(doc, sel)`, `navigate_edge(doc, sheet, from, dir)`, `resolve_name_box(doc, text)`, `list_functions()`, `assist_suggest(doc, query, sel)`, `recent_files()`, `list_templates()`.

Events: `doc-changed` (payload `{ doc, result: ApplyResult }`), `menu-action` (payload `{ id: string }`).

**TS `EngineClient`** (`@calc/bridge`): the same methods camelCased, returning Promises of the generated types. Implemented by `TauriClient` (WP7) and `MockClient` (WP0 stub, filled out by WP6/WP7 as needed).

---

### WP1: Engine core (`crates/calc-engine`, plus the `DocumentService` impl in `crates/calc-api/src/service.rs`)

- **Scope:** workbook, sheets, `ColumnStore` (BTreeMap impl), string pool, style table, formula cell slotmap, dependency index (cell map + area listeners), dirty propagation, stack-based evaluator (operators, implicit intersection, array lifting, lazy args, ArgList/EvalContext impl), cycle detection, ops/transactions/undo, insert/delete reference adjustment (stretch), clipboard internal snapshot/paste, selection stats, navigate_edge, `Workbook::from_data`/`to_data`.
- **Consumes:** `calc-formula` (parse/print/adjust), `calc-functions` (registry), `calc-numfmt` (format/recognize), `calc-model`.
- **Until those land:** stub with `calc-formula` returning a trivial AST for `=A1+B1` and a hand-registered SUM. Work behind the trait boundaries.
- **Acceptance:**
  - unit tests for storage (set/get/iterate 1M rows), dirty propagation, diamond dependencies, chains of 100k without stack overflow, cycles giving Err:522 on all members, undo/redo round trip for every Op (proptest: random op sequences followed by undo-all equal the initial state);
  - the conformance runner executes `tests/conformance/**/*.toml`;
  - criterion benchmarks: 100k-formula recalc under 300 ms on an M-series Mac; SUM(A1:A1000000) under 10 ms (BTreeMap impl allowed 3x slack in v0.1).

### WP2: Formula language (`crates/calc-formula`, `crates/calc-formula/tests/fixtures/tokens/*.json`)

- **Scope:** lexer, Pratt parser, AST (section 3.5), dialects CalcA1/OpenFormula/ExcelA1, printer, `adjust_refs(op)` / `move_refs`, `tokenize()` for highlighting, `explain()`, display prettifier.
- **Exposes:**
  ```rust
  parse(src, origin, dialect, &dyn Resolver) -> Result<Formula, ParseError{kind: ErrorKind, span}>
  print(&Formula, origin, dialect, &dyn Resolver) -> String
  tokenize(src, dialect) -> Vec<Token{kind, span}>
  adjust_for_structure_change(&mut Formula, origin, &StructureChange) -> bool
  explain(&Formula, origin, &dyn LabelSource) -> Option<String>
  trait Resolver { fn sheet_by_name(&self, &str) -> Option<SheetId>; fn sheet_name(&self, SheetId) -> Option<Arc<str>>; fn name(&self, &str, SheetId) -> Option<NameId>; fn function(&self, &str) -> Option<FuncId>; }
  ```
  Function ids are resolved through the trait, so WP2 does not depend on WP3.
- **Acceptance:**
  - insta snapshots of ASTs for about 300 formulas;
  - round trips: CalcA1 → AST → CalcA1 is identity (modulo whitespace), and CalcA1 ↔ OpenFormula ↔ ExcelA1 preserves semantics;
  - relative copy test (`=A1+$B$1` copied from C1 to D5 prints `=B5+$B$1`);
  - insert-row adjustment cases including `#REF!`;
  - Err:501/508/509 classification for malformed input;
  - a fuzz target that never panics and stays within depth limits;
  - shared token fixtures published (at least 150).

### WP3: Function library batch 0 (`crates/calc-functions`, `tests/conformance/**`)

- **Scope:** `registry.rs`, `coerce.rs`, `criteria.rs`, `args.rs` helpers, the about 60 v0.1 functions listed in v0.1, `MockEvalContext` for unit tests, and `docs/contributing-functions.md` (a template showing how to add a function in about 30 lines including tests).
- **Consumes:** `EvalContext` / `ArgList` traits (defined in calc-functions itself and implemented by WP1), and `calc-numfmt` for TEXT/VALUE/DATEVALUE (until WP4 lands, `TEXT` uses a stub).
- **Acceptance:**
  - every function has a TOML vector file with at least 8 cases, including error propagation, empty cells, text in ranges, bools, and missing optional arguments;
  - criteria tests across all three modes;
  - `xtask function-coverage` reports batch 0 at 100%;
  - oracle diff against LibreOffice headless run once and recorded.

### WP4: Number formats, input recognition, dates (`crates/calc-numfmt`)

- **Scope:** format-code parser and formatter (section 3.8; v0.1 must cover General/Standard, `0`, `0.00`, `#,##0`, `#,##0.00`, `0%`, `0.00%`, currency `[$$-409]#,##0.00` and `$#,##0.00;[RED]-$#,##0.00`, `0.00E+00`, `@`, `MM/DD/YYYY`, `YYYY-MM-DD`, `DD MMM YYYY`, `HH:MM`, `HH:MM:SS`, `[HH]:MM`, AM/PM, and 2–4 sections with colours and conditions); input recognizer; date serial conversions with all three epochs; the `Locale` struct with en-US data; Excel ↔ OOo format-code translation (can slip to v0.2).
- **Exposes:**
  ```rust
  parse_format(code) -> Result<FormatAst>
  format_value(&Value, &FormatAst, &Locale, width_chars: Option<u16>) -> Formatted
  recognize_input(&str, &Locale) -> Recognized
  serial_from_ymd / ymd_from_serial / serial_from_hms / hms_from_serial(epoch)
  builtin_format(kind: QuickFormat::{Currency, Percent, Date, ...}, &Locale) -> String
  ```
- **Acceptance:**
  - a table-driven test file with at least 400 (value, code, expected) cases, cross-checked against LibreOffice TEXT() output;
  - recognizer covering at least 100 inputs;
  - date round trip for every day between 1583 and 9999 (proptest);
  - the format parser fuzz target.

### WP5: File IO, ODS and CSV (`crates/calc-io-ods`, `crates/calc-io-csv`, `crates/calc-model` fields as needed through a lead-reviewed PR, `tests/fixtures/{ods,csv,malicious}`)

- **Scope:** ODS read/write to and from `WorkbookData` (values, formulas as OpenFormula text, cached values, cell styles: bold, italic, number format, fill, alignment; column widths and row heights; multiple sheets; merges read-only; active sheet and cursor via settings.xml); all the security limits in section 5.4; CSV import/export with options and the injection guard.
- **Exposes:**
  ```rust
  read_ods(&mut impl Read+Seek, &Limits) -> Result<WorkbookData>
  write_ods(&WorkbookData, &mut impl Write+Seek) -> Result<()>
  read_csv(bytes, &CsvOptions) -> Result<SheetData>   // raw strings; the engine runs the recognizer
  write_csv(&SheetData-or-iterator, &CsvOptions, &mut impl Write)
  ```
- **Acceptance:**
  - opens 10 real ODS files saved by AOO 4.1 and LibreOffice 25.x without loss of supported features;
  - our output opens cleanly in LibreOffice (CI check with `soffice --headless --convert-to csv` and value comparison, Linux job);
  - mimetype is the first stored entry;
  - malicious corpus (zip bomb, DOCTYPE/XXE, billion laughs, `..` paths, repeated-rows bomb, deep nesting) is rejected within 1 s and 100 MB RSS;
  - fuzz target in place.

### WP6: Canvas grid (`packages/grid`, `packages/formula-ts`, `packages/tokens`)

- **Scope:** renderer (cells, gridlines, headers, selection overlay, active cell, reference highlights), virtualization with prefix sums, tile cache with request batching, hit testing, selection model and keyboard navigation actions, column/row resize, in-cell editor host (hidden textarea with IME), zoom, DPR, theme tokens, React wrapper `<Grid/>`; also the TS formula lexer passing the shared fixtures.
- **Interfaces:**
  ```ts
  interface GridDataSource {
    getLayout(sheet: number): SheetLayout;
    getCell(r: number, c: number): CellView | undefined;
    getStyle(id: number): StyleDto | undefined;
    ensureRanges(ranges: RangeDto[]): void;
    subscribe(fn: (dirty: RangeDto[] | 'all') => void): () => void;
  }
  interface GridProps {
    source: GridDataSource; theme: Theme; zoom: number; selection: Selection; edit: EditState | null;
    onSelectionChange(sel: Selection): void;
    onStartEdit(cell: CellPos, seed?: string): void;
    onEditChange(text: string, caret: number): void;
    onCommitEdit(move: 'down'|'up'|'right'|'left'|'none'): void;
    onCancelEdit(): void;
    onResize(axis: 'col'|'row', start: number, end: number, px: number): void;
    onNavigateEdge(from: CellPos, dir: Dir): Promise<CellPos>;
    onContextMenu(cell: CellPos, x: number, y: number): void;
  }
  ```
  - `CellView`, `StyleDto`, `SheetLayout` and `RangeDto` come from the generated bridge types.
  - Develop against `MockClient`-backed and synthetic sources (1M rows of generated data).
- **Acceptance:**
  - Playwright (Chromium plus WebKit) perf test: a scroll fling across 1M rows keeps p95 frame time under 16 ms on an M-series Mac;
  - vitest for prefix-sum mapping, selection logic (shift/ctrl/headers) and keyboard actions;
  - visual snapshot tests for gridlines, text alignment, overflow, chip rendering and dark theme;
  - IME composition test (Japanese input) in WebKit;
  - TS lexer passes 100% of the shared fixtures.

### WP7: App shell, chrome and Tauri integration (`apps/desktop/**`, `packages/app/**`, `packages/bridge/src/{tauri.ts,mock.ts}`)

- **Scope:**
  - Tauri 2.12 app: window config (overlay title bar on macOS), capabilities, commands wrapping `DocumentService` (with a `Mutex<ServiceImpl>` in managed state), dialogs, recent files, bundled templates, native menu with accelerators, `doc-changed` events;
  - React chrome: toolbar pills wired to Transactions (Bold/Italic toggles produce `ApplyStyle`; $ and % produce `SetNumberFormat` from `builtin_format`; Σ Add up inserts `=SUM(<auto-detected range>)`), name box (via `resolve_name_box`), formula bar with hint (`cell_edit_info`), sheet tabs (add/rename/delete/reorder), status bar (`selection_stats`, debounced 50 ms) and zoom, left sidebar, ⌘K palette (cmdk + `CommandRegistry` + `assist_suggest` with an Insert action), clipboard event handling, keymap, stores, theme switch.
- **Consumes:** `DocumentService` (WP1), the Grid (WP6), `assist_suggest` (WP8). Builds against `MockClient` until WP1 is ready.
- **Acceptance:**
  - `pnpm tauri dev` runs on macOS arm64; CI bundle jobs produce mac, Windows and Linux artifacts;
  - Playwright e2e on the web build with MockClient covers: type values and a formula, see the result, Bold, undo, add a sheet, rename it, ⌘K "sum of Total" inserts a formula;
  - manual test checklist on all three OSes documented in `docs/qa/v0.1.md`;
  - lighthouse-style check that no network requests leave the app.

### WP8: Assist (`crates/calc-assist`), small; can go to the WP2 engineer from week 4

- **Scope:** data region and header detection over a read-only `SheetView` trait (`fn value(&self, r, c) -> Value; fn used(&self) -> Range`), the intent lexicon and rules from section 4.7, candidate generation, validation through `calc-formula::parse`, preview through a callback `Fn(&str) -> Value` supplied by WP1, the `AssistProvider` trait, `RuleProvider`.
- **Acceptance:** a golden test table of at least 60 (sheet fixture, query) → expected formula cases, for example:
  - "total of unpaid orders" gives `=SUMIF(D2:D51;"<>Paid";F2:F51)`;
  - "average price" gives `=AVERAGE(E2:E51)`;
  - "how many overdue" gives `=COUNTIF(D2:D51;"Overdue")`;
  - "max quantity in March" gives the MAXIFS-equivalent or `SUMPRODUCT(MAX(...))` per AOO capability (note: AOO 4.1 lacks MAXIFS);
  - zero network dependencies (`cargo tree` check in CI).

### Staffing

- **6 engineers:** WP1, WP2 (then WP8), WP3, WP4, WP5, WP6; WP7 is shared by the lead and the WP6 engineer after week 3, or a 7th person.
- **4 engineers:** (WP1), (WP2 + WP8 + WP4), (WP3 + WP5), (WP6 + WP7), with the lead doing WP0 and reviews.
- **Integration checkpoints:** end of week 2 (parser + evaluator + SUM end to end in the CLI); week 4 (Tauri app showing a live engine grid); week 6 (ODS round trip in the app, feature freeze); weeks 7–8 (polish, cross-OS QA).

---

## 8. Risks and open questions

### Risks

1. **Webview divergence (Tauri):**
   - Linux WebKitGTK canvas performance, GPU compositing bugs on some drivers, and IME differences;
   - font rendering differs per OS, so measurement must be runtime-based.
   - Mitigation: e2e on all three, the engine kept transport-agnostic (Electron fallback), test `WEBKIT_DISABLE_DMABUF_RENDERER` workarounds.
2. **Tauri 3 alpha:** ecosystem churn. Pin 2.x and track 3.0 stable; migrate after it is stable for 3+ months.
3. **Local toolchain:** no rustup means no x86_64 or universal mac builds locally, and no nightly for cargo-fuzz. Recommend installing rustup (it can coexist; put `~/.cargo/bin` ahead of Homebrew in PATH) or rely on CI. Also fix the root-owned `~/.npm` cache.
4. **Semantic fidelity:** AOO vs LibreOffice vs Excel differ (criteria regex vs wildcard, text-to-number conversion, empty-string handling, ROUND edge cases, financial function convergence). Mitigation: settings flags, the oracle harness, documented deviations.
5. **Floating-point display parity:** 15-digit rounding and `approxEqual` behaviours must match OOo or users see "0.30000000000000004". Port `rtl::math::round` and `approxEqual` semantics early (WP4/WP1).
6. **Reference adjustment complexity:** insert/delete/move with ranges partially overlapping, names, conditional formats, validation and charts all need adjusting. Centralize in one visitor and one `StructureChange` type; property-test against a naive model.
7. **The dependency-graph area index** is the hardest data structure here. Prototype it in week 1; benchmark whole-column references (`SUM(A:A)` in 10k cells).
8. **ODS fidelity scope creep:** styles.xml inheritance and automatic styles are large. Define a "supported subset" and preserve unknown XML where feasible (for example unknown settings) rather than dropping it.
9. **Accessibility** with a canvas grid is a known weak point; budget it explicitly (v0.2 basic, v0.7 full).
10. **Function count realism:** about 430 functions × a few hours each with tests is roughly 1.5–2 engineer-years. The batches let it parallelize, but statistical distributions and bond functions are precision-sensitive; port the AOO code.
11. **The mockup's non-standard syntax** (`=D6 × E6`, `"≠Paid"`, header-name references like `Status`): prettification must never leak into stored formulas. Header-name references require named ranges, or AOO's "automatically find column and row labels" feature; decide which (Q4).
12. **Trademark and legal:** avoid "OpenOffice"/"Calc" branding; Apache-2.0 attribution for ported code; no MPL code copied.
13. **Licence hygiene of dependencies:** echarts (Apache-2.0) and calamine (MIT) are fine; enforce with cargo-deny and a license-checker for npm.

### Open questions for the owner

- **Q1. Behaviour baseline:** Apache OpenOffice 4.1.x exactly, or "OpenOffice-compatible but follow LibreOffice/Excel where they're saner"? This affects the defaults for criteria mode (regex in AOO vs wildcards) and argument separators (`;` vs `,` in en-US).
- **Q2. Sheet size:** 1,048,576 × 16,384 (XLSX-compatible, recommended) or AOO's 1,048,576 × 1,024?
- **Q3. Text-to-number conversion default** ("Convert unambiguous only", recommended, vs AOO-like locale conversion).
- **Q4. Mockup-style header references** (`SUMIF(Status; ...; Total)`): auto-create named ranges for table headers, implement AOO label recognition, or introduce Excel-like structured tables later?
- **Q5. Share button:** what does it do in v0.1 with no cloud (export a PDF/ODS and open the OS share sheet)? Any intent to add collaboration later? That would push toward a CRDT-friendly op log now.
- **Q6. LLM provider:** which providers to support, and is a local model (Ollama/llama.cpp) preferred over cloud for the opt-in path?
- **Q7. Calendar:** replicate OOo's Julian-before-1582 behaviour or use proleptic Gregorian (simpler; matches Excel and LibreOffice for most purposes)?
- **Q8. Distribution:** code-signing certificates (Apple Developer ID, Windows EV/Azure Trusted Signing), app stores (Mac App Store sandboxing affects file access), auto-update channel.
- **Q9. Web build:** is a browser version a product goal? If yes, promote the WASM client from test harness to first-class in v0.3.
- **Q10. Macros:** is Basic macro support ever in scope (huge), or permanently "preserve but never run"?

### Critical files for implementation (to be created; none exist yet)

- /Volumes/FILES/Projects/OSX_Office/crates/calc-types/src/lib.rs (shared value, address, error and settings types; the contract everything depends on)
- /Volumes/FILES/Projects/OSX_Office/crates/calc-api/src/dto.rs and /Volumes/FILES/Projects/OSX_Office/crates/calc-api/src/service.rs (DTOs exported to TS, and the `DocumentService` facade that Tauri, WASM and the CLI wrap)
- /Volumes/FILES/Projects/OSX_Office/crates/calc-functions/src/registry.rs and /Volumes/FILES/Projects/OSX_Office/crates/calc-functions/src/coerce.rs (function registry and central coercion that every function batch relies on)
- /Volumes/FILES/Projects/OSX_Office/crates/calc-formula/src/ast.rs (relative-reference AST that copy/paste, fill, reference adjustment and all three dialects rely on)
- /Volumes/FILES/Projects/OSX_Office/packages/bridge/src/client.ts and /Volumes/FILES/Projects/OSX_Office/packages/grid/src/types.ts (the `EngineClient` and `GridDataSource` contracts that let the UI and grid be built against mocks in parallel)
- /Volumes/FILES/Projects/OSX_Office/apps/desktop/src-tauri/src/commands.rs and /Volumes/FILES/Projects/OSX_Office/apps/desktop/src-tauri/tauri.conf.json (command surface, capabilities/CSP lockdown, macOS overlay title bar)

No files were created or modified during this planning task.