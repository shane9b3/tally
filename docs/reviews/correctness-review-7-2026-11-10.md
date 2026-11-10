# Review 7: correctness of multithreaded recalculation (6fe72d5)

Method: I read the whole diff (`parallel.rs`, the `recalc.rs` changes, `lookup_cache.rs`, `formula_cells.rs`, the prefs plumbing) against docs/recalc-parallel.md. I checked each finding with a throwaway probe crate in the session scratch area. The probe is a binary that depends on `crates/tally-engine` by path; nothing was committed. It builds `Threading::Sequential` and `Threading::Parallel` copies of the same workbook (padded with 3,000 formulas, like `tests/recalc_parallel.rs`) and compares values and `RecalcReport`s. For the regression check I built a second probe against `git archive 6fe72d5^`. Builds used `CARGO_TARGET_DIR=.targets/rev7c -j 4`.

## Findings

### 1. High (regression, sequential mode): the "one deferred cell at a time" work stack makes recalc quadratic in the number of deferred siblings
- Code: `crates/tally-engine/src/recalc.rs:1065-1072` (`CalcCtx::evaluate`). The commit replaced "push every deferred dirty cell not yet on `work`" with "push the first one, `break`".
- Why it is slow: when an attempt is deferred, every cell on the evaluation stack is reset to Dirty. With one push per attempt, `top` is retried once for every sibling it deferred. Each retry re-walks all the remaining sibling chains down to depth 16 before deferring the next one. M sibling deferrals now cost about M × M × 16 evaluations; before the commit they cost about M × 16.
- Reproduction (sequential, release build): `A1 =SUM(INDIRECT("B1:B<n>"))`. Each row r has a 24-long dynamic chain, `col c =INDIRECT(ADDRESS(r;c+2))`, ending in `=1`. This is the first recalc, so the dynamic precedents are not known yet.

  | n | 6fe72d5^ | 6fe72d5 |
  |---|---|---|
  | 100 | 7 ms | 120 ms |
  | 400 | 26 ms | 1.76 s |
  | 1000 | 72 ms | 11.3 s |

  Results are the same (A1 = n). The whole cost sits inside one `ctx.evaluate` call. The budget/deadline (`recalc.rs:252`) is checked only between components, so the 20 s open deadline does not cut it off. A file like this hangs on open for minutes (about 3 min at n = 4000). The parallel path does not help: A1 bails in the pre-pass and runs sequentially.
- Fix: push all deferred siblings again, and make the cycle test exact instead of relying on "every entry waits for the one above it". Keep a parent index per `work` entry (the entry that deferred it). Then deferring `d` is a cycle only if `d` is on the parent chain of `top`; a sibling that happens to be on `work` is not. A cheaper option: push all siblings, and when `d` is already on `work` but is not an ancestor of `top`, skip it rather than declare a cycle. Add the n = 1000 case to the tests as a timing guard.

### 2. High: a pre-pass evaluation that stops reading a range early skips the sequential pass's range-wide on-demand pull, so values (Err:522) and counts differ
- Code: `recalc.rs:1512-1516` (`ensure_clean` returns at once in a parallel context) and `recalc.rs:1717` (`for_each_in_range`). In sequential mode `ensure_clean(range)` evaluates every dirty formula in the range before iterating. It also marks the reader circular if the reader itself lies in the range (the `on_stack` block). The parallel context only checks the cells it actually visits. When the callback returns `Break` (SUM/SUMPRODUCT/conditional and financial aggregates stop at the first error value: `coerce.rs:584-660`, `sumproduct.rs:70`, `conditional.rs:169-191`, `financial/mod.rs:123`), the rest of the range is never checked. A dirty formula ordered *after* the reader, which the sequential pass would pull inside the reader, goes unnoticed, and the reader is pre-computed.
- Only reachable through dynamic ranges whose precedents are not known yet (INDIRECT/OFFSET/cross-sheet `:`). That covers the first recalc after a file is opened or after the formula is entered or its target changes. Static ranges create Tarjan edges, so their formulas come earlier or in the same cyclic component.
- Reproduction (probe; B1 is a constant `#DIV/0!`):

  | cells | sequential | parallel |
  |---|---|---|
  | `F1 =SUM(OFFSET(B1;0;0;1;6))` (range includes F1) | F1 Err:522, circular 1 | F1 `#DIV/0!`, circular 0 |
  | `F1 =SUM(INDIRECT("B1:B10"))`, `B5 =F1+1` | F1, B5 Err:522, circular 2 | F1, B5 `#DIV/0!`, circular 0 |
  | `F1 =SUM(INDIRECT("B1:B10"))`, `B5 =U1+1` | values equal, `evaluated` 3001 | values equal, `evaluated` 3002 |

  In a workbook with ≥ 4,096 dirty formulas, opening it therefore shows (and a save writes) a different value than single-threaded recalc. The next recalc knows the dynamic precedent, treats the cells as a cyclic component and corrects them, so the value flips after an unrelated edit. In the third case `RecalcReport::evaluated` differs, which breaks the "counts identical" claim and shifts formula-budget accounting. The differential suite misses this because its random workbooks rarely put a constant error before a later-ordered dirty formula in a dynamic range.
