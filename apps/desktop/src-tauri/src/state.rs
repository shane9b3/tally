//! Managed application state.
//!
//! # Locking
//!
//! Every open document has its own engine service ([`EngineService::fork`]
//! of [`AppState::hub`]) behind its own mutex, so a slow open, recalc, save
//! or print layout in one document never blocks a command on another. The
//! forks share document ids, the internal clipboard and print jobs through
//! the engine's own leaf locks (`tally_engine::service::Shared`).
//!
//! Lock order (a lock may only be taken while holding locks *above* it):
//!
//! 1. one document's service (a [`DocSlot`]): at most one at a time; no code
//!    path holds two documents' locks, and none may be added without a fixed
//!    order (ascending doc id);
//! 2. `paths`, then `recent`, then `allowed` (small maps);
//! 3. leaf locks, held only for a lookup, insert, remove or clone and never
//!    while taking another lock: `hub` (to fork), `docs` (to find or file a
//!    document's slot), and the engine's shared clipboard / print-job locks.
//!
//! In particular the `docs` registry lock is never held while a document is
//! locked or an engine call runs: [`AppState::doc`] clones the slot's `Arc`
//! and drops the registry guard before the caller locks the slot.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tally_api::{ApiError, DocId};
use tally_engine::EngineService;

/// One document's engine service and its lock.
pub type DocSlot = Arc<Mutex<EngineService>>;

/// Everything the commands share. See the module docs for the lock order.
pub struct AppState {
    /// The configured engine (`tally_engine::EngineService`; see `lib.rs`).
    /// It holds no documents: each document is opened or created on a fork
    /// of it ([`AppState::detached`]), which then lives in `docs`.
    hub: Mutex<EngineService>,
    /// Open documents (doc id → its own service). Leaf lock.
    docs: Mutex<HashMap<u32, DocSlot>>,
    /// Recently opened or saved files, most recent first.
    pub recent: Mutex<Vec<String>>,
    /// Paths the user picked in a native dialog this session (open allowlist).
    pub allowed: Mutex<HashSet<PathBuf>>,
    /// Current file of each open document (doc id → path), for Save.
    pub paths: Mutex<HashMap<u32, PathBuf>>,
}

impl AppState {
    /// Wraps a configured engine (codec, ⌘K provider, open limits).
    pub fn new(service: EngineService) -> Self {
        Self {
            hub: Mutex::new(service),
            docs: Mutex::new(HashMap::new()),
            recent: Mutex::new(Vec::new()),
            allowed: Mutex::new(HashSet::new()),
            paths: Mutex::new(HashMap::new()),
        }
    }

    /// A fresh service with no documents, sharing ids, clipboard and print
    /// jobs with every document's service. Nothing stays locked: the caller
    /// owns it (to open a document into it, or for document-free calls such
    /// as the function list or a print job's pages).
    pub fn detached(&self) -> EngineService {
        lock(&self.hub).fork()
    }

    /// The service of `doc` (the registry lock is released on return).
    pub fn doc(&self, doc: DocId) -> Result<DocSlot, ApiError> {
        lock(&self.docs)
            .get(&doc.0)
            .cloned()
            .ok_or_else(|| ApiError::not_found(format!("no document {}", doc.0)))
    }

    /// Files `svc` as the service of `doc` (just opened or created on it).
    pub fn insert_doc(&self, doc: DocId, svc: EngineService) {
        lock(&self.docs).insert(doc.0, Arc::new(Mutex::new(svc)));
    }

    /// Forgets the service of `doc`. A command still running on it keeps
    /// its own `Arc` and finishes; later ones get `not_found`.
    pub fn remove_doc(&self, doc: DocId) {
        lock(&self.docs).remove(&doc.0);
    }
}

/// Locks a mutex, recovering from poisoning.
///
/// Engine calls run under `catch_unwind` (`commands::with_doc`), so a panic
/// in one command is turned into an `internal` error before the guard is
/// dropped and a document's mutex is never poisoned that way. Poisoning can
/// still come from a panic elsewhere while a lock is held (the small
/// path/recent/allowlist maps, or a panic while unwinding). Recovering is
/// safe: at worst the state touched by the one failed call is inconsistent
/// (for the engine, one document), while refusing every later command
/// disabled the app and lost unsaved work.
pub fn lock<T: ?Sized>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}
