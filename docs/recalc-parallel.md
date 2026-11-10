# Multithreaded recalculation

Status: built 2026-11-10 (`crates/tally-engine/src/parallel.rs`, the
`parallel` cargo feature of `tally-engine`, on by default). Plan reference:
`docs/architecture-plan.md` §3.6 ("rayon over independent dirty
components").

AOO 4.1 calculates on one thread (`ScDocument::CalcAll`,
`ScFormulaCell::Interpret` with its recursion helper); there is no setting.
LibreOffice added "Enable multi-threaded calculation" (Tools ▸ Options ▸
Calc ▸ Calculate ▸ CPU threading settings, on by default). Tally follows
LibreOffice here: an app preference, on by default, with no effect on
results.

## How a recalc works (single-threaded, unchanged)

`recalc::recalc_pass` (see the module docs of `recalc.rs`):

1. **Seeds**: pending edits, new formulas, every volatile formula (NOW,
   TODAY, RAND, RANDBETWEEN, OFFSET, INDIRECT, INFO, CELL ...), and with
   "precision as shown" the re-formatted cells.
2. **Dirty propagation**: breadth-first over the reverse dependency index
   (single cells, area listeners, 3-D spans, names), coalesced per level.
3. **Ordering**: an iterative Tarjan over the dirty subgraph. Edges are
   static precedents plus the dynamic precedents (INDIRECT, OFFSET, the
   `:` operator across sheets) recorded at the previous evaluation.
   Strongly connected components come out precedents first. Members of a
   cycle of *strict* precedents get Err:522 without evaluation; with
   iterations on a cyclic component is iterated Gauss–Seidel (Err:523 when
   it does not converge, at most `MAX_ITERATION_EVALS` evaluations per
   recalc).
4. **Evaluation** in that order through `CalcCtx`. A dirty precedent met
   during evaluation (a newly discovered dynamic reference, a lazy IF
   branch) is evaluated on demand (nested up to 16 deep, deeper ones
   deferred to an explicit work stack). Reading a cell that is being
   evaluated is a followed cycle: Err:522 for the cells on it.

Shared mutable state touched while evaluating: the formula cells
(`state`, `cached`, `array_result`, `dynamic_deps`), the dependency index
(when dynamic precedents change), `Workbook::rng` (RAND, one stream in
evaluation order), the `MULTIPLE.OPERATIONS` counters (`table_op_calls`,
`table_op_work`, capped per recalc, so which call hits the cap depends on
order), and the per-recalc lookup index cache (`lookup_cache.rs`).
`RecalcBudget` (the watchdog: 50M formulas or 20 s on open) is checked
between components.

## Scheme: a parallel pre-pass, then the unchanged sequential pass

The parallel path does not replace the sequential algorithm; it runs
*before* it and pre-computes the results of as many formulas as it can
prove are safe, then the sequential loop runs exactly as before, taking a
pre-computed result instead of evaluating whenever it reaches a formula
that has one.

1. Steps 1–3 as above. Tarjan keeps each node's dirty precedents (it
   computes them anyway; normally they are freed as nodes finish).
2. **Dataflow scheduling.** Every dirty formula gets a counter of its dirty
   precedents (including itself on a self-reference). Formulas with no
   dirty precedent are the roots. On the thread pool (`rayon::scope`, the
   roots fed through a parallel iterator), a task evaluates one formula
   with a read-only `CalcCtx` (`WbAccess::Par`) and, on success, stores the
   result in that formula's `OnceLock` slot and decrements the counters of
   its dependents. A dependent that reaches zero is ready; the first ready
   dependent is continued on the same thread (so a chain runs as a loop,
   without a task per cell), the others are spawned. This parallelises
   independent chains (100 chains run on 12 threads), wide flat sheets
   (200k independent SUMs) and mixed DAGs alike, without level barriers.
3. **Bail-out instead of guessing.** A parallel evaluation of the formula
   at position *p* of the sequential (Tarjan) order reads only *final*
   values: a clean formula's cached value (it is not dirty in this recalc,
   so it never changes during it) or the slot of a dirty formula that was
   already evaluated in this pre-pass **and comes before *p* in the
   sequential order**. Anything else makes the evaluation *bail*: its
   result is discarded and its slot stays empty:
   - a dirty formula ordered before *p* whose slot is still empty (a
     precedent found only now, through INDIRECT, OFFSET or a reference
     into an array block): the formula *waits* for it (it is parked under
     that formula's node and re-run when its slot is set, at most 8
     times), so a new INDIRECT target delays a formula instead of sending
     it and its dependents to the sequential pass;
   - a dirty formula ordered at or after *p* (only reachable through a
     dynamic reference found now: known edges always point backwards), or
     outside the dirty set: the sequential pass would evaluate it on demand
     *inside* this formula, which changes where it is counted and when its
     dynamic precedents are registered; left to the sequential pass;
   - RAND / RANDBETWEEN (`rand()`): the random stream must be consumed in
     the sequential order;
   - `MULTIPLE.OPERATIONS` (`table_op`): its per-recalc caps are
     order-dependent and its sandbox walks the dependency index.

   A formula that bails for good never decrements its dependents, so
   everything downstream of it (and every member of a cyclic component,
   whose counter includes an edge from inside the component) is simply
   never run in the pre-pass.
