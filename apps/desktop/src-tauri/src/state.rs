//! Managed application state.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, PoisonError};

use tally_api::DocumentService;

/// Everything the commands share. Lock order (when more than one is needed):
/// `service`, then `paths`, then `recent`, then `allowed`.
pub struct AppState {
    /// The engine (`tally_engine::EngineService`; see `lib.rs`).
    pub service: Mutex<Box<dyn DocumentService + Send>>,
    /// Recently opened or saved files, most recent first.
    pub recent: Mutex<Vec<String>>,
    /// Paths the user picked in a native dialog this session (open allowlist).
    pub allowed: Mutex<HashSet<PathBuf>>,
    /// Current file of each open document (doc id → path), for Save.
    pub paths: Mutex<HashMap<u32, PathBuf>>,
}

impl AppState {
    /// Wraps a service.
    pub fn new(service: Box<dyn DocumentService + Send>) -> Self {
        Self {
            service: Mutex::new(service),
            recent: Mutex::new(Vec::new()),
            allowed: Mutex::new(HashSet::new()),
            paths: Mutex::new(HashMap::new()),
        }
    }
}

/// Locks a mutex, recovering from poisoning.
///
/// Engine calls run under `catch_unwind` (`commands::with_service`), so a
/// panic in one command is turned into an `internal` error before the guard
/// is dropped and the service mutex is never poisoned that way. Poisoning
/// can still come from a panic elsewhere while a lock is held (the small
/// path/recent/allowlist maps, or a panic while unwinding). Recovering is
/// safe: at worst the state touched by the one failed call is inconsistent
/// (for the engine, one document), while refusing every later command, as
/// before, disabled the app and lost unsaved work in every other document.
pub fn lock<T: ?Sized>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}
