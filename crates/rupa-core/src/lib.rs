//! Sans-IO core of RUPA: values, the query IR, the typed builder, and the
//! entity, row and dialect traits. Nothing here touches a connection.

#![warn(missing_docs)]

pub mod builder;
pub mod capability;
#[cfg(feature = "chrono")]
pub mod chrono_text;
pub mod column;
pub mod config;
pub mod connection;
pub mod dialect;
/// Entities: tables, their ids and row decoding.
pub mod entity;
#[cfg(feature = "dynamic")]
pub mod erased;
/// Error types shared by the core and drivers.
pub mod error;
pub mod exec;
pub mod expr;
pub mod ir;
pub mod join;
pub mod pool;
pub mod query;
#[cfg(all(feature = "dynamic", feature = "async"))]
pub mod registry;
pub mod repo;
pub mod row;
pub mod schema;
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
    pub use crate::column::ReadAs;
    pub use crate::column::{
        Column, ColumnKind, ColumnMeta, Encoding, Generated, Json, JsonCodec, Kinded,
        NullableJsonCodec, Reference, Scalar, ScalarCodec, Stored,
    };
    pub use crate::dialect::{Capability, CapabilityMarker, Dialect, DslAvailable, Supports, caps};
    pub use crate::entity::{Entity, FromRow, IdValues};
    pub use crate::error::ResultError;
    pub use crate::error::{DslError, DynError};
    pub use crate::exec::ExecError;
    pub use crate::expr::{Expr, IntoExpr};
    pub use crate::expr::{ExprOps, IntoCondition};
    pub use crate::ir::{BinOp, DslFnDef, ExprNode, TableRef};
    pub use crate::ir::{ColumnRef, SourceId};
    pub use crate::join::{
        Admits, At, Fwd, JoinPart, JoinTo, Joined, Opt, PartOf, Plain, RefersTo, Rel, Rev,
        relation_on, source_col,
    };
    pub use crate::query::Query;
    pub use crate::row::Row;
    pub use crate::value::Value;
    pub use ::std::clone::Clone;
    pub use ::std::option::Option;
    pub use ::std::result::Result;
    pub use ::std::vec::Vec;
}

pub use builder::{delete, get, insert, raw, select, update};
pub use capability::{Deletable, Gettable, Insertable, Keyed, NoKey, RowKey, Updatable};
pub use column::{Column, ColumnKind, ColumnMeta, Generated, Json, ReadAs, Reference, Scalar};
pub use config::{DbConfig, PoolConfig};
pub use connection::{Connection, OpenTransaction, TxEnd};
pub use dialect::{
    Capability, Dialect, DialectId, DslAvailable, DynDialect, Memory, MySql, Postgres, Sqlite,
    Supports,
};
pub use entity::{Entity, FromRow, IdValues};
#[cfg(all(feature = "dynamic", feature = "async"))]
pub use erased::{BoxAsyncExecutor, BoxAsyncTx};
#[cfg(feature = "dynamic")]
pub use erased::{BoxExecutor, BoxTx};
pub use error::{ConnectError, DecodeError, DslError, DynError, ResultError, RowError, TxError};
#[cfg(feature = "async")]
pub use exec::{AsyncExecutor, Awaited, Blocking};
pub use exec::{Completion, Engine, ExecError, Executed, Executor, Immediate, Outcome, now};
pub use expr::{Expr, ExprOps, IntoCondition, IntoExpr, JsonPath, bind, json};
pub use join::{
    Admits, At, Fwd, Has, Inner, IntoJoined, JoinBuilder, JoinOnto, JoinPart, JoinTo, Joined, Left,
    OffsetRow, Opt, PartOf, PendingJoin, Plain, RefersTo, Rel, Rev, Source, Sources, Src,
    source_col,
};
#[cfg(feature = "async")]
pub use pool::{AsyncConnect, AsyncPool, OwnedAsyncTx, SingleConnection};
pub use pool::{Connect, OwnedTx, Pool, PoolStatus, Pooled, SyncConnect};
pub use query::{AffectedResult, Expect, Output, Query, QueryResult, RowsResult};
#[cfg(all(feature = "dynamic", feature = "async"))]
pub use registry::{Database, Driver, Registry};
pub use repo::DynRepository;
pub use row::{Row, RowCursor};
#[cfg(feature = "async")]
pub use security::AsyncSecureTransactional;
pub use security::{SecureTransactional, SecureTx, SecurityContext};
#[cfg(feature = "async")]
pub use tx::{AsyncTransaction, AsyncTransactional, AsyncTx, AsyncTxConnection};
pub use tx::{IsolationLevel, Transaction, Transactional, Tx, TxConnection, TxOptions};
pub use value::{ScalarColumn, SqlType, Value};

/// The names most code needs: `use rupa_core::prelude::*`.
pub mod prelude {
    pub use crate::{
        Column, Deletable, Entity, Expr, ExprOps, FromRow, Gettable, Insertable, IntoExpr, Json,
        Query, Scalar, Updatable, bind, delete, get, insert, raw, select, update,
    };
}