4. **Sequential pass.** The normal loop over the components in Tarjan
   order. For a formula with a filled slot that is still dirty, the slot's
   result is stored exactly as `evaluate_once` would store it (value,
   array matrix, "precision as shown" rounding already applied, dynamic
   precedents re-registered when they changed) and counted as evaluated.
   Every other dirty formula (bailed, never run, cyclic, iterated) is
   evaluated as before, on demand pulls and cycle detection included. A
   formula that was pulled on demand before its turn is clean when its
   turn comes, and its slot is ignored.

### Why the results are identical

- *Values.* A pre-computed formula read only final values, which are the
  values the sequential pass gives those cells, and formula evaluation is
  a deterministic function of the values it reads (same code, same float
  operations in the same order within the formula). So its slot holds the
  value sequential evaluation would compute.
- *Same moment.* Every dirty formula a pre-computed formula read comes
  before it in the sequential order, so when the sequential pass reaches
  it, all of them are already stored: the sequential evaluation would not
  have pulled anything on demand from inside it. Taking the slot instead
  of evaluating changes nothing that happens later (counters, dynamic
  precedent registration, on-demand pulls by later formulas).
- *No hidden cycles.* A formula succeeds only after every cell it read was
  final, so the "read" relation among pre-computed formulas follows time
  and is acyclic. A pre-computed formula pulled on demand by a sequential
  one re-reads only clean or pre-computed cells, so it never reaches a
  cell that is being evaluated: it is never on a followed cycle, and
  Err:522/523 are decided by the unchanged sequential code for exactly the
  same cells.
- *Two sequential fixes this needed* (`CalcCtx::evaluate`, the explicit
  work stack for on-demand chains deeper than 16): when a deferred cell
  was found already on the work stack, every entry above it was set to
  Err:522, including entries that had meanwhile finished with a proper
  result (so a cell that other cells had already read changed value
  afterwards); and several deferred siblings were pushed at once, so the
  second "waited for" the first and reading it was reported as a cycle
  (a false Err:522). Now one deferred cell is pushed at a time (so every
  entry waits for the one above it, which the cycle test assumes) and
  only entries still dirty are set to Err:522. Both cases need dynamic
  references or lazy branches nested more than 16 deep and were found by
  the differential tests.
- *Order-dependent state stays sequential.* RAND and `MULTIPLE.OPERATIONS`
  formulas (and their readers) always bail, so the random stream and the
  table-op caps are consumed by the sequential pass in its usual order. A
  pre-computed formula never reads a RAND cell (it would not be final), so
  it never changes where the sequential pass pulls one. Dynamic
  precedents are re-registered in the sequential order (step 4), so
  `MULTIPLE.OPERATIONS` sees the same dependency index as before.
- *Counters.* `RecalcReport::evaluated` counts a pre-computed formula when
  the loop reaches it, exactly where the sequential pass would have
  counted its evaluation; `circular`, `changed`, `iteration_evaluations`
  come from the unchanged code.
