//! Joins: source handles, the join builder, and decoding joined rows.
//!
//! [`SelectBuilder::inner_join`] and [`SelectBuilder::left_join`] turn a
//! select into a [`JoinBuilder`]. Its closures (`on`, `filter`, `order_by`)
//! receive one [`Src`] handle per row source, in join order, and columns are
//! taken through them, so every column names its source:
//!
//! ```ignore
//! let rows: Vec<(User, Option<Order>)> = select::<User>()
//!     .left_join::<Order>(|u, o| o.col(col!(Order::user_id)).eq(u.col(col!(User::id))))
//!     .filter(|u, _| u.col(col!(User::active)).eq(true))
//!     .all()
//!     .run(&mut db)?;
//! ```
//!
//! [`Src::col`] only accepts columns of its own entity. A column used without
//! a handle refers to the first source (the entity `select` started from).
//!
//! Rows decode into tuples: each element is an entity, or `Option<E>` for a
//! left-joined one, which is `None` when the join found no match. A join has
//! at most 10 sources.

use std::fmt;
use std::marker::PhantomData;

use crate::builder::{SelectBuilder, and_filter, count_param};
use crate::column::Column;
use crate::entity::{Entity, FromRow};
use crate::error::{ResultError, RowError};
use crate::expr::Expr;
use crate::ir::{
    ColumnRef, ExprNode, FromClause, Join, JoinKind, OrderBy, Select, SourceId, Statement,
};
use crate::query::{Expect, Query, RowsResult};
use crate::row::Row;
use crate::value::{SqlType, Value};

/// Handle to row source `N` of a join, an `E` table. Join closures receive
/// one per source; take columns through it with [`col`](Self::col).
pub struct Src<E, const N: u16>(PhantomData<fn() -> E>);

impl<E, const N: u16> Clone for Src<E, N> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<E, const N: u16> Copy for Src<E, N> {}

impl<E, const N: u16> fmt::Debug for Src<E, N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Src<{}, {N}>", std::any::type_name::<E>())
    }
}

impl<E, const N: u16> Src<E, N> {
    /// This source's id within the statement.
    pub const ID: SourceId = SourceId(N);

    const fn new() -> Self {
        Self(PhantomData)
    }

    /// `column`, of this source. Only columns of `E` are accepted, so a
    /// column can't be read from the wrong table.
    pub const fn col<T, K>(&self, column: Column<E, T, K>) -> Column<E, T, K> {
        column.of_source(SourceId(N))
    }
}

/// A row source handle; implemented by [`Src`].
pub trait Source: Copy {
    /// The source's entity.
    type Entity: Entity;
    /// The source's id within the statement.
    const ID: SourceId;
    /// The handle.
    fn handle() -> Self;
}

impl<E: Entity, const N: u16> Source for Src<E, N> {
    type Entity = E;
    const ID: SourceId = SourceId(N);
    fn handle() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------

/// A view of `row` starting at column `base`, so an entity's `from_row` (which
/// reads its columns from 0) can decode its part of a joined row.
#[derive(Clone, Copy)]
pub struct OffsetRow<'a> {
    row: &'a dyn Row,
    base: usize,
}

impl<'a> OffsetRow<'a> {
    /// `row`, from column `base` on.
    pub fn new(row: &'a dyn Row, base: usize) -> Self {
        Self { row, base }
    }
}

impl fmt::Debug for OffsetRow<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OffsetRow")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

impl Row for OffsetRow<'_> {
    fn len(&self) -> usize {
        self.row.len().saturating_sub(self.base)
    }

    fn get(&self, index: usize, ty: SqlType) -> Result<Value, RowError> {
        self.row.get(self.base + index, ty)
    }
}

/// One element of a joined row: an entity, or `Option<E>` for a left-joined
/// entity. Tuples of these implement [`FromRow`].
pub trait JoinPart: Sized {
    /// The number of columns this part reads.
    fn width() -> usize;
    /// Decodes this part from the columns of `row` starting at `base`.
    fn decode(row: &dyn Row, base: usize) -> Result<Self, ResultError>;
}

impl<E: Entity> JoinPart for E {
    fn width() -> usize {
        E::columns().len()
    }

    fn decode(row: &dyn Row, base: usize) -> Result<Self, ResultError> {
        E::from_row(&OffsetRow::new(row, base))
    }
}

/// `None` when every id column is `NULL`: an id is never `NULL` in a stored
/// row, so that is a left join that found no match.
impl<E: Entity> JoinPart for Option<E> {
    fn width() -> usize {
        E::columns().len()
    }

    fn decode(row: &dyn Row, base: usize) -> Result<Self, ResultError> {
        let columns = E::columns();
        for id in E::ID_COLUMNS {
            let index = columns
                .iter()
                .position(|c| c.name == *id)
                .expect("id columns are columns of the entity");
            if !row.get(base + index, columns[index].sql_type)?.is_null() {
                return E::decode(row, base).map(Some);
            }
        }
        Ok(None)
    }
}

