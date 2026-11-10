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
   Before reading a range, every dirty formula in it is evaluated on demand
   (`ensure_clean`), even if the function then stops reading at the first
   error.

Shared mutable state touched while evaluating: the formula cells
(`state`, `cached`, `array_result`, `dynamic_deps`), the dependency index
(when dynamic precedents change), `Workbook::rng` (RAND, one stream in
evaluation order), the `MULTIPLE.OPERATIONS` counters (`table_op_calls`,
`table_op_work`, capped per recalc, so which call hits the cap depends on
order), and the per-recalc lookup index cache (`lookup_cache.rs`).
`RecalcBudget` (the watchdog: 50M formulas or 20 s on open) is checked
between components and, for a formula whose on-demand chain was deferred,
between the attempts of its work stack.

## Scheme: a parallel pre-pass, then the unchanged sequential pass

The parallel path does not replace the sequential algorithm; it runs
*before* it and pre-computes the results of as many formulas as it can
prove are safe, then the sequential loop runs exactly as before, taking a
pre-computed result instead of evaluating whenever it reaches a formula
that has one.

1. Steps 1–3 as above. Tarjan keeps each node's dirty precedents (it
   computes them anyway; normally they are freed as nodes finish), at most
   `max(MAX_EDGES, 16 × dirty formulas)` edges (`MAX_EDGES` = 4 Mi, about
   12 bytes each). A range precedent is one edge per dirty formula in the
   range, so N formulas summing a range of N formulas are N² edges; past
   the cap the lists are freed and the recalc runs single-threaded (security
   review 7, H2: 16,000 such formulas held 3.2 GB).
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
     times per formula and as many retries in all as there are dirty
     formulas), so a new INDIRECT target delays a formula instead of
     sending it and its dependents to the sequential pass;
   - a dirty formula ordered at or after *p* (only reachable through a
     dynamic reference found now: known edges always point backwards), or
     outside the dirty set: the sequential pass would evaluate it on demand
     *inside* this formula, which changes where it is counted and when its
     dynamic precedents are registered; left to the sequential pass;
   - RAND / RANDBETWEEN (`rand()`): the random stream must be consumed in
     the sequential order;
   - `MULTIPLE.OPERATIONS` (`table_op`): its per-recalc caps are
     order-dependent and its sandbox walks the dependency index;
   - a range read: before it, the sequential pass evaluates every dirty
     formula of the range on demand and finds the reader itself inside it
     (a cycle). So the pre-pass checks *every* dirty formula of the range
     with the two rules above before reading (not only the cells the
     function visits: SUM stops at the first error value), and bails for
     good if the formula being evaluated lies in the range (correctness
     review 7, finding 2). A lookup index build checks its line the same
     way.

   A bail stops the evaluation: from then on cell and range reads return
   at once and the evaluator runs no further function call
   (`EvalContext::abandoned`), so a discarded attempt costs only the work
   up to its first bail, not a whole (possibly different, since it read
   Err:522 in place of a value) evaluation (security review 7, M1).

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
- *Sequential fixes this needed* (`CalcCtx::evaluate`, the explicit work
  stack for on-demand chains deeper than 16): when a deferred cell was
  found already on the work stack, every entry above it was set to
  Err:522, including entries that had meanwhile finished with a proper
  result (so a cell that other cells had already read changed value
  afterwards), and including siblings that were merely waiting their turn
  (a false Err:522). Each work-stack entry now records the entry that
  deferred it; all cells one attempt deferred are pushed at once and the
  attempt is repeated once after all of them. A deferred cell is a cycle
  only if it is an ancestor of the attempt (one of the cells *waiting* for
  their deferred cells), and only the entries on that parent chain that
  are still dirty get Err:522. An attempt whose deferred cells were all
  finished later in the same attempt (on demand, at a shallower depth)
  made progress and is repeated rather than given Err:522. A cell left
  incomplete by a deferral and read again in the same attempt is deferred
  too instead of being re-entered (re-entering redid its whole on-demand
  subtree once per reader). (The first
  version of this fix pushed one deferred cell at a time, which made one
  `evaluate` cubic in the number of deferred siblings: security review 7,
  H1; correctness review 7, findings 1 and 3.) These cases need dynamic
  references or lazy branches nested more than 16 deep and were found by
  the differential tests and the reviews.
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
- *Panics.* A panic in a worker (a bug) is caught around the pool: the
  pre-pass is dropped (results and lookup cache) and the sequential pass
  runs alone, so it stores everything before the panicking formula and
  then panics on it in its own order, exactly like a single-threaded
  recalc.