- Fix: in a parallel context, make `ensure_clean` mirror the sequential one instead of returning:
  - for each id from `dirty_formulas_in(range)`, call `sh.read(id, self.par_pos)` and bail (wait on `Pending`, bail for good on `Never`) unless it is `Done`;
  - bail for good if the formula being evaluated intersects the range (the `on_stack` check).

  Treat the `!all_final` branch of `par_lookup_index` (`recalc.rs:1483`) the same way: it currently returns `None` without bailing. After the fix the following scan would bail anyway, but bailing there directly is clearer. Add the three cases above to `recalc_parallel.rs`, and let the random generator put error constants into ranges read by INDIRECT/OFFSET.

### 3. Medium (pre-existing, not fixed by this commit): false Err:522 when the first deferred cell was evaluated later in the same attempt
- Code: `recalc.rs:1046-1048` and `1074-1078`. If every cell in `deferred` is clean by the end of the attempt, `pushed` stays false and `top` is given Err:522 (and counted as circular). This happens when a cell deferred at depth 16 was later pulled on demand at a shallower depth. Progress was made, and retrying `top` would succeed.
- Reproduction (sequential, both 6fe72d5^ and 6fe72d5): `K1..K24 =INDIRECT("K<i+1>")`, `K25 =7`, `A1 =INDIRECT("K1")+INDIRECT("K<k>")`. For k = 11…16, A1 is Err:522 (circular 1); for other k it is 14. The commit message says this stack "no longer reports a false Err:522", but this case remains. Both modes agree, so the differential tests cannot see it.
- Fix: when nothing was pushed but some entry of `deferred` is now clean (or `top`'s attempt made progress), `continue` to retry `top`. Keep the give-up only for an attempt where nothing changed (for example, compare a count of formulas made clean before and after the attempt).

### 4. Low (performance): the pre-pass can permanently downgrade the lookup cache, and retries repeat expensive evaluations
- `lookup_cache.rs` `not_clean`: pre-pass builders that fail because the line holds formulas not yet final for *them* count against `MAX_ATTEMPTS` (3). The line then becomes `Skip` for the rest of the recalc, including the sequential pass, where it would have been built once the formulas were clean. Lookups into that line then scan every time, which is quadratic for many lookups. Reachable with dynamic lookup ranges. Fix: in the pre-pass, do not count a failure caused by ordering (only `Pending`/later-ordered formulas) toward `failed`; or reset `Seen`/`Skip` lines touched by the pre-pass when the cache is handed back.
- `parallel.rs:392`: a formula can be fully re-evaluated up to 9 times (8 retries). A bailed evaluation also keeps running on Err:522 placeholders until it finishes. For expensive formulas with a late dynamic target, the pre-pass can do several times the sequential work. With the open deadline this can leave more cells stale than a single-threaded open would. The doc already accepts deadline differences, but not extra work. Consider stopping evaluation at the first bail (an early-out flag checked in `for_each_in_range` and `cell_value`), and fewer retries for formulas that already used much work.

### 5. Low: a panic in a worker discards the whole pre-pass, which differs from a sequential panic
- `rayon::scope` re-raises a worker panic on the calling thread after the other tasks finish. The shell's `catch_unwind` (`commands.rs:57`) catches it, so the app does not crash and the mutexes recover (`PoisonError::into_inner`). Nothing is stored, though: every dirty formula stays `Dirty` with its stale cached value, and `wb.pending` was already taken. A sequential panic at the same formula would have stored everything ordered before it. The two cases are equivalent in kind (the document is inconsistent until a hard recalc), but in parallel mode more stale values remain. Optional: on a pre-pass panic, catch it in `prepass` (`catch_unwind` around `pool.install`), drop the slots and run the sequential pass, which reproduces the sequential behaviour exactly.

## Checked and found OK

- Read paths: `formula_value`, `cell_value` (Formula and ArrayPart), `for_each_in_range` (Formula and ArrayPart, including anchors outside the range), `lookup_index`, `table_op`, `rand`.
  - All of them consult the slot unless the formula is `Clean`.
  - All other `EvalContext` methods (`cell_format_info`, `pivot_data`/`pivot_data_excel` (stored pivot lookup), `hidden_row`, `filtered_row`, `used_bounds`, names, `parse_reference_*`, `now_serial`) read state that the sequential pass never changes.
  - `eval.rs` reaches cells only through the context.
  - No thread-locals or global state in the engine, functions, formula or numfmt crates affect results (the only ones are test-only counters, the function registry and l10n tables).
- `ParShared::read`:
  - the `pos >= me` test puts same-component and later formulas in `Never`;
  - a self-read is `Never`;
  - cyclic components never run, because each member's counter includes an in-component edge;
  - strict members and iterated members are never read as final.
- Iteration: `iterate_scc` touches only members and clears the cache every round. A pre-computed formula cannot have read a member (it would be `Pending` forever or `Never`).
- RAND/RANDBETWEEN and `MULTIPLE.OPERATIONS` bail for good. Readers of RAND cells wait forever and fall to the sequential pass, so the RNG stream and the table-op caps and dependency walk see sequential order. NOW/TODAY use the recalc's fixed `now`. CELL/INFO are static.
- Precision as shown: `eval_par` rounds with `shown_number` exactly as `evaluate_once` does. Constants are rounded identically in `cell_value` and `for_each_in_range`. Array matrices are stored unrounded in both modes.
- Array formulas: `array_value_of` is a straight extraction of `array_value`. `matrix` is kept only for array formulas, as in `store_result`. `formulas_in` and `dirty_formulas_in` include anchors of blocks that spill into the range (ArrayPart rows are in `formula_rows` and `formula_id()` returns the anchor), so `latest` in `par_lookup_index` covers them.
- Dynamic precedent registration: `Done` carries the dynamic deps and spans, and `store_result` re-registers them at the formula's sequential turn, the same moment as `evaluate_once`. Workers never touch `wb.deps`.
- Lookup cache:
  - `Building` is always resolved: `insert` or `not_clean` on every path after `Request::Build`, and no early return in between.
  - Index and `with_dirty` are inserted under one lock.
  - Hits on an index holding formulas at or after the reader's position are refused.
  - `with_dirty` lines are `forget`-ed before the sequential pass. Constant-only lines are kept, which is safe.
  - The mutex is held only for map operations, never across evaluation.
- Scheduler:
  - `pending` counters with `AcqRel` and `OnceLock` publication give happens-before for readers.
  - `park`/`wake` is a correct Dekker pair: SeqCst fences, `has_waiters` set while holding the lock that `wake` takes, and a recheck of the slot under the lock.
  - A node runs in at most one task at a time.
  - Waits follow strictly decreasing positions, so there is no deadlock. Parked nodes whose target never completes are just dropped when `scope` ends.
  - Retries are bounded (8). Chains loop instead of recursing, so stack use is bounded.
- Budget: `report.evaluated` increments at the same points in both modes (one per component member reached while dirty, slot or not). Formula-budget exhaustion therefore stops at the same component in both modes, given finding 2 is fixed. The `max_cells_evaluated >= dirty` guard is conservative. Deadline differences are documented.
- Stack: workers use 8 MiB stacks, larger than the 2 MiB tokio blocking threads the sequential path runs on, and the pre-pass never nests on-demand evaluation.
- The deferred-stack change "only entries still dirty get Err:522" is correct: an entry finished by a nested on-demand evaluation is not on the cycle, and overwriting it changed values other cells had already read.
- `wanted()`: Sequential/Parallel/Auto precedence; the env override is cached once; the pool is not built for small Auto recalcs (`dirty >= PAR_MIN_FORMULAS` short-circuits `threads()`); there is a test-hook escape. Without the `parallel` feature everything is stubbed and `prepass` is never called.
- Tauri prefs:
  - `prefs::apply_multithreading` runs in `setup` before any document opens, and in `set_prefs` after a successful `store` (a failed store leaves the old setting, which matches the file).
  - The default is on for a missing, garbage or non-bool value (unit test present).
  - Other writers (`with_string_key`) keep the key as is.
  - The setting is process-wide and atomic, and read once per recalc.
- TS: `calcMultithreading` has a `DEFAULT_PREFS` value of true and is parsed with `bool(..., default)`, so old prefs files migrate to on. The dialog binds the app prefs (not the document settings), so the setting is not saved with the document.
