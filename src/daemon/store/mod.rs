//! SQLite storage — tasks, events, tool versions, parser registry.

#[allow(unused_imports)]
mod events;
mod prune;
mod schema;
mod tasks;
mod versions;

#[allow(unused_imports)]
pub use events::*;
pub use prune::*;
pub use schema::*;
pub use tasks::*;
pub use versions::*;

use arshy_lib::Result;
use std::path::Path;
use std::sync::Mutex;

/// Thread-safe SQLite store wrapper.
pub struct Store {
    conn: Mutex<rusqlite::Connection>,
}

impl Store {
    /// Open (or create) the SQLite database at `path`.
    pub fn open(path: &Path, wal_mode: bool) -> Result<Self> {
        let conn = rusqlite::Connection::open(path)?;
        if wal_mode {
            conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        }
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, rusqlite::Connection> {
        self.conn.lock().expect("store mutex poisoned")
    }
}
