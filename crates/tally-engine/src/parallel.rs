//! Multithreaded recalculation: the parallel pre-pass of
//! [`crate::recalc`] and its settings (design and correctness argument in
//! docs/recalc-parallel.md).
//!
//! The pre-pass evaluates dirty formulas on a rayon pool in dataflow order
//! (a formula runs once all its dirty precedents have results) with a
//! read-only [`CalcCtx`]. A formula that reads anything not final, calls
//! RAND/RANDBETWEEN or `MULTIPLE.OPERATIONS`, bails; it and everything
//! downstream of it are left to the sequential pass, which then runs
//! unchanged and takes the pre-computed results as it reaches them.
//!
//! Without the `parallel` feature (targets without threads, such as
//! wasm32) everything here is a stub and recalc is always sequential.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

#[cfg(feature = "parallel")]
use crate::formula_cells::FormulaCellId;
use crate::recalc::RecalcBudget;
#[cfg(feature = "parallel")]
use crate::recalc::{CalcCtx, ParCache, ParShared};
use crate::workbook::Workbook;
#[cfg(feature = "parallel")]
use std::collections::HashMap;
#[cfg(feature = "parallel")]
use std::sync::Mutex;

/// Fewest dirty formulas for which [`Threading::Auto`] runs the parallel
/// pre-pass; smaller recalcs (an edit with a few hundred dependents) take
/// the sequential path with no extra work.
pub const PAR_MIN_FORMULAS: usize = 4096;

/// Most threads of the recalc pool.
pub const MAX_THREADS: usize = 16;

/// Stack size of the recalc pool's threads (formula nesting and name
/// expansion recurse).
pub const THREAD_STACK: usize = 8 << 20;

/// How one workbook recalculates. Not saved in files.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum Threading {
    /// The app setting ([`set_enabled`], `TALLY_CALC_THREADING`) and the
    /// size threshold [`PAR_MIN_FORMULAS`] decide.
    #[default]
    Auto,
    /// Always single-threaded.
    Sequential,
    /// The parallel pre-pass whenever the build supports it, whatever the
    /// size and the app setting (differential tests).
    Parallel,
}

/// The app preference ("Enable multi-threaded calculation"), on by default.
static ENABLED: AtomicBool = AtomicBool::new(true);

/// Turns multithreaded recalculation on or off for the whole process (the
/// app preference). `TALLY_CALC_THREADING` overrides it.
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

/// The app preference as last set (see [`set_enabled`]).
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// `TALLY_CALC_THREADING`: 0 unset or unknown, 1 `off`, 2 `on`, 3 `force`.
static ENV_MODE: AtomicU8 = AtomicU8::new(u8::MAX);

fn env_mode() -> u8 {
    let m = ENV_MODE.load(Ordering::Relaxed);
    if m != u8::MAX {
        return m;
    }
    let m = match std::env::var("TALLY_CALC_THREADING")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "off" | "0" | "false" | "sequential" => 1,
        "on" | "1" | "true" => 2,
        "force" | "always" => 3,
        _ => 0,
    };
    ENV_MODE.store(m, Ordering::Relaxed);
    m
}

/// Threads the recalc pool has (or would have): `TALLY_CALC_THREADS` if
/// set, else the available parallelism, capped at [`MAX_THREADS`]; 1
/// without the `parallel` feature.
pub fn threads() -> usize {
    #[cfg(feature = "parallel")]
    {
        pool().map_or(1, rayon::ThreadPool::current_num_threads)
    }
    #[cfg(not(feature = "parallel"))]
    {
        1
    }
}

#[cfg(feature = "parallel")]
fn wanted_threads() -> usize {
    let env = std::env::var("TALLY_CALC_THREADS")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|n| *n > 0);
    let n = env.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(std::num::NonZeroUsize::get)
            .unwrap_or(1)
    });
    n.clamp(1, MAX_THREADS)
}

/// The recalc pool, built on first use (`None` if it cannot be built).
#[cfg(feature = "parallel")]
fn pool() -> Option<&'static rayon::ThreadPool> {
    static POOL: std::sync::OnceLock<Option<rayon::ThreadPool>> = std::sync::OnceLock::new();
    POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(wanted_threads())
            .stack_size(THREAD_STACK)
            .thread_name(|i| format!("tally-calc-{i}"))
            .build()
            .ok()
    })
    .as_ref()
}

// Workers share `&Workbook`.
#[cfg(feature = "parallel")]
const _: fn() = || {
    fn sync<T: Sync>() {}
    sync::<Workbook>();
    sync::<ParShared>();
};

