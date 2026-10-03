//! RUPA — Rust Unified Persistence API.
//!
//! Facade crate: re-exports the standalone API. DI integrations are opt-in
//! via cargo features and never compiled by default.

pub use rupa_core as core;
pub use rupa_dsl_std as dsl_std;
pub use rupa_macros as macros;
pub use rupa_sql as sql;

pub use rupa_core::{
    AsyncExecutor, Column, Deletable, Entity, Executor, Expr, ExprOps, FromRow, Gettable,
    Insertable, Json, NoKey, Query, Scalar, Updatable, bind, col, delete, get, insert, raw, select,
    update,
};
pub use rupa_macros::{Deletable, Entity, Gettable, Insertable, Updatable};

#[cfg(feature = "upwell")]
pub use rupa_upwell as upwell;

#[doc(hidden)]
pub use rupa_core::__macro_support;

pub mod prelude {
    pub use rupa_core::prelude::*;
    pub use rupa_macros::{Deletable, Entity, Gettable, Insertable, Updatable};
}
