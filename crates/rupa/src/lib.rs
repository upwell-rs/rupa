//! RUPA — Rust Unified Persistence API.
//!
//! Facade crate: re-exports the standalone API. Drivers and DI integrations
//! are opt-in via cargo features and never compiled by default.
//!
//! # Connecting from configuration
//!
//! Enable the drivers you need (`postgres`, `postgres-sync`, `sqlite`,
//! `mysql`, `memory`, the sqlx-based `sqlx-postgres`, `sqlx-mysql` and
//! `sqlx-sqlite`, or `full`) and connect with a [`DbConfig`]:
//!
//! ```ignore
//! let db: rupa::Database = rupa::connect(&DbConfig::new("sqlite://app.db")).await?;
//! let users: Arc<dyn UserRepository> = Arc::new(Repo::new(db.clone()));
//! ```
//!
//! [`registry()`] lists the drivers compiled in, in the order they are
//! tried for a URL scheme. See
//! [`rupa_core::registry`] for the details and [`rupa_core::config`] for the
//! configuration format.

#![warn(missing_docs)]

pub use rupa_core as core;
pub use rupa_dsl_std as dsl_std;
pub use rupa_macros as macros;
pub use rupa_sql as sql;

pub use rupa_core::config::{
    DbConfig, DriverConfig, MySqlOptions, PoolConfig, PostgresOptions, SqliteOptions,
};
pub use rupa_core::erased::{BoxAsyncExecutor, BoxAsyncTx, BoxExecutor, BoxTx, DynError};
pub use rupa_core::pool::{AsyncPool, Pool, PoolStatus};
pub use rupa_core::registry::{ConnectError, Database, Driver, Registry};
pub use rupa_core::security::{
    AsyncSecureTransactional, SecureTransactional, SecureTx, SecurityContext,
};
pub use rupa_core::tx::{
    AsyncTransaction, AsyncTransactional, IsolationLevel, Transaction, Transactional, TxOptions,
};
pub use rupa_core::{
    Acquire, AcquireAsync, AsyncExecutor, Column, Deletable, Entity, Executor, Expr, ExprOps,
    FromRow, Gettable, Insertable, Json, NoKey, Query, Repo, Scalar, Shared, SharedAsync,
    Updatable, bind, col, delete, get, insert, raw, select, update,
};
pub use rupa_macros::{Deletable, Entity, Gettable, Insertable, Updatable, query, repository};

/// DSL functions: the built-ins, and the macro to write your own.
///
/// ```ignore
/// use rupa::dsl::{self, Dialect, DialectId, DslError, Expr};
///
/// #[dsl::function(requires = Ilike)]          // or runtime-checked: branch on `dialect.id()`
/// fn starts_with_ci(dialect: &dyn Dialect, s: Expr<String>, prefix: Expr<String>) -> Result<Expr<bool>, DslError> { .. }
/// ```
pub mod dsl {
    pub use rupa_core::dialect::{Capability, Dialect, DialectId, DslAvailable, Supports, caps};
    pub use rupa_core::{DslError, Expr, Value, json};
    pub use rupa_dsl_std::*;
    pub use rupa_macros::function;
}

#[cfg(feature = "upwell")]
pub use rupa_upwell as upwell;

/// The driver crates enabled by cargo features.
pub mod drivers {
    #[cfg(feature = "memory")]
    pub use rupa_driver_memory as memory;
    #[cfg(feature = "mysql")]
    pub use rupa_driver_mysql as mysql;
    #[cfg(feature = "postgres-sync")]
    pub use rupa_driver_postgres as postgres_sync;
    #[cfg(feature = "sqlite")]
    pub use rupa_driver_rusqlite as sqlite;
    #[cfg(any(
        feature = "sqlx-postgres",
        feature = "sqlx-mysql",
        feature = "sqlx-sqlite"
    ))]
    pub use rupa_driver_sqlx as sqlx;
    #[cfg(feature = "postgres")]
    pub use rupa_driver_tokio_postgres as postgres;
}

/// The registry of every driver enabled by cargo features, built on first
/// use and kept for the life of the program (drivers are stateless unit
/// structs, so this is one small allocation, once).
///
/// Drivers are tried for a URL scheme in this order: `postgres` (async),
/// `sqlx-postgres`, `postgres-sync`, `sqlite`, `sqlx-sqlite`, `mysql`,
/// `sqlx-mysql`, `memory`. A config that names its driver
/// (`driver = "sqlx-mysql"`) skips the scheme lookup; if the named driver
/// isn't enabled, another one for the same database serves it (see
/// [`Registry::resolve`]).
///
/// The memory driver registered here creates databases without tables. To
/// register tables, or to add third-party drivers, start from a clone
/// (`rupa::registry().clone().with(..)`) or an empty
/// [`Registry::new`]; cloning shares the drivers.
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
        #[cfg(feature = "sqlite")]
        registry.register(rupa_driver_rusqlite::SqliteDriver);
        #[cfg(feature = "sqlx-sqlite")]
        registry.register(rupa_driver_sqlx::sqlite::SqliteDriver::new());
        #[cfg(feature = "mysql")]
        registry.register(rupa_driver_mysql::MySqlDriver);
        #[cfg(feature = "sqlx-mysql")]
        registry.register(rupa_driver_sqlx::mysql::MySqlDriver::new());
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

/// Paths used by `rupa-macros` output. Not public API.
#[doc(hidden)]
pub mod __macro_support {
    pub use ::std::boxed::Box;
    pub use ::std::future::Future;
    pub use ::std::marker::Send;
    pub use ::std::pin::Pin;
    pub use rupa_core::__macro_support::*;
    pub use rupa_core::exec::{AsyncExecutor, Executor};
    pub use rupa_core::repo::{Acquire, AcquireAsync, Repo};

    /// Runs `future` to completion from a sync repository method of an async
    /// repository.
    pub fn block_in_place<F: Future>(future: F) -> F::Output {
        #[cfg(feature = "tokio")]
        if let Ok(handle) = tokio::runtime::Handle::try_current()
            && handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread
        {
            return tokio::task::block_in_place(|| handle.block_on(future));
        }
        pollster::block_on(future)
    }
}

/// The names most code needs: `use rupa::prelude::*`.
pub mod prelude {
    pub use rupa_core::Repo;
    pub use rupa_core::prelude::*;
    pub use rupa_core::tx::{
        AsyncTransaction, AsyncTransactional, Transaction, Transactional, TxOptions,
    };
    pub use rupa_macros::{Deletable, Entity, Gettable, Insertable, Updatable, query, repository};
}
