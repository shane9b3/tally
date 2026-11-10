# Security review 7: multithreaded recalculation (commit 6fe72d5)

Scope:
- `crates/tally-engine/src/parallel.rs` (new) and the `recalc.rs` changes: the parallel pre-pass, `topo_order`/`tarjan` with `keep`, `ParShared`, `par_lookup_index`, and the sequential deferred-work-stack change in `CalcCtx::evaluate`.
- The `lookup_cache.rs` `Building` state.
- `TALLY_CALC_THREADING` and `TALLY_CALC_THREADS`.
- The `calcMultithreading` pref in `apps/desktop/src-tauri/src/{prefs.rs,lib.rs}`.

Threat model: a hostile spreadsheet opened (and then edited) in the desktop app.

Method: probes ran as throwaway integration tests on `git archive 6fe72d5` and on `git archive 6fe72d5~1`, in `scratchpad/review7/{head,parent}`, release build, on a 12-core Mac. The probe source is `scratchpad/review7/head/crates/tally-engine/tests/probe7.rs`. Nothing was committed.

## Summary

| # | Sev | Where | What |
|---|---|---|---|
| H1 | High | `recalc.rs:1058-1071` (`CalcCtx::evaluate`, "one at a time") and `:1395` (`deferred.contains`) | The deferred-work-stack fix now re-runs the deferring formula once per deferred sibling, and each attempt is O(k²). The result is **O(k³) inside a single `evaluate`**, which the 20 s open watchdog does not see. k = 11,000 takes 63 s on open with the watchdog set, against 27 ms before the commit at k = 8,000. Affects every mode, including sequential. |
| H2 | High | `recalc.rs:633-637`, `:748` (`tarjan` with `keep`), `parallel.rs:199-210` | The parallel path keeps every dirty precedent list (`Vec<Vec<usize>>`) plus a `u32` reverse copy. Memory is **O(dirty edges), about 12.4 bytes per edge**, not "about 100 bytes per formula". 16,000 `=SUM($A$1:$A$16000)` over formula cells: 3.17 GB peak against 35 MB sequential. This runs before any watchdog check. |
| M1 | Medium | `parallel.rs:385-399`, `recalc.rs:971-981`, `:1374`, `:1682` | A bailed pre-pass evaluation is not stopped. It runs to completion with `#Err:522` in place of the unread value, so it can take a different and expensive branch. Each formula is evaluated up to 9 times. No recalc budget counts this work. 500 formulas: 3.4 ms sequential, **5.6 s** parallel. |
| M2 | Medium | `recalc.rs:1495-1499` (`par_lookup_index`), `lookup_cache.rs:138-149` | A build attempt by an evaluation that has *already* bailed for an unrelated reason (RAND) reports the line `not_clean`. Three such attempts mark the line `Skip` for the whole recalc, including the sequential pass. Every exact lookup into it then scans: 80,000 lookups take **8.6 s parallel against 0.11 s sequential** (quadratic). A benign `RANDBETWEEN` sampling sheet triggers it. |
| L1 | Low | `parallel.rs:120-133`, `:257` | One rayon pool serves all documents. Rayon runs injected jobs only when no local or stealable work is left, so a long pre-pass in one document stalls the parallel recalcs of every other open document. Until now, documents recalculated independently. |
| L2 | Low | `recalc.rs:229-245`, `:252-263` | The pre-computed results (`Done`: text up to 4 MiB, matrices) are held next to the old cached values until the sequential pass stores them, which doubles peak result memory. When the deadline cuts the sequential pass, the computed results of skipped formulas are thrown away. |

Informational:
- I1: `off`/`rev` offsets are `u32` and unchecked (`parallel.rs:199-210`).
- I2: Three statements in `docs/security.md` / `docs/recalc-parallel.md` are inaccurate (see I2).

Checked with no finding:
- unsafe code.
- `Sync` and data-race soundness.
- The park/wake protocol, deadlock and livelock.
- The retry counter.
- Panic propagation and Mutex poisoning.
- Thread stack sizes.
- The deadline and formula-budget gating.
- The `Building` state.
- Env var parsing and pref parsing.

Details are at the end.

---

## H1 (High): deferred siblings make one `evaluate` cubic and invisible to the watchdog

**Where:** `crates/tally-engine/src/recalc.rs:1058-1071`. The commit replaced

```rust
if !work.contains(&d) { work.push(d); pushed = true; }      // all siblings at once
```