/// Whether a recalc of `dirty` formulas of `wb` within `budget` runs the
/// parallel pre-pass.
pub(crate) fn wanted(wb: &Workbook, dirty: usize, budget: &RecalcBudget) -> bool {
    if !cfg!(feature = "parallel") || dirty == 0 {
        return false;
    }
    #[cfg(test)]
    if crate::recalc::test_hook::active() {
        // The hook is thread-local: keep the hooked evaluation on this thread.
        return false;
    }
    // The formula budget must not run out inside the pre-pass, so the
    // sequential pass stops at the same formula as without it.
    if budget.max_cells_evaluated < dirty as u64 {
        return false;
    }
    let forced = match wb.threading {
        Threading::Sequential => return false,
        Threading::Parallel => true,
        Threading::Auto => match env_mode() {
            1 => return false,
            2 => false,
            3 => true,
            _ if !enabled() => return false,
            _ => false,
        },
    };
    if forced {
        return true;
    }
    dirty >= PAR_MIN_FORMULAS && threads() > 1
}

/// Runs the parallel pre-pass over `dirty` (the recalc's dirty formulas,
/// `preds[i]` the dirty precedents of `dirty[i]` as indices). Returns the
/// pre-computed results; `None` if the pool is not available.
#[cfg(feature = "parallel")]
pub(crate) fn prepass(
    wb: &mut Workbook,
    dirty: &[FormulaCellId],
    preds: &[Vec<usize>],
    pos: Vec<u32>,
    now: f64,
    budget: &RecalcBudget,
) -> Option<ParShared> {
    use std::sync::atomic::AtomicU32;
    use std::sync::OnceLock;

    use rayon::prelude::*;
    use slotmap::SecondaryMap;

    let pool = pool()?;
    let n = dirty.len();
    if preds.len() != n || pos.len() != n {
        return None;
    }
    // Dependents as compressed rows: `rev[off[u]..off[u + 1]]` read `u`.
    let mut off = vec![0u32; n + 1];
    for p in preds {
        for &u in p {
            off[u + 1] += 1;
        }
    }
    for i in 0..n {
        off[i + 1] += off[i];
    }
    let mut fill: Vec<u32> = off[..n].to_vec();
    let mut rev = vec![0u32; off[n] as usize];
    for (v, p) in preds.iter().enumerate() {
        for &u in p {
            rev[fill[u] as usize] = v as u32;
            fill[u] += 1;
        }
    }
    drop(fill);
    let pending: Vec<AtomicU32> = preds
        .iter()
        .map(|p| AtomicU32::new(p.len() as u32))
        .collect();
    let roots: Vec<u32> = (0..n as u32)
        .filter(|v| preds[*v as usize].is_empty())
        .collect();
    let mut local: SecondaryMap<FormulaCellId, usize> = SecondaryMap::new();
    for (i, id) in dirty.iter().enumerate() {
        local.insert(*id, i);
    }
    let mut slots = Vec::with_capacity(n);
    slots.resize_with(n, OnceLock::new);
    let shared = ParShared {
        local,
        pos,
        slots,
        cache: Mutex::new(ParCache {
            cache: wb.lookup_cache.take(),
            with_dirty: HashMap::new(),
        }),
    };
    {
        let run = Run {
            wb,
            sh: &shared,
            dirty,
            off: &off,
            rev: &rev,
            pending: &pending,
            now,
            deadline: budget.deadline,
            stop: AtomicBool::new(false),
            waiters: Mutex::new(HashMap::new()),
            has_waiters: AtomicBool::new(false),
            retries: (0..n)
                .map(|_| std::sync::atomic::AtomicU8::new(0))
                .collect(),
        };
        let run = &run;
        pool.install(|| {
            rayon::scope(|s| {
                roots.par_iter().for_each(|&v| process(s, v, run));
            });
        });
    }
    let ParShared {
        local,
        pos,
        slots,
        cache,
    } = shared;
    let ParCache {
        mut cache,
        with_dirty,
    } = cache
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // Indexes over pre-computed formulas stay out of the sequential pass:
    // a sequential lookup into such a line evaluates the line's formulas
    // first (on demand), which an index would skip.
    if let Some(c) = cache.as_mut() {
        for line in with_dirty.keys() {
            c.forget(line);
        }
    }
    wb.lookup_cache = cache;
    Some(ParShared {
        local,
        pos,
        slots,
        cache: Mutex::new(ParCache::default()),
    })
}

