//! The registry entry, for run-time driver selection.

use crate::SqlitePool;

rupa_core::blocking_driver! {
    /// The registry entry for this driver: name `sqlite`, scheme `sqlite`.
    ///
    /// Builds a [`SqlitePool`], served to async code through
    /// [`Blocking`](rupa_core::exec::Blocking) (statements run on the calling
    /// thread).
    pub struct SqliteDriver {
        name: "sqlite",
        dialect: Sqlite,
        schemes: ["sqlite"],
        pool: SqlitePool,
    }
}
