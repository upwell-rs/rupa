//! Sans-IO core of RUPA: values, the query IR, the typed builder, and the
//! entity, row and dialect traits. Nothing here touches a connection.

pub mod builder;
pub mod capability;
pub mod column;
pub mod dialect;
pub mod entity;
pub mod error;
pub mod exec;
pub mod expr;
pub mod ir;
pub mod query;
pub mod repo;
pub mod row;
pub mod security;
pub mod sem;
pub mod tx;
pub mod value;

#[doc(hidden)]
pub use column::__private;

/// Paths used by `rupa-macros` output. Not public API.
#[doc(hidden)]
pub mod __macro_support {
    pub use crate::__private::*;
    pub use crate::builder::{delete, get, insert, raw, select, update};
    pub use crate::capability::{Deletable, Gettable, Insertable, Keyed, NoKey, Updatable, key_of};
    pub use crate::column::{Column, ColumnMeta, Json, Scalar};
    pub use crate::dialect::{Capability, CapabilityMarker, Dialect, DslAvailable, Supports, caps};
    pub use crate::entity::{Entity, FromRow, IdValues};
    pub use crate::error::DslError;
    pub use crate::error::ResultError;
    pub use crate::expr::{Expr, IntoExpr};
    pub use crate::expr::{ExprOps, IntoCondition};
    pub use crate::ir::{BinOp, DslFnDef, ExprNode, TableRef};
    pub use crate::query::Query;
    pub use crate::row::Row;
    pub use crate::value::Value;
    pub use ::std::clone::Clone;
    pub use ::std::option::Option;
    pub use ::std::result::Result;
    pub use ::std::sync::OnceLock;
    pub use ::std::vec::Vec;
}

pub use builder::{delete, get, insert, raw, select, update};
pub use capability::{Deletable, Gettable, Insertable, Keyed, NoKey, RowKey, Updatable};
pub use column::{Column, ColumnKind, ColumnMeta, Json, Scalar};
pub use dialect::{
    Capability, Dialect, DialectId, DslAvailable, DynDialect, Memory, MySql, Postgres, Sqlite,
    Supports,
};
pub use entity::{Entity, FromRow, IdValues};
pub use error::{DecodeError, DslError, ResultError, RowError, TxError};
pub use exec::{
    AsyncExecutor, Blocking, BoxAsyncExecutor, BoxExecutor, DynError, ExecError, Executor, Outcome,
};
pub use expr::{Expr, ExprOps, IntoCondition, IntoExpr, JsonPath, bind, json};
pub use query::{AffectedResult, Expect, Output, Query, QueryResult, RowsResult};
pub use repo::{Acquire, AcquireAsync, Repo, Shared, SharedAsync};
pub use row::{Row, RowCursor};
pub use security::{AsyncSecureTransactional, SecureTransactional, SecureTx, SecurityContext};
pub use tx::{
    AsyncTransaction, AsyncTransactional, IsolationLevel, Transaction, Transactional, TxOptions,
};
pub use value::{ScalarColumn, SqlType, Value};

pub mod prelude {
    pub use crate::col;
    pub use crate::{
        Column, Deletable, Entity, Expr, ExprOps, FromRow, Gettable, Insertable, IntoExpr, Json,
        Query, Scalar, Updatable, bind, delete, get, insert, raw, select, update,
    };
}
