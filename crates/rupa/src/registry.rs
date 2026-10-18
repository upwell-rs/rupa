//! The registry of the drivers enabled by cargo features.

use rupa_core::config::DbConfig;
use rupa_core::error::ConnectError;
use rupa_core::registry::{Database, Registry};

/// The registry of every driver enabled by cargo features, built on first
/// use and kept for the life of the program (drivers are stateless unit
/// structs, so this is one small allocation, once).
///
/// Drivers are tried for a URL scheme in this order: `postgres` (async),
/// `sqlx-postgres`, `postgres-sync`, `sqlite`, `sqlx-sqlite`, `sqlx-mysql`,
/// `mysql`, `memory`.
///
/// The order follows the `drivers` benchmark (`docs/benchmarks.md`). For
/// networked databases, async drivers come first: a sync driver blocks a
/// runtime worker for every round trip, and `postgres-sync` can't run inside
/// tokio at all. For SQLite, rusqlite comes first: it is 3–6× faster per
/// statement than sqlx-sqlite, and blocking for a few microseconds costs
/// little. A config that names its driver (`driver = "sqlx-mysql"`) skips
/// the scheme lookup; if the named driver isn't enabled, another one for the
/// same database serves it (see [`Registry::resolve`]).
///
/// The memory driver registered here creates databases without tables. To
/// register tables, or to add third-party drivers, start from a clone
/// (`rupa::registry().clone().with(..)`) or an empty [`Registry::new`];
/// cloning shares the drivers.
pub fn registry() -> &'static Registry {
    static REGISTRY: std::sync::OnceLock<Registry> = std::sync::OnceLock::new();

    REGISTRY.get_or_init(|| {
        #[allow(unused_mut)]
        let mut registry = Registry::new();

        #[cfg(feature = "postgres")]
        registry.register(rupa_driver_tokio_postgres::PgDriver);
        #[cfg(feature = "sqlx-postgres")]
        registry.register(rupa_driver_sqlx::postgres::PgDriver::new());
        #[cfg(feature = "postgres-sync")]
        registry.register(rupa_driver_postgres::PgDriver);
        #[cfg(feature = "sqlite-unbundled")]
        registry.register(rupa_driver_rusqlite::SqliteDriver);
        #[cfg(any(feature = "sqlx-sqlite", feature = "sqlx-sqlite-unbundled"))]
        registry.register(rupa_driver_sqlx::sqlite::SqliteDriver::new());
        #[cfg(feature = "sqlx-mysql")]
        registry.register(rupa_driver_sqlx::mysql::MySqlDriver::new());
        #[cfg(feature = "mysql")]
        registry.register(rupa_driver_mysql::MySqlDriver);
        #[cfg(feature = "memory")]
        registry.register(rupa_driver_memory::MemoryDriver::new());

        registry
    })
}

/// Connects to the database `config` describes, with the drivers in
/// [`registry()`].
pub async fn connect(config: &DbConfig) -> Result<Database, ConnectError> {
    registry().connect(config).await
}
