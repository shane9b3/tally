//! Managed application state.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use tally_api::{ApiError, DocumentService};

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

/// Locks a mutex, mapping poisoning to an `internal` [`ApiError`].
pub fn lock<'a, T: ?Sized>(m: &'a Mutex<T>, what: &str) -> Result<MutexGuard<'a, T>, ApiError> {
    m.lock()
        .map_err(|_| ApiError::new(ApiError::INTERNAL, format!("{what} lock poisoned")))
}
