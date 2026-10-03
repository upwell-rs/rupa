//! Sans-IO core of RUPA: values, the query IR, the typed builder, and the
//! entity, row and dialect traits. Nothing here touches a connection.

pub mod builder;
pub mod column;
pub mod dialect;
pub mod entity;
pub mod error;
pub mod expr;
pub mod ir;
pub mod query;
pub mod row;
pub mod value;

#[doc(hidden)]
pub use column::__private;

pub use builder::{delete, insert, raw, select, update};
pub use column::{Column, ColumnKind, ColumnMeta, Json, Scalar};
pub use dialect::{Capability, Dialect, DialectId, DynDialect, Postgres, Supports};
pub use entity::{Entity, FromRow, IdValues};
pub use error::{DecodeError, DslError, ResultError, RowError};
pub use expr::{Expr, ExprOps, IntoExpr, bind};
pub use query::{AffectedResult, Expect, Output, Query, QueryResult, RowsResult};
pub use row::{Row, RowCursor};
pub use value::{ScalarColumn, SqlType, Value};

pub mod prelude {
    pub use crate::col;
    pub use crate::{
        Column, Entity, Expr, ExprOps, FromRow, IntoExpr, Json, Query, Scalar, bind, delete,
        insert, raw, select, update,
    };
}