with

```rust
work.push(d);
pushed = true;
break;                                                       // one at a time
```

**The mechanism:**
1. A formula that defers k siblings is no longer retried once. It is retried after each sibling, k+1 attempts in all.
2. Each attempt re-walks its whole on-demand chain. The frames marked `deferred` stay dirty (`:1145-1148`), so up to 16 levels are evaluated again.
3. At the bottom of the chain, the attempt re-collects the k−i siblings that are still deferred. `self.deferred.contains(&id)` (`:1395`) is a linear scan, so each attempt costs O((k−i)²).
4. In total, O(k³) inside one top-level `CalcCtx::evaluate`.

The watchdog (`budget.exhausted`, `:252`) runs only between components, and the per-formula work budget resets on every attempt. Neither bounds this.

**Measured** (sequential mode, release). The file: `A1..A15 = INDIRECT("A<n+1>")` (an on-demand chain 16 deep), `A16 = SUM(INDIRECT("B1:B<k>"))`, and `B1..Bk = =1`:

| k | before (6fe72d5~1) | after (6fe72d5) |
|---|---|---|
| 1,000 | 0.9 ms | 71 ms |
| 2,000 | 2.1 ms | 435 ms |
| 4,000 | 11 ms | 3.2 s |
| 8,000 | 27 ms | 24.2 s |
| 11,000, `RecalcBudget::on_open()` | — | **62.6 s** (`incomplete = true`: the 20 s deadline was noticed only after the formula finished) |

The time grows 8× per doubling. Extrapolated: k = 30,000 takes about 20 minutes, and k = 100,000 about 12 hours.

**Attack:**
1. An ODS or XLSX file with 16 INDIRECT cells and 100,000 `=1` cells (a few hundred KB).
2. Opening it hangs the document's engine thread for hours, holding the document lock. Close also needs the lock, so the document cannot be closed.
3. No setting avoids this: the sequential pass runs in every mode, and the pre-pass bails on these formulas (INDIRECT reaching a later formula is `Never`).

**Fix:**
1. Keep the correctness fix (only a true ancestor counts as a cycle) without the one-at-a-time retry. Store `work` entries as `(id, parent_index)` and push all deferred siblings of `top` with `parent = top`. The cycle test then walks the parent chain of `top` (`d` is an ancestor) instead of `work.iter().position`. Siblings never "wait" for each other, and `top` is retried once after all of them.
2. Make `CalcCtx::deferred` a `Vec` plus a `HashSet` (or a `SecondaryMap<FormulaCellId, ()>`) so that recording a deferral is O(1).
3. Defense in depth: count attempts inside `evaluate` against the recalc budget and check the deadline in the `while let Some(&top)` loop. When it is exceeded, set the remaining entries circular or stale and return.
4. Add a regression test: the file above with k = 50,000 recalculates in under 1 s, and `evaluate` attempts are ≤ 2 per deferred cell.

## H2 (High): the parallel path holds every precedent edge, quadratic memory before any watchdog

**Where:** `recalc.rs:226-227`. When `parallel` is set, `topo_order(wb, &dirty, true)` makes `tarjan` keep `succ[v]` for every node (`:748`), and returns the lists as `preds: Vec<Vec<usize>>` (`:633-637`). `parallel::prepass` then builds `off` (`u32`, n+1) and `rev` (`u32`, one per edge) (`parallel.rs:199-210`).

The sequential `tarjan` frees each list as its node finishes, so it holds only the DFS path. This path keeps:
- 8 bytes per edge in `preds`,
- 4 bytes per edge in `rev`,
- `Vec` headers.

An edge is every dirty formula inside a range precedent: `dirty_successors` expands ranges to cells. So N formulas reading a range of N dirty formulas give N² edges.

**Measured** (A1:AN = `=1`, B1:BN = `=SUM($A$1:$A$N)`, `/usr/bin/time -l`, peak footprint):

| N | Sequential | Parallel |
|---|---|---|
| 8,000 | 18 MB, 3.0 s | **802 MB**, 2.0 s |
| 16,000 | 35 MB, 15.5 s | **3.17 GB**, 10.0 s |

The cost is about 12.4 bytes per edge. N = 32,000 needs about 12.7 GB, and N = 45,000 about 25 GB. This is only 90,000 short formulas, a file of about 1 MB.