/// Without the `parallel` feature: never called ([`wanted`] is `false`).
#[cfg(not(feature = "parallel"))]
pub(crate) fn prepass(
    _wb: &mut Workbook,
    _dirty: &[crate::formula_cells::FormulaCellId],
    _preds: &[Vec<usize>],
    _pos: Vec<u32>,
    _now: f64,
    _budget: &RecalcBudget,
) -> Option<crate::recalc::ParShared> {
    None
}

/// What every pre-pass task reads.
#[cfg(feature = "parallel")]
struct Run<'a> {
    wb: &'a Workbook,
    sh: &'a ParShared,
    dirty: &'a [FormulaCellId],
    off: &'a [u32],
    rev: &'a [u32],
    pending: &'a [std::sync::atomic::AtomicU32],
    now: f64,
    deadline: Option<std::time::Instant>,
    /// The deadline passed: start nothing new.
    stop: AtomicBool,
    /// Bailed formulas waiting for a formula they read (by node).
    waiters: Mutex<HashMap<u32, Vec<u32>>>,
    /// `waiters` may be non-empty (checked without the lock).
    has_waiters: AtomicBool,
    /// Retries per node (at most [`MAX_RETRIES`]).
    retries: Vec<std::sync::atomic::AtomicU8>,
}

/// Most times one formula is retried after waiting for a precedent found
/// during its evaluation.
#[cfg(feature = "parallel")]
const MAX_RETRIES: u8 = 8;

#[cfg(feature = "parallel")]
impl<'a> Run<'a> {
    fn waiters(&self) -> std::sync::MutexGuard<'_, HashMap<u32, Vec<u32>>> {
        self.waiters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Parks node `v` until node `x` has a result; `false` if `x` already
    /// has one (retry now).
    ///
    /// Against [`Self::wake`]: `x` sets its slot, then (SeqCst fence) reads
    /// `has_waiters`; here `has_waiters` is set, then (fence) the slot is
    /// read, under the lock `wake` takes. One of the two sees the other's
    /// write, so a parked node is always woken.
    fn park(&self, v: u32, x: usize) -> bool {
        let mut w = self.waiters();
        self.has_waiters.store(true, Ordering::Relaxed);
        std::sync::atomic::fence(Ordering::SeqCst);
        if self.sh.slots[x].get().is_some() {
            return false;
        }
        w.entry(x as u32).or_default().push(v);
        true
    }

    /// Node `v` has its result: re-run the nodes waiting for it.
    fn wake(&'a self, s: &rayon::Scope<'a>, v: usize) {
        std::sync::atomic::fence(Ordering::SeqCst);
        if !self.has_waiters.load(Ordering::Relaxed) {
            return;
        }
        let list = self.waiters().remove(&(v as u32));
        for w in list.into_iter().flatten() {
            s.spawn(move |s| process(s, w, self));
        }
    }
}

/// Evaluates node `v`, then whatever it makes ready: the first ready
/// dependent on this thread (a chain is a loop, not a task per cell), the
/// others as new tasks.
#[cfg(feature = "parallel")]
fn process<'a>(s: &rayon::Scope<'a>, v: u32, run: &'a Run<'a>) {
    let mut v = v as usize;
    loop {
        if run.stop.load(Ordering::Relaxed) {
            return;
        }
        if run.deadline.is_some_and(|d| std::time::Instant::now() >= d) {
            run.stop.store(true, Ordering::Relaxed);
            return;
        }
        let mut ctx = CalcCtx::par(run.wb, run.sh, run.now);
        let Some(done) = ctx.eval_par(run.dirty[v]) else {
            // Bailed. If it read a formula of the dirty set that is still
            // to come (a precedent found only now, through INDIRECT or
            // OFFSET), it waits for that one and is retried; otherwise it
            // and everything downstream stay for the sequential pass
            // (their counters never reach zero).
            match ctx.retry_after() {
                Some(x) if run.retries[v].fetch_add(1, Ordering::Relaxed) < MAX_RETRIES => {
                    if run.park(v as u32, x) {
                        return;
                    }
                    // `x` finished meanwhile: retry at once.
                    continue;
                }
                _ => return,
            }
        };
        let _ = run.sh.slots[v].set(done);
        run.wake(s, v);
        let mut next: Option<usize> = None;
        let (a, b) = (run.off[v] as usize, run.off[v + 1] as usize);
        for &w in &run.rev[a..b] {
            if run.pending[w as usize].fetch_sub(1, Ordering::AcqRel) == 1 {
                if next.is_none() {
                    next = Some(w as usize);
                } else {
                    s.spawn(move |s| process(s, w, run));
                }
            }
        }
        match next {
            Some(w) => v = w,
            None => return,
        }
    }
}
