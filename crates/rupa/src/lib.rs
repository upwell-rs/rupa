//! RUPA — Rust Unified Persistence API.
//!
//! Facade crate: re-exports the standalone API. DI integrations are opt-in
//! via cargo features and never compiled by default.

pub use rupa_core as core;
pub use rupa_dsl_std as dsl_std;
pub use rupa_macros as macros;
pub use rupa_sql as sql;

pub use rupa_core::exec::DynError;
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

pub mod prelude {
    pub use rupa_core::Repo;
    pub use rupa_core::prelude::*;
    pub use rupa_macros::{Deletable, Entity, Gettable, Insertable, Updatable, query, repository};
}