macro_rules! tuple_from_row {
    ($($R:ident),+) => {
        impl<$($R: JoinPart),+> FromRow for ($($R,)+) {
            #[allow(unused_assignments, reason = "the last part's width is not needed")]
            fn from_row(row: &dyn Row) -> Result<Self, ResultError> {
                let mut base = 0;
                Ok(($({
                    let part = $R::decode(row, base)?;
                    base += $R::width();
                    part
                },)+))
            }
        }
    };
}

tuple_from_row!(R0, R1);
tuple_from_row!(R0, R1, R2);
tuple_from_row!(R0, R1, R2, R3);
tuple_from_row!(R0, R1, R2, R3, R4);
tuple_from_row!(R0, R1, R2, R3, R4, R5);
tuple_from_row!(R0, R1, R2, R3, R4, R5, R6);
tuple_from_row!(R0, R1, R2, R3, R4, R5, R6, R7);
tuple_from_row!(R0, R1, R2, R3, R4, R5, R6, R7, R8);
tuple_from_row!(R0, R1, R2, R3, R4, R5, R6, R7, R8, R9);

// ---------------------------------------------------------------------------
// Building
// ---------------------------------------------------------------------------

/// A select over joined sources being built. `S` is the tuple of source
/// handles the closures receive; `R` is the row type (a tuple of
/// [`JoinPart`]s). Finish with [`all`](Self::all), `optional`, `one` or
/// `exists`.
pub struct JoinBuilder<S, R> {
    select: Select,
    _p: PhantomData<fn() -> (S, R)>,
}

impl<S, R> Clone for JoinBuilder<S, R> {
    fn clone(&self) -> Self {
        Self::from_select(self.select.clone())
    }
}

impl<S, R> fmt::Debug for JoinBuilder<S, R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JoinBuilder")
            .field("row", &std::any::type_name::<R>())
            .field("select", &self.select)
            .finish()
    }
}

impl<S, R> JoinBuilder<S, R> {
    fn from_select(select: Select) -> Self {
        Self {
            select,
            _p: PhantomData,
        }
    }
}

/// Adds `J` as the next source: `kind JOIN j ON on`, and its columns to the
/// projection.
fn add_join<J: Entity>(mut select: Select, kind: JoinKind, on: ExprNode) -> Select {
    let source =
        SourceId(u16::try_from(select.from.source_count()).expect("a join has at most 10 sources"));
    let join = Join {
        kind,
        table: J::TABLE,
        on,
    };
    select.from = match select.from {
        FromClause::Table(base) => FromClause::Join {
            base,
            joins: vec![join],
        },
        FromClause::Join { base, mut joins } => {
            joins.push(join);
            FromClause::Join { base, joins }
        }
    };
    select
        .projection
        .extend(J::columns().iter().map(|c| ColumnRef::of(source, c.name)));
    select
}

impl<E: Entity> SelectBuilder<E> {
    #[allow(clippy::type_complexity, reason = "the sources and the row, by position")]
    /// `INNER JOIN`s `J`: each row pairs with every `J` row `on` accepts, and
    /// rows without a match are dropped. Rows decode as `(E, J)`.
    pub fn inner_join<J: Entity>(
        self,
        on: impl FnOnce(Src<E, 0>, Src<J, 1>) -> Expr<bool>,
    ) -> JoinBuilder<(Src<E, 0>, Src<J, 1>), (E, J)> {
        let on = on(Src::new(), Src::new()).into_node();
        JoinBuilder::from_select(add_join::<J>(self.select, JoinKind::Inner, on))
    }

    #[allow(clippy::type_complexity, reason = "the sources and the row, by position")]
    /// `LEFT JOIN`s `J`: like [`inner_join`](Self::inner_join), but rows
    /// without a match are kept, with `None` for `J`. Rows decode as
    /// `(E, Option<J>)`.
    pub fn left_join<J: Entity>(
        self,
        on: impl FnOnce(Src<E, 0>, Src<J, 1>) -> Expr<bool>,
    ) -> JoinBuilder<(Src<E, 0>, Src<J, 1>), (E, Option<J>)> {
        let on = on(Src::new(), Src::new()).into_node();
        JoinBuilder::from_select(add_join::<J>(self.select, JoinKind::Left, on))
    }
}