- *No data races.* Workers hold `&Workbook` only; the borrow checker
  forbids writes. Shared writable state in the pre-pass is the slots
  (`OnceLock`, set once by the formula's own task), the counters
  (`AtomicU32`, `AcqRel`: a dependent becomes ready only after all its
  precedents' slots were set, and the `OnceLock` publish/observe pairs
  give the happens-before for readers through unknown edges), the stop
  flag, and the lookup cache (moved out of the workbook into a `Mutex`
  for the pre-pass and moved back after it). `Workbook: Sync` is checked
  at compile time.

### Shared caches

The lookup index cache (`lookup_cache.rs`) is a pure accelerator: an
index answers exactly what a scan answers (tests
`lookup_cache::randomised_engine_results_equal_linear_scan`). During the
pre-pass it sits behind a `Mutex`; a request that wants to build marks the
line `Building` so concurrent requests scan instead of building twice; the
index is built outside the lock, only when every formula in the line is
final for the builder (clean, or pre-computed and ordered before it),
from those final values. A line holding pre-computed formulas remembers
the latest position among them; a formula ordered before that position
gets no index (it scans, and bails on the formula it may not read yet).
After the pre-pass such indexes are dropped (a sequential lookup into a
line with dirty formulas evaluates them on demand first, which an index
would skip); indexes over lines without dirty formulas (constant tables,
the common case) stay for the sequential pass of the same recalc and are
dropped at its end, as before. Iterated components still clear the cache
every round.

### Thresholds, threads, settings

- The pre-pass runs only when there are at least `PAR_MIN_FORMULAS`
  (4,096) dirty formulas, the pool has more than one thread, the recalc's
  formula budget cannot run out inside the pre-pass
  (`max_cells_evaluated >= dirty count`) and multithreading is enabled.
  Smaller recalcs (typing into a cell) take the old path with no extra
  work.
- Thread pool: a dedicated rayon pool (not the global one), built on the
  first parallel recalc, `min(available_parallelism, 16)` threads, 8 MiB
  stacks (formula nesting and name expansion recurse; the sequential path
  runs on the caller's thread). `TALLY_CALC_THREADS=N` caps the pool
  (`1` disables the pre-pass).
- `tally_engine::parallel::set_enabled(bool)`: the app preference Tools ▸
  Options ▸ Calculate ▸ "Enable multi-threaded calculation"
  (`calcMultithreading` in `prefs.json`, default on), read by the shell at
  start-up and on every prefs save. `TALLY_CALC_THREADING=off|on|force`
  overrides it (`force` also ignores the size threshold; tests).
- `Workbook::threading` (`Threading::Auto | Sequential | Parallel`) forces
  one mode for one workbook (differential tests, the conformance suite in
  parallel mode). Not saved in files.
- The watchdog: workers check the deadline before every formula; when it
  passes they stop, and the sequential pass then sees the exhausted budget
  and marks the rest clean with their cached values, as before. Which
  formulas are left unevaluated when the deadline hits can differ (it did
  already between runs, since it is time-based).
- The pool size is capped and the work is the same work the sequential
  path would do, so a hostile file gains nothing over the single-threaded
  limits (formula count, time, per-formula work budgets).

### WASM and builds without threads

`rayon` is an optional dependency behind the `parallel` feature (default
on). Without it `parallel.rs` compiles to the sequential stubs and the
setting is ignored. No wasm32 target is built today (no `wasm32` target
installed, no wasm crate in the workspace); `cargo check -p tally-engine
--no-default-features` is the build that a WASM crate would use.

## Tests

- `tests/recalc_parallel.rs`: seeded random differential runs (60 random three-sheet workbooks
  of constants and formulas: chains, ranges, SUM/VLOOKUP/MATCH, IF,
  INDIRECT/OFFSET, 3-D references, array formulas, cycles, iterations,
  precision as shown, RAND with a fixed seed), each workbook recalculated
  sequentially and with `Threading::Parallel`, compared cell by cell
  (values bit for bit, errors, array results, dynamic precedents) and report by
  report; then the same after random edits. Plus targeted cases (cycles
  with and without iterations, INDIRECT discovering a new precedent,
  RAND order, 3-D spans, arrays, MULTIPLE.OPERATIONS, budget).
  `TALLY_PAR_STRESS=N` multiplies the case count for race hunting;
  `SEED=n ... debug_seed -- --ignored --nocapture` prints one workbook
  cell by cell with the differences marked. (The random workbooks run on
  a 16 MiB thread: unoptimized builds of 16-deep on-demand chains with
  `MULTIPLE.OPERATIONS` sandboxes need more than a test thread's 2 MiB,
  single-threaded too.)
- `tests/conformance.rs` runs every vector a second time with
  `Threading::Parallel`.
- `TALLY_CALC_THREADING=force cargo test -p tally-engine` runs the whole
  engine suite through the pre-pass.

## Measurements

See `docs/perf-baseline.md` (section "Multithreaded recalc").

## Limits

- RAND / RANDBETWEEN formulas, `MULTIPLE.OPERATIONS`, cyclic components and
  everything downstream of them are evaluated on one thread.
- A formula whose new INDIRECT/OFFSET target is a dirty formula ordered
  *after* it (the order follows known precedents, so this happens when the
  target was entered later than the reader and nothing else links them)
  is left to the sequential pass with its dependents in that recalc. The
  next recalc knows the precedent and orders them correctly.
- With a formula budget (`RecalcBudget::max_cells_evaluated`, 50M on open)
  that iterative calculation could exhaust (iterations count one
  evaluation per member per step) or a deadline, which formulas are left
  with their cached values can differ from a single-threaded run. That was
  already true for the deadline, which is time-based.
- The pre-pass keeps each dirty formula's dirty precedents (one small list
  per formula) and a few counters; for 200k formulas that is a few MB for
  the duration of the recalc.
- A long single chain has no parallelism (it runs as one task).
- Iterative calculation is sequential.
