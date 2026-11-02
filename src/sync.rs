//! Locking without panics (see `agent.md`, "Never crash").

use std::sync::{Mutex, MutexGuard, PoisonError};

/// Locks `mutex`. A poisoned lock means a panic in another task while it
/// held the lock; the data is still usable, and a panic here would only
/// spread it.
pub fn lock<T: ?Sized>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