macro_rules! join_common {
    ($($S:ident $R:ident),+) => {
        impl<$($S: Source,)+ $($R: JoinPart,)+> JoinBuilder<($($S,)+), ($($R,)+)> {
            /// Adds a condition over the sources; repeated calls are ANDed.
            pub fn filter(mut self, cond: impl FnOnce($($S),+) -> Expr<bool>) -> Self {
                and_filter(&mut self.select.filter, cond($($S::handle()),+).into_node());
                self
            }

            /// Adds a sort key over the sources; repeated calls sort by each
            /// in turn.
            pub fn order_by(mut self, order: impl FnOnce($($S),+) -> OrderBy) -> Self {
                self.select.order_by.push(order($($S::handle()),+));
                self
            }

            /// Returns at most `n` rows.
            pub fn limit(mut self, n: u64) -> Self {
                self.select.limit = Some(count_param(n));
                self
            }

            /// Skips the first `n` rows.
            pub fn offset(mut self, n: u64) -> Self {
                self.select.offset = Some(count_param(n));
                self
            }

            /// Builds with the result shape taken from `Res`.
            pub fn build<Res: RowsResult<Item = ($($R,)+)>>(self) -> Query<Res> {
                Query::new(Statement::Select(self.select), Res::EXPECT)
            }

            /// Every matching row.
            pub fn all(self) -> Query<Vec<($($R,)+)>> {
                self.build()
            }

            /// At most one row; more is
            /// [`ResultError::TooManyRows`](crate::ResultError::TooManyRows).
            pub fn optional(self) -> Query<Option<($($R,)+)>> {
                self.build()
            }

            /// Exactly one row.
            pub fn one(self) -> Query<($($R,)+)> {
                self.build()
            }

            /// Whether any row matches. Ordering, limit and offset are discarded.
            pub fn exists(mut self) -> Query<bool> {
                self.select.order_by.clear();
                self.select.limit = None;
                self.select.offset = None;
                Query::new(Statement::Exists(self.select), Expect::Exists)
            }
        }
    };
}

macro_rules! join_next {
    ($($S:ident $R:ident),+ => $N:literal) => {
        impl<$($S: Source,)+ $($R: JoinPart,)+> JoinBuilder<($($S,)+), ($($R,)+)> {
            /// `INNER JOIN`s `J` as the next source; `on` may use every source.
            #[allow(clippy::type_complexity, reason = "the sources and the row, by position")]
            pub fn inner_join<J: Entity>(
                self,
                on: impl FnOnce($($S,)+ Src<J, $N>) -> Expr<bool>,
            ) -> JoinBuilder<($($S,)+ Src<J, $N>), ($($R,)+ J)> {
                let on = on($($S::handle(),)+ Src::new()).into_node();
                JoinBuilder::from_select(add_join::<J>(self.select, JoinKind::Inner, on))
            }

            /// `LEFT JOIN`s `J` as the next source; `on` may use every source.
            /// `J` decodes as `Option<J>`.
            #[allow(clippy::type_complexity, reason = "the sources and the row, by position")]
            pub fn left_join<J: Entity>(
                self,
                on: impl FnOnce($($S,)+ Src<J, $N>) -> Expr<bool>,
            ) -> JoinBuilder<($($S,)+ Src<J, $N>), ($($R,)+ Option<J>)> {
                let on = on($($S::handle(),)+ Src::new()).into_node();
                JoinBuilder::from_select(add_join::<J>(self.select, JoinKind::Left, on))
            }
        }
    };
}

join_common!(S0 R0, S1 R1);
join_common!(S0 R0, S1 R1, S2 R2);
join_common!(S0 R0, S1 R1, S2 R2, S3 R3);
join_common!(S0 R0, S1 R1, S2 R2, S3 R3, S4 R4);
join_common!(S0 R0, S1 R1, S2 R2, S3 R3, S4 R4, S5 R5);
join_common!(S0 R0, S1 R1, S2 R2, S3 R3, S4 R4, S5 R5, S6 R6);
join_common!(S0 R0, S1 R1, S2 R2, S3 R3, S4 R4, S5 R5, S6 R6, S7 R7);
join_common!(S0 R0, S1 R1, S2 R2, S3 R3, S4 R4, S5 R5, S6 R6, S7 R7, S8 R8);
join_common!(S0 R0, S1 R1, S2 R2, S3 R3, S4 R4, S5 R5, S6 R6, S7 R7, S8 R8, S9 R9);

join_next!(S0 R0, S1 R1 => 2);
join_next!(S0 R0, S1 R1, S2 R2 => 3);
join_next!(S0 R0, S1 R1, S2 R2, S3 R3 => 4);
join_next!(S0 R0, S1 R1, S2 R2, S3 R3, S4 R4 => 5);
join_next!(S0 R0, S1 R1, S2 R2, S3 R3, S4 R4, S5 R5 => 6);
join_next!(S0 R0, S1 R1, S2 R2, S3 R3, S4 R4, S5 R5, S6 R6 => 7);
join_next!(S0 R0, S1 R1, S2 R2, S3 R3, S4 R4, S5 R5, S6 R6, S7 R7 => 8);
join_next!(S0 R0, S1 R1, S2 R2, S3 R3, S4 R4, S5 R5, S6 R6, S7 R7, S8 R8 => 9);