### Shared caches

The lookup index cache (`lookup_cache.rs`) is a pure accelerator: an
index answers exactly what a scan answers (tests
`lookup_cache::randomised_engine_results_equal_linear_scan`). During the
pre-pass it sits behind a `Mutex`; a request that wants to build marks the
line `Building` so concurrent requests scan instead of building twice; the
index is built outside the lock, only when every formula in the line is
final for the builder (clean, or pre-computed and ordered before it),
from those final values. A builder that finds a formula not final for it
waits or bails as for any read, and hands the line back uncounted
(`LookupCache::release`): the failure is about the reader's position, not
the line, so it must not turn the line into a scan for the rest of the
recalc. An evaluation that has already bailed (RAND) does not ask for an
index at all (security review 7, M2). A line holding pre-computed formulas remembers
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
  runs on the caller's thread). `TALLY_CALC_THREADS=N` sets the pool size
  instead (1 to 16, so it can also be larger than the available
  parallelism; `1` disables the pre-pass).
- One pre-pass uses the pool at a time. With `Threading::Auto`, a recalc
  that finds the pool busy (another document's pre-pass) runs
  single-threaded instead of waiting behind it; `Threading::Parallel` and
  `TALLY_CALC_THREADING=force` wait.
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
- The pool size is capped and the pre-pass work is bounded by a small
  multiple of the sequential work: every dirty formula is attempted once,
  plus at most one retry per dirty formula in all, each attempt under the
  per-formula work budget and stopped at its first bail; the sequential
  pass then evaluates whatever bailed. Kept edges are capped (above). So
  a hostile file gains at most that constant factor over the
  single-threaded limits (formula count, time, per-formula work budgets).
  `RecalcReport::prepass_retries` and `prepass_wasted_work` report the
  discarded work.

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
  RAND order, 3-D spans, arrays, MULTIPLE.OPERATIONS, budget, and the
  review 7 cases: range reads that stop at an error, a lookup line with a
  later formula, the edge cap, a bailed evaluation's wasted work, the
  retry budget, RAND readers and the lookup index). The random workbooks
  include error constants and INDIRECT/OFFSET ranges.
- Unit tests (`recalc::tests`, `parallel::tests`): the work stack retries
  a reader once after all its deferred cells and stops at the deadline,
  the edge cap, lookup lines released uncounted, whole-range checks, a
  worker panic falling back to the sequential pass, a busy pool.
- `tests/recalc.rs`: deferred siblings (30,000) and 600 dynamic 24-long
  chains in linear time, a deferred cell finished later in the same
  attempt, a real cycle through deferred cells.
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
- The pre-pass keeps each dirty formula's dirty precedents (about 12 bytes
  per edge, at most `max(4 Mi, 16 × dirty)` edges, otherwise the recalc is
  single-threaded) and a few counters per formula; for 200k formulas with
  a few precedents each that is a few MB for the duration of the recalc.
  Pre-computed results (text up to 4 MiB, array matrices) are held next to
  the cells' old values until the sequential pass stores them, so peak
  result memory can be about twice a single-threaded recalc's; results of
  formulas the deadline skips are dropped unused (security review 7, L2:
  accepted).
- Wide ranges over many dirty formulas (past the edge cap) are
  single-threaded.
- A long single chain has no parallelism (it runs as one task).
- Iterative calculation is sequential.