**Attack:**
1. On open, the dirty count is ≥ 4,096, so `Threading::Auto` takes the parallel path (default on).
2. `topo_order` runs before the pre-pass and before `budget.exhausted` is ever checked, so the 20 s watchdog cannot stop it.
3. macOS compresses and swaps, then either jetsams Tally (losing unsaved work in other documents) or the allocation fails and the process aborts.

The sequential path already spends O(edges) *time* in `topo_order`, which is a pre-existing gap: the watchdog does not cover ordering. But its memory stays flat, so this commit turns slow into out-of-memory.

**Fix:**
1. Bound the edges kept. Count edges as `tarjan` produces them, and if the total passes a cap (for example `16 × dirty.len()` or 32 M), drop `keep`, free the kept lists and run sequentially. Use the same check before allocating `rev`.
2. Better: never materialise range edges. The pre-pass needs only, per node, a pending count and its dependents. Build `pending` and `rev` straight from the edge callback as `u32` without keeping `preds`. For wide ranges, register one "range waiter" per range instead of per-cell edges (as `DepIndex` does for area listeners).
3. Correct `docs/security.md` ("about 100 bytes each" is per edge, not per formula), and add a release test: 16,000 × `SUM` over 16,000 formulas stays under 200 MB in parallel mode.

## M1 (Medium): bailed evaluations run to the end, on the wrong branch, up to 9 times

**Where:** `parallel.rs:385-399`; `recalc.rs` `bail_waiting` / `bail_for_good` (`:971-981`); `formula_value` and `cell_value` in the parallel branch return `Value::Error(ErrorKind::Circular)` (`:1374`, `:1682`).

**The mechanism:**
1. A bail only sets a flag. The evaluator keeps going with `Err:522` standing in for the value it could not read.
2. A formula whose cheap result depends on that value can therefore take its expensive branch in the pre-pass:
   ```
   =IF(ISNUMBER(INDIRECT("Z"&ROW()));0;SUMPRODUCT(LEN($A$1:$A$1000000)))
   ```
   Here `Z` is a later formula, so the read is `Never` and the formula bails for good.
3. The result is then thrown away, and the sequential pass evaluates the formula again cheaply.
4. With `Pending` reads (an INDIRECT target ordered earlier), this repeats up to `MAX_RETRIES` + 1 = 9 times per formula, at up to 50 M work units each (`MAX_FORMULA_WORK`).
5. None of this is counted against `max_cells_evaluated`. On `apply` there is no deadline at all.

**Measured** (N = 500 such formulas plus 500 `Z` cells, `Threading::Parallel`): sequential 3.4 ms, parallel **5.6 s** (×1,650). Scaled to 50,000 formulas this is about 9 minutes per edit that dirties them, and on open it uses up the whole 20 s watchdog in the pre-pass. The sequential pass then has no time left, and every formula keeps its file-cached value.

`docs/security.md` says the pre-pass "does only work the single-threaded recalc would do". That is not true.

A hostile file can already make sequential recalc expensive directly, so this is a multiplier (bounded per formula by 9 × 50 M) rather than a new capability. That is why it is Medium.

**Fix:**
1. Short-circuit once the context bails. Have the work-budget charge in `eval.rs` fail as soon as `ctx.bailed` is set: return the formula-overflow path, which unwinds to `eval_par` in O(depth). Also have `for_each_in_range` and the range functions check it.
2. Count pre-pass attempts in the `RecalcReport`, and leave the pre-pass early (`stop`) when wasted attempts pass a fraction of `dirty`.
3. Correct the `security.md` sentence.

## M2 (Medium): an unrelated bail marks a lookup line `Skip` for the whole recalc

**Where:** `recalc.rs:1495-1499`:

```rust
self.for_each_in_range(&bounds, true, &mut |a, v| { ... });
if self.bailed {
    if let Some(c) = sh.cache().cache.as_mut() { c.not_clean(line); }
    return None;
}
```

`self.bailed` is the *whole evaluation's* flag, so it is already true when the formula bailed earlier on RAND or on an unrelated cell, even though the line is perfectly final. `not_clean` counts this as a failed build. After `MAX_ATTEMPTS` = 3 (`lookup_cache.rs:57`) the line becomes `Skip`. `prepass` hands the cache back with that `Skip` (only `with_dirty` lines are forgotten), so every lookup into the line, in the pre-pass and in the sequential pass, scans for the rest of the recalc.

