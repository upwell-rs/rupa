//! RUPA — Rust Unified Persistence API.
//!
//! Facade crate: re-exports the standalone API. Everything beyond the sync,
//! statically typed core is opt-in through cargo features, and nothing is
//! compiled that wasn't asked for:
//!
//! | feature | adds |
//! |---|---|
//! | `chrono`, `uuid` (default) | date, time and UUID columns, in every enabled driver |
//! | `async` | async executors, transactions, pools and repositories |
//! | `tokio` | sync methods of async repositories block with `block_in_place` |
//! | `dynamic` | run-time driver selection and type erasure ([`registry()`], [`connect`], `Database`) |
//! | `postgres`, `postgres-sync`, `sqlite`, `sqlite-unbundled`, `mysql`, `memory`, `sqlx-*` | drivers |
//! | `tls-rustls` | TLS for every enabled networked driver |
//! | `migrations` | explicit SQL/Rust migration catalogs and native SQLite apply |
//! | `full` | every driver, plus `dynamic` and `migrations` |
//!
//! # Statically typed (the default)
//!
//! Use the driver's pool or executor directly: no erasure, no dynamic
//! dispatch per statement.
//!
//! ```ignore
//! let pool = rupa::drivers::sqlite::SqlitePool::connect(&DbConfig::new("sqlite://app.db"))?;
//! let users = UserRepositoryRepo::new(&pool);
//! ```
//!
//! Every pool, connection and transaction is a [`Connection`], and so is a
//! `&`, `&mut`, `Box`, `Rc` or `Arc` of one. A repository holds one:
//!
//! ```ignore
//! let tx = pool.begin()?;
//! NoteWritesRepo::new(&tx).add(&note)?;
//! NoteReadsRepo::new(&tx).all()?;
//! tx.commit()?;
//! ```
//!
//! # Chosen at run time (`dynamic`)
//!
//! ```ignore
//! let db: rupa::Database = rupa::connect(&DbConfig::new("sqlite://app.db")).await?;
//! let users = UserRepositoryRepo::new(&db);
//! ```
//!
//! [`registry()`] lists the drivers compiled in, in the order they are
//! tried for a URL scheme. See `rupa_core::registry` for the details and
//! [`rupa_core::config`] for the configuration format.

#![warn(missing_docs)]

#[doc(hidden)]
pub mod __macro_support;
#[cfg(feature = "dynamic")]
mod registry;

pub use rupa_core as core;
pub use rupa_dsl_std as dsl_std;
pub use rupa_macros as macros;
#[cfg(feature = "migrations")]
pub use rupa_migrate as migrate;
pub use rupa_sql as sql;

#[cfg(feature = "dynamic")]
pub use registry::{connect, registry};
pub use rupa_core::config::{
    DbConfig, DriverConfig, MySqlOptions, PoolConfig, PostgresOptions, SqliteOptions,
};
pub use rupa_core::dialect::{
    Capability, Dialect, DialectId, DynDialect, Memory, MySql, Postgres, Sqlite, Supports,
};
#[cfg(feature = "dynamic")]
pub use rupa_core::erased::{BoxAsyncExecutor, BoxAsyncTx, BoxExecutor, BoxTx};
pub use rupa_core::error::{
    ConnectError, DecodeError, DslError, DynError, ResultError, RowError, TxError,
};
#[cfg(feature = "async")]
pub use rupa_core::exec::{AsyncExecutor, Awaited, Blocking};
pub use rupa_core::exec::{ExecError, Executor, Immediate, now};
#[cfg(feature = "async")]
pub use rupa_core::pool::{AsyncConnect, AsyncPool, OwnedAsyncTx, SingleConnection};
pub use rupa_core::pool::{Connect, OwnedTx, Pool, PoolStatus, SyncConnect};
#[cfg(feature = "dynamic")]
pub use rupa_core::registry::{Database, Driver, Registry};
#[cfg(feature = "async")]
pub use rupa_core::security::AsyncSecureTransactional;
pub use rupa_core::security::{SecureTransactional, SecureTx, SecurityContext};
#[cfg(feature = "async")]
pub use rupa_core::tx::{AsyncTransaction, AsyncTransactional, AsyncTx, AsyncTxConnection};
pub use rupa_core::tx::{IsolationLevel, Transaction, Transactional, Tx, TxConnection, TxOptions};
pub use rupa_core::{
    Column, Connection, Deletable, DynRepository, Entity, Expr, ExprOps, FromRow, Gettable,
    Insertable, Json, Keyed, NoKey, Query, Reference, Scalar, SqlType, Updatable, Value, bind,
    delete, get, insert, json, raw, select, update,
};
pub use rupa_macros::{
    Deletable, Entity, Gettable, Insertable, Joined, Updatable, on, query, repository,
};

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
    #[cfg(feature = "sqlite-unbundled")]
    pub use rupa_driver_rusqlite as sqlite;
    #[cfg(any(
        feature = "sqlx-postgres",
        feature = "sqlx-mysql",
        feature = "sqlx-sqlite",
        feature = "sqlx-sqlite-unbundled"
    ))]
    pub use rupa_driver_sqlx as sqlx;
    #[cfg(feature = "postgres")]
    pub use rupa_driver_tokio_postgres as postgres;
}

/// The names most code needs: `use rupa::prelude::*`.
pub mod prelude {
    pub use rupa_core::error::DynError;
    pub use rupa_core::prelude::*;
    #[cfg(feature = "async")]
    pub use rupa_core::tx::{AsyncTransaction, AsyncTransactional};
    pub use rupa_core::tx::{Transaction, Transactional, TxOptions};
    pub use rupa_core::{Connection, DbConfig};
    pub use rupa_macros::{
        Deletable, Entity, Gettable, Insertable, Joined, Updatable, on, query, repository,
    };
    #[cfg(feature = "migrations")]
    #[allow(
        unused_imports,
        reason = "empty unless exactly one migration mode is enabled"
    )]
    pub use rupa_migrate::prelude::*;
}