**Measured.** The file:
- A table in `A1:B<r>`.
- 200 formulas `=RAND()+VLOOKUP(i;$A$1:$B$r;2;0)`.
- r formulas `=VLOOKUP(ROW()+0*$G$1;$A$1:$B$r;2;0)`, where `G1` is slow so the RAND formulas run first.

| r | Sequential | Parallel | Parallel without the RAND formulas |
|---|---|---|---|
| 20,000 | 25 ms | 385 ms | 24 ms |
| 80,000 | 109 ms | **8.6 s** | — |

The parallel time grows quadratically with r. The trigger is nondeterministic: it depends on which thread wins the `Building` slot. Benign workbooks that sample with `=VLOOKUP(RANDBETWEEN(1;n);table;2;0)` hit it.

**Fix:**
1. Record `self.bailed` before the scan and test only for a bail *during* it. Simplest: save `let before = self.bailed; self.bailed = false;` and restore after.
2. Or, in `par_lookup_index`, return `None` without asking for a build (`request`) when the context has already bailed, since its result will be discarded anyway.
3. Either way, on a bail that is not the line's fault, release `Building` back to `Seen { failed }` unchanged rather than counting it as a failure.

## L1 (Low): one pool for all documents, so a hostile document stalls other documents' recalcs

**Where:** `parallel.rs:120-133` (one `static POOL`) and `:257` (`pool.install`).

Every document's recalc runs on its own tokio blocking thread (`commands.rs:119`). Rayon's `find_work` (`rayon-core-1.13.0/src/registry.rs:833-842`) prefers local and stolen jobs over injected ones. While document A's pre-pass has tasks, document B's `install` job (and its whole pre-pass) waits. With M1, A's pre-pass can run for minutes on an edit, and B, a benign document with ≥ 4,096 dirty formulas, freezes for that long.

`commands.rs` says the other documents "stay usable"; for parallel recalcs they now do not.

**Fix:** guard the pool with a try-lock or an "in use" `AtomicBool`. If another recalc owns it, run this one sequentially. Alternatively, give each pre-pass a share of the threads.

## L2 (Low): pre-computed results are held next to the old values

**Where:** `recalc.rs:229-245` (`pre` lives until `recalc_pass` returns) and `:252-263`.

`ParShared::slots` holds one `Done` per pre-computed formula, holding text up to `MAX_TEXT_BYTES` (4 MiB) or an array matrix. Meanwhile the cells still hold their old `cached` / `array_result`. The sequential pass replaces them one by one, so peak result memory is about twice that of a sequential recalc. When the deadline cuts the sequential pass (`budget.exhausted`), the remaining `Done`s are dropped unused, after their full cost was paid.

**Fix:** none needed now. If text- or array-heavy files become a concern, cap the bytes held in `slots`: once over the cap, stop the pre-pass from starting new formulas.

## Informational

- **I1.** `off` and `rev` are `u32` with unchecked `+=` (`parallel.rs:199-210`). Above 2³² edges, the release build wraps, and `rev[a..b]` would panic (caught by `guarded`). This is unreachable today because H2 runs out of memory first, at about 50 GB. The edge cap from H2 also closes it.
- **I2.** Documentation inaccuracies:
  - `security.md` says the pre-pass holds "about 100 bytes" per dirty formula. It is O(edges) (H2).
  - `security.md` says it does "only work the single-threaded recalc would do". It does not (M1).
  - `security.md` and `recalc-parallel.md` say `TALLY_CALC_THREADS` "lowers" or "caps" the pool. It can also *raise* it above `available_parallelism`, up to 16 (`wanted_threads`, `parallel.rs:106-117`). That is harmless, but the wording is wrong.

---

## Checked and found OK

- **`unsafe`:** none added (`git show 6fe72d5 | grep unsafe` matches only the doc line).
- **Sync and data races:**
  - Workers get `&Workbook` and `&ParShared`. The compile-time assertion (`parallel.rs:137-142`) proves both are `Sync`.
  - The engine, function and formula crates have no shared interior mutability reachable through `&Workbook`. The `RefCell`s in `EvalEnv`, `condfmt` and `eval` are per-context and not shared. The only `thread_local!` instances are test hooks, and `wanted` disables the pre-pass while a hook is set. `tally-functions`' `REGISTRY` is an immutable `OnceLock`.
- **Park/wake** (`parallel.rs:345-373`):
  - The `has_waiters` store, SeqCst fence and slot read in `park`, against the slot set (`OnceLock`, release), SeqCst fence and `has_waiters` read in `wake`, form a correct Dekker pair. `park` also re-reads the slot under the lock that `wake` takes.
  - No lock is held while calling `process`, and the `Mutex`es are never nested.
  - A node waiting on a node that later bails is simply never woken. The scope still ends (no outstanding task), and the node falls to the sequential pass. Two nodes waiting on each other through INDIRECT end the same way. **No deadlock or hang.**
- **Retries:**
  - Each node has exactly one live instance. It is started only by `pending` reaching 0 (or as a root), or by one waiter entry. So `retries[v]` grows by one per attempt, and the `< MAX_RETRIES` gate stops it at 9 attempts. The `AtomicU8` cannot wrap.
  - The `continue` path (the precedent finished meanwhile) also passes through that gate and through the deadline check.
- **Panics:**
  - `rayon::scope` catches job panics and rethrows the first at the end of the scope. `install` rethrows it on the calling tokio thread. `commands::guarded` (`catch_unwind`) turns it into an `internal` error, and the pool threads survive.
  - The `waiters` and `ParCache` locks recover from poisoning with `into_inner`. `ParShared` is dropped on unwind, and the next `recalc_with` sets a fresh `lookup_cache`, so later recalcs are not bricked.
  - The release profile has no `panic = "abort"`.
  - What is lost (the taken `pending` set) matches a panic in the sequential path.
- **Stacks:**
  - The engine runs on Tauri's tokio blocking threads (`commands::blocking` → `spawn_blocking`). `TokioRuntime::new()` sets no `thread_stack_size`, so they have the std default of **2 MiB**.
  - The workers have **8 MiB**, and in the pre-pass they never evaluate on demand (`formula_value`/`ensure_clean` return or bail). Their recursion is one expression plus ≤ 16 nested names, while the sequential path nests up to 16 formulas on 2 MiB. So workers have strictly more headroom than the main path.
  - Rayon's join "stack stacking" during `par_iter` is bounded by log₂(roots) join frames plus one leaf `process` task. `process` never blocks, and chains are a loop, not recursion.
  - A stack overflow in a worker would abort the process, exactly as on the engine thread. Nothing new.
- **Watchdog and formula budget:**
  - Workers check `deadline` before every formula, including retries, and set `stop`. The sequential pass then sees `exhausted` and keeps cached values.
  - `wanted` refuses the pre-pass when `max_cells_evaluated < dirty`, so the formula budget cannot run out inside it.
  - Per-formula work (`MAX_FORMULA_WORK`) applies per attempt. Concurrency adds at most one in-flight formula per thread past the deadline.
  - The gaps are the ones above: H1 is inside one formula, H2 is before the watchdog, and M1's work is uncounted.
- **`Building` state** (`lookup_cache.rs`): every `Request::Build` in `par_lookup_index` ends in `insert` (Built or Skip) or `not_clean` (Seen or Skip). There is no early return between `request` and those calls except the shared cache vanishing, which cannot happen during the pre-pass. A stuck `Building` would only mean scanning, never a hang.
- **`TALLY_CALC_THREADS`:**
  - Parsed as `usize` after `trim`. `0`, negative, garbage and overflow fall back to `available_parallelism`. Huge values clamp to 16, and `clamp(1, 16)` ensures at least 1.
  - The pool is built once. If thread creation fails, `build().ok()` gives `None` and recalc runs sequentially.
- **`TALLY_CALC_THREADING`:** unknown values mean `Auto` and follow the pref. The result is cached in an atomic.
- **Prefs:** `multithreading` handles non-JSON, a non-object, a missing key and a non-bool value by returning `true`. `set_prefs` applies the pref only after `store` has validated the JSON object and its size cap. The webview can toggle threading, which has no security impact.
- **Pre-pass isolation:** RAND and `MULTIPLE.OPERATIONS` bail for good (`rand`, `table_op`), so the shared RNG stream and the per-recalc table-op caps stay sequential. `wb_mut()` is `None` for `WbAccess::Par`, so no write path is reachable.

Probe source (not committed): `scratchpad/review7/head/crates/tally-engine/tests/probe7.rs`. The tests are `probe_deferral`, `probe_deferral_open`, `probe_edges`, `probe_branch_steer` and `probe_lookup_skip`; the parent copy holds `probe_deferral` and `probe_edges` only.
