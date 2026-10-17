//! Joins: source handles, the join builder, and decoding joined rows.
//!
//! [`SelectBuilder::inner_join_on`] and [`SelectBuilder::left_join_on`] turn
//! a select into a [`JoinBuilder`]. Its closures (`on`, `filter`, `order_by`)
//! receive a tuple of [`Src`] handles, one per row source in join order, and
//! columns are taken through them, so every column names its source:
//!
//! ```ignore
//! let rows: Vec<(User, Option<Order>)> = select::<User>()
//!     .left_join_on::<Order>(|(u, o)| o.col(col!(Order::user_id)).eq(u.col(col!(User::id))))
//!     .filter(|(u, _)| u.col(col!(User::active)).eq(true))
//!     .all()
//!     .run(&mut db)?;
//! ```
//!
//! `on!(Order::user_id == User::id)` writes the same closure: it finds each
//! entity's source by type ([`Has`]), which works whenever the entity is a
//! source exactly once.
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
    BinOp, ColumnRef, ExprNode, FromClause, Join, JoinKind, OrderBy, Select, SourceId, Statement,
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

/// The source handles of a join, as a tuple in join order. Join closures
/// receive it: `|(user, order)| ..`.
pub trait Sources: Copy {
    /// The handles.
    fn handles() -> Self;
}

macro_rules! sources {
    ($($S:ident),+) => {
        impl<$($S: Source),+> Sources for ($($S,)+) {
            fn handles() -> Self {
                ($($S::handle(),)+)
            }
        }
    };
}

sources!(S0, S1);
sources!(S0, S1, S2);
sources!(S0, S1, S2, S3);
sources!(S0, S1, S2, S3, S4);
sources!(S0, S1, S2, S3, S4, S5);
sources!(S0, S1, S2, S3, S4, S5, S6);
sources!(S0, S1, S2, S3, S4, S5, S6, S7);
sources!(S0, S1, S2, S3, S4, S5, S6, S7, S8);
sources!(S0, S1, S2, S3, S4, S5, S6, S7, S8, S9);

/// Position marker for [`Has`].
#[derive(Debug, Clone, Copy)]
pub struct At<const P: u16>;

/// Sources `Self` include an `E` source, at position `I` (an [`At`] marker the
/// compiler infers). If `E` is a source twice, as in a self-join, inference
/// is ambiguous; use the source handles instead.
#[diagnostic::on_unimplemented(
    message = "`{E}` is not a source of this join",
    note = "only entities joined so far (or the one `select` started from) can be used here"
)]
pub trait Has<E, I> {
    /// The id of the `E` source.
    const ID: SourceId;
}

macro_rules! has {
    ($( ($($before:ident),*) $p:literal ($($after:ident),*) );* $(;)?) => {$(
        impl<E, $($before,)* $($after,)*> Has<E, At<$p>> for ($($before,)* Src<E, $p>, $($after,)*) {
            const ID: SourceId = SourceId($p);
        }
    )*};
}

has! {
    () 0 (S1);
    (S0) 1 ();
    () 0 (S1, S2);
    (S0) 1 (S2);
    (S0, S1) 2 ();
    () 0 (S1, S2, S3);
    (S0) 1 (S2, S3);
    (S0, S1) 2 (S3);
    (S0, S1, S2) 3 ();
    () 0 (S1, S2, S3, S4);
    (S0) 1 (S2, S3, S4);
    (S0, S1) 2 (S3, S4);
    (S0, S1, S2) 3 (S4);
    (S0, S1, S2, S3) 4 ();
    () 0 (S1, S2, S3, S4, S5);
    (S0) 1 (S2, S3, S4, S5);
    (S0, S1) 2 (S3, S4, S5);
    (S0, S1, S2) 3 (S4, S5);
    (S0, S1, S2, S3) 4 (S5);
    (S0, S1, S2, S3, S4) 5 ();
    () 0 (S1, S2, S3, S4, S5, S6);
    (S0) 1 (S2, S3, S4, S5, S6);
    (S0, S1) 2 (S3, S4, S5, S6);
    (S0, S1, S2) 3 (S4, S5, S6);
    (S0, S1, S2, S3) 4 (S5, S6);
    (S0, S1, S2, S3, S4) 5 (S6);
    (S0, S1, S2, S3, S4, S5) 6 ();
    () 0 (S1, S2, S3, S4, S5, S6, S7);
    (S0) 1 (S2, S3, S4, S5, S6, S7);
    (S0, S1) 2 (S3, S4, S5, S6, S7);
    (S0, S1, S2) 3 (S4, S5, S6, S7);
    (S0, S1, S2, S3) 4 (S5, S6, S7);
    (S0, S1, S2, S3, S4) 5 (S6, S7);
    (S0, S1, S2, S3, S4, S5) 6 (S7);
    (S0, S1, S2, S3, S4, S5, S6) 7 ();
    () 0 (S1, S2, S3, S4, S5, S6, S7, S8);
    (S0) 1 (S2, S3, S4, S5, S6, S7, S8);
    (S0, S1) 2 (S3, S4, S5, S6, S7, S8);
    (S0, S1, S2) 3 (S4, S5, S6, S7, S8);
    (S0, S1, S2, S3) 4 (S5, S6, S7, S8);
    (S0, S1, S2, S3, S4) 5 (S6, S7, S8);
    (S0, S1, S2, S3, S4, S5) 6 (S7, S8);
    (S0, S1, S2, S3, S4, S5, S6) 7 (S8);
    (S0, S1, S2, S3, S4, S5, S6, S7) 8 ();
    () 0 (S1, S2, S3, S4, S5, S6, S7, S8, S9);
    (S0) 1 (S2, S3, S4, S5, S6, S7, S8, S9);
    (S0, S1) 2 (S3, S4, S5, S6, S7, S8, S9);
    (S0, S1, S2) 3 (S4, S5, S6, S7, S8, S9);
    (S0, S1, S2, S3) 4 (S5, S6, S7, S8, S9);
    (S0, S1, S2, S3, S4) 5 (S6, S7, S8, S9);
    (S0, S1, S2, S3, S4, S5) 6 (S7, S8, S9);
    (S0, S1, S2, S3, S4, S5, S6) 7 (S8, S9);
    (S0, S1, S2, S3, S4, S5, S6, S7) 8 (S9);
    (S0, S1, S2, S3, S4, S5, S6, S7, S8) 9 ();
}

/// `column`, of the one `E` source among `sources`. This is how `on!` finds a
/// field's source; a join with two `E` sources must use the handles instead.
pub fn source_col<S, E, I, T, K>(_sources: &S, column: Column<E, T, K>) -> Column<E, T, K>
where
    S: Has<E, I>,
{
    column.of_source(S::ID)
}

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
    /// `INNER JOIN`s `J` on a condition: each row pairs with every `J` row
    /// `on` accepts, and rows without a match are dropped. Rows decode as
    /// `(E, J)`.
    #[allow(
        clippy::type_complexity,
        reason = "the sources and the row, by position"
    )]
    pub fn inner_join_on<J: Entity>(
        self,
        on: impl FnOnce((Src<E, 0>, Src<J, 1>)) -> Expr<bool>,
    ) -> JoinBuilder<(Src<E, 0>, Src<J, 1>), (E, J)> {
        let on = on(Sources::handles()).into_node();
        JoinBuilder::from_select(add_join::<J>(self.select, JoinKind::Inner, on))
    }

    /// `LEFT JOIN`s `J` on a condition: like
    /// [`inner_join_on`](Self::inner_join_on), but rows without a match are
    /// kept, with `None` for `J`. Rows decode as `(E, Option<J>)`.
    #[allow(
        clippy::type_complexity,
        reason = "the sources and the row, by position"
    )]
    pub fn left_join_on<J: Entity>(
        self,
        on: impl FnOnce((Src<E, 0>, Src<J, 1>)) -> Expr<bool>,
    ) -> JoinBuilder<(Src<E, 0>, Src<J, 1>), (E, Option<J>)> {
        let on = on(Sources::handles()).into_node();
        JoinBuilder::from_select(add_join::<J>(self.select, JoinKind::Left, on))
    }
}

impl<S: Sources, R: FromRow> JoinBuilder<S, R> {
    /// Adds a condition over the sources; repeated calls are ANDed.
    pub fn filter(mut self, cond: impl FnOnce(S) -> Expr<bool>) -> Self {
        and_filter(&mut self.select.filter, cond(S::handles()).into_node());
        self
    }

    /// Adds a sort key over the sources; repeated calls sort by each in turn.
    pub fn order_by(mut self, order: impl FnOnce(S) -> OrderBy) -> Self {
        self.select.order_by.push(order(S::handles()));
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
    pub fn build<Res: RowsResult<Item = R>>(self) -> Query<Res> {
        Query::new(Statement::Select(self.select), Res::EXPECT)
    }

    /// Every matching row.
    pub fn all(self) -> Query<Vec<R>> {
        self.build()
    }

    /// At most one row; more is
    /// [`ResultError::TooManyRows`].
    pub fn optional(self) -> Query<Option<R>> {
        self.build()
    }

    /// Exactly one row.
    pub fn one(self) -> Query<R> {
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

macro_rules! join_next {
    ($($S:ident $R:ident),+ => $N:literal) => {
        impl<$($S: Source,)+ $($R: JoinPart,)+> JoinBuilder<($($S,)+), ($($R,)+)> {
            /// `INNER JOIN`s `J` as the next source; `on` may use every source.
            #[allow(clippy::type_complexity, reason = "the sources and the row, by position")]
            pub fn inner_join_on<J: Entity>(
                self,
                on: impl FnOnce(($($S,)+ Src<J, $N>)) -> Expr<bool>,
            ) -> JoinBuilder<($($S,)+ Src<J, $N>), ($($R,)+ J)> {
                let on = on(Sources::handles()).into_node();
                JoinBuilder::from_select(add_join::<J>(self.select, JoinKind::Inner, on))
            }

            /// `LEFT JOIN`s `J` as the next source; `on` may use every source.
            /// `J` decodes as `Option<J>`.
            #[allow(clippy::type_complexity, reason = "the sources and the row, by position")]
            pub fn left_join_on<J: Entity>(
                self,
                on: impl FnOnce(($($S,)+ Src<J, $N>)) -> Expr<bool>,
            ) -> JoinBuilder<($($S,)+ Src<J, $N>), ($($R,)+ Option<J>)> {
                let on = on(Sources::handles()).into_node();
                JoinBuilder::from_select(add_join::<J>(self.select, JoinKind::Left, on))
            }
        }
    };
}

join_next!(S0 R0, S1 R1 => 2);
join_next!(S0 R0, S1 R1, S2 R2 => 3);
join_next!(S0 R0, S1 R1, S2 R2, S3 R3 => 4);
join_next!(S0 R0, S1 R1, S2 R2, S3 R3, S4 R4 => 5);
join_next!(S0 R0, S1 R1, S2 R2, S3 R3, S4 R4, S5 R5 => 6);
join_next!(S0 R0, S1 R1, S2 R2, S3 R3, S4 R4, S5 R5, S6 R6 => 7);
join_next!(S0 R0, S1 R1, S2 R2, S3 R3, S4 R4, S5 R5, S6 R6, S7 R7 => 8);
join_next!(S0 R0, S1 R1, S2 R2, S3 R3, S4 R4, S5 R5, S6 R6, S7 R7, S8 R8 => 9);

// ---------------------------------------------------------------------------
// Relations
// ---------------------------------------------------------------------------

/// Direction of a [`JoinTo`] impl: the joined entity holds the reference
/// (`Order` joined onto a `User` source through `Order.user_id`).
#[derive(Debug, Clone, Copy)]
pub struct Fwd;

/// Direction of a [`JoinTo`] impl: the joined entity is referenced (`User`
/// joined onto an `Order` source through `Order.user_id`).
#[derive(Debug, Clone, Copy)]
pub struct Rev;

/// `Self` can be joined onto an `E` source through declared relation `R`
/// (a marker type `#[column(references = ..)]` generates), in direction `D`
/// ([`Fwd`] or [`Rev`]). Generated by `#[derive(Entity)]`; not written by hand.
#[diagnostic::on_unimplemented(
    message = "`{Self}` has no declared relation to `{E}`",
    label = "no `#[column(references = ..)]` between them",
    note = "declare one with `#[column(references = ..)]`, or give the condition: `inner_join_on` / `left_join_on`"
)]
pub trait JoinTo<E, D, R> {
    /// The `ON` condition, with `Self` as source `new` and `E` as `earlier`.
    fn on(new: SourceId, earlier: SourceId) -> ExprNode;
}

/// `fk = target.id`: the `ON` condition of a declared relation. `fk` is the
/// referencing column; `target` the referenced entity's source.
#[doc(hidden)]
pub fn relation_on(fk: ColumnRef, target: SourceId, target_ids: &[&'static str]) -> ExprNode {
    let [id] = target_ids else {
        panic!("a relation must reference an entity with a single id column")
    };
    ExprNode::Binary(
        BinOp::Eq,
        Box::new(ExprNode::Column(fk)),
        Box::new(ExprNode::Column(ColumnRef::of(target, id))),
    )
}

/// A declared relation, as its generated accessor returns it
/// (`Order::user()`); picks the relation with [`PendingJoin::inner_via`] and
/// [`PendingJoin::left_via`] when an entity has several.
pub struct Rel<R>(PhantomData<fn() -> R>);

impl<R> Rel<R> {
    /// The relation (a zero-sized value).
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}

impl<R> Default for Rel<R> {
    fn default() -> Self {
        Self::new()
    }
}

impl<R> Clone for Rel<R> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<R> Copy for Rel<R> {}

impl<R> fmt::Debug for Rel<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Rel<{}>", std::any::type_name::<R>())
    }
}

/// The value types a `#[column(references = E)]` field may have: `E`'s id
/// type, or an `Option` of it.
#[diagnostic::on_unimplemented(
    message = "a `references` field must hold the referenced entity's id (`{Id}`), or an `Option` of it",
    label = "this field's type is `{Self}`"
)]
pub trait RefersTo<Id> {}
impl<T> RefersTo<T> for T {}
impl<T> RefersTo<T> for Option<T> {}

/// `J` can join the sources `Self` through one declared relation: `R`, in
/// direction `D`, onto the source at `P` (an [`At`] marker). All three are
/// inferred; it's ambiguous when several relations fit.
#[diagnostic::on_unimplemented(
    message = "`{J}` has no declared relation to the sources of this join",
    note = "declare one with `#[column(references = ..)]`, or give the condition: `inner_join_on` / `left_join_on`"
)]
pub trait JoinOnto<J, P, D, R> {
    /// The `ON` condition, with `J` as source `new`.
    fn on(new: SourceId) -> ExprNode;
}

macro_rules! join_onto {
    ($( ($($before:ident),*) $p:literal ($($after:ident),*) );* $(;)?) => {$(
        impl<J, E, D, R, $($before,)* $($after,)*> JoinOnto<J, At<$p>, D, R>
            for ($($before,)* Src<E, $p>, $($after,)*)
        where
            J: JoinTo<E, D, R>,
        {
            fn on(new: SourceId) -> ExprNode {
                J::on(new, SourceId($p))
            }
        }
    )*};
}

join_onto! {
    () 0 ();
    () 0 (S1);
    (S0) 1 ();
    () 0 (S1, S2);
    (S0) 1 (S2);
    (S0, S1) 2 ();
    () 0 (S1, S2, S3);
    (S0) 1 (S2, S3);
    (S0, S1) 2 (S3);
    (S0, S1, S2) 3 ();
    () 0 (S1, S2, S3, S4);
    (S0) 1 (S2, S3, S4);
    (S0, S1) 2 (S3, S4);
    (S0, S1, S2) 3 (S4);
    (S0, S1, S2, S3) 4 ();
    () 0 (S1, S2, S3, S4, S5);
    (S0) 1 (S2, S3, S4, S5);
    (S0, S1) 2 (S3, S4, S5);
    (S0, S1, S2) 3 (S4, S5);
    (S0, S1, S2, S3) 4 (S5);
    (S0, S1, S2, S3, S4) 5 ();
    () 0 (S1, S2, S3, S4, S5, S6);
    (S0) 1 (S2, S3, S4, S5, S6);
    (S0, S1) 2 (S3, S4, S5, S6);
    (S0, S1, S2) 3 (S4, S5, S6);
    (S0, S1, S2, S3) 4 (S5, S6);
    (S0, S1, S2, S3, S4) 5 (S6);
    (S0, S1, S2, S3, S4, S5) 6 ();
    () 0 (S1, S2, S3, S4, S5, S6, S7);
    (S0) 1 (S2, S3, S4, S5, S6, S7);
    (S0, S1) 2 (S3, S4, S5, S6, S7);
    (S0, S1, S2) 3 (S4, S5, S6, S7);
    (S0, S1, S2, S3) 4 (S5, S6, S7);
    (S0, S1, S2, S3, S4) 5 (S6, S7);
    (S0, S1, S2, S3, S4, S5) 6 (S7);
    (S0, S1, S2, S3, S4, S5, S6) 7 ();
    () 0 (S1, S2, S3, S4, S5, S6, S7, S8);
    (S0) 1 (S2, S3, S4, S5, S6, S7, S8);
    (S0, S1) 2 (S3, S4, S5, S6, S7, S8);
    (S0, S1, S2) 3 (S4, S5, S6, S7, S8);
    (S0, S1, S2, S3) 4 (S5, S6, S7, S8);
    (S0, S1, S2, S3, S4) 5 (S6, S7, S8);
    (S0, S1, S2, S3, S4, S5) 6 (S7, S8);
    (S0, S1, S2, S3, S4, S5, S6) 7 (S8);
    (S0, S1, S2, S3, S4, S5, S6, S7) 8 ();
}

/// A join of `J` whose condition comes from a declared relation; finish it
/// with [`inner`](Self::inner) / [`left`](Self::left), or pick a relation
/// with [`inner_via`](Self::inner_via) / [`left_via`](Self::left_via).
pub struct PendingJoin<S, R, J> {
    select: Select,
    _p: Phantom<S, R, J>,
}

type Phantom<S, R, J> = PhantomData<fn() -> (S, R, J)>;

impl<S, R, J> fmt::Debug for PendingJoin<S, R, J> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PendingJoin")
            .field("joining", &std::any::type_name::<J>())
            .field("select", &self.select)
            .finish()
    }
}

impl<E: Entity> SelectBuilder<E> {
    /// Joins `J` through a declared relation (`#[column(references = ..)]`)
    /// between `J` and `E`; finish with `.inner()` or `.left()`.
    pub fn join<J: Entity>(self) -> PendingJoin<(Src<E, 0>,), (E,), J> {
        PendingJoin {
            select: self.select,
            _p: PhantomData,
        }
    }
}

macro_rules! join_method {
    ($($S:ident $R:ident),+) => {
        impl<$($S,)+ $($R,)+> JoinBuilder<($($S,)+), ($($R,)+)> {
            /// Joins `J` through a declared relation (`#[column(references = ..)]`)
            /// between `J` and one of the sources; finish with `.inner()` or
            /// `.left()`.
            pub fn join<J: Entity>(self) -> PendingJoin<($($S,)+), ($($R,)+), J> {
                PendingJoin {
                    select: self.select,
                    _p: PhantomData,
                }
            }
        }
    };
}

join_method!(S0 R0, S1 R1);
join_method!(S0 R0, S1 R1, S2 R2);
join_method!(S0 R0, S1 R1, S2 R2, S3 R3);
join_method!(S0 R0, S1 R1, S2 R2, S3 R3, S4 R4);
join_method!(S0 R0, S1 R1, S2 R2, S3 R3, S4 R4, S5 R5);
join_method!(S0 R0, S1 R1, S2 R2, S3 R3, S4 R4, S5 R5, S6 R6);
join_method!(S0 R0, S1 R1, S2 R2, S3 R3, S4 R4, S5 R5, S6 R6, S7 R7);
join_method!(S0 R0, S1 R1, S2 R2, S3 R3, S4 R4, S5 R5, S6 R6, S7 R7, S8 R8);

macro_rules! pending {
    ($($S:ident $R:ident),+ => $N:literal) => {
        impl<$($S,)+ $($R,)+ J: Entity> PendingJoin<($($S,)+), ($($R,)+), J> {
            /// `INNER JOIN`, on the one declared relation between `J` and the
            /// sources. Rows without a match are dropped.
            #[allow(clippy::type_complexity, reason = "the sources and the row, by position")]
            pub fn inner<P, D, Rl>(self) -> JoinBuilder<($($S,)+ Src<J, $N>), ($($R,)+ J)>
            where
                ($($S,)+): JoinOnto<J, P, D, Rl>,
            {
                let on = <($($S,)+) as JoinOnto<J, P, D, Rl>>::on(SourceId($N));
                JoinBuilder::from_select(add_join::<J>(self.select, JoinKind::Inner, on))
            }

            /// `LEFT JOIN`, on the one declared relation between `J` and the
            /// sources. `J` decodes as `Option<J>`.
            #[allow(clippy::type_complexity, reason = "the sources and the row, by position")]
            pub fn left<P, D, Rl>(self) -> JoinBuilder<($($S,)+ Src<J, $N>), ($($R,)+ Option<J>)>
            where
                ($($S,)+): JoinOnto<J, P, D, Rl>,
            {
                let on = <($($S,)+) as JoinOnto<J, P, D, Rl>>::on(SourceId($N));
                JoinBuilder::from_select(add_join::<J>(self.select, JoinKind::Left, on))
            }

            /// [`inner`](Self::inner) through relation `rel`, for when `J`
            /// has several relations to the sources.
            #[allow(clippy::type_complexity, reason = "the sources and the row, by position")]
            pub fn inner_via<P, D, Rl>(
                self,
                rel: Rel<Rl>,
            ) -> JoinBuilder<($($S,)+ Src<J, $N>), ($($R,)+ J)>
            where
                ($($S,)+): JoinOnto<J, P, D, Rl>,
            {
                let _ = rel;
                self.inner::<P, D, Rl>()
            }

            /// [`left`](Self::left) through relation `rel`, for when `J`
            /// has several relations to the sources.
            #[allow(clippy::type_complexity, reason = "the sources and the row, by position")]
            pub fn left_via<P, D, Rl>(
                self,
                rel: Rel<Rl>,
            ) -> JoinBuilder<($($S,)+ Src<J, $N>), ($($R,)+ Option<J>)>
            where
                ($($S,)+): JoinOnto<J, P, D, Rl>,
            {
                let _ = rel;
                self.left::<P, D, Rl>()
            }
        }
    };
}

pending!(S0 R0 => 1);
pending!(S0 R0, S1 R1 => 2);
pending!(S0 R0, S1 R1, S2 R2 => 3);
pending!(S0 R0, S1 R1, S2 R2, S3 R3 => 4);
pending!(S0 R0, S1 R1, S2 R2, S3 R3, S4 R4 => 5);
pending!(S0 R0, S1 R1, S2 R2, S3 R3, S4 R4, S5 R5 => 6);
pending!(S0 R0, S1 R1, S2 R2, S3 R3, S4 R4, S5 R5, S6 R6 => 7);
pending!(S0 R0, S1 R1, S2 R2, S3 R3, S4 R4, S5 R5, S6 R6, S7 R7 => 8);
pending!(S0 R0, S1 R1, S2 R2, S3 R3, S4 R4, S5 R5, S6 R6, S7 R7, S8 R8 => 9);

// ---------------------------------------------------------------------------
// Merged entities
// ---------------------------------------------------------------------------

/// Slot kind for [`PartOf`]: the entity is inner-joined (or the first source).
#[derive(Debug, Clone, Copy)]
pub struct Inner;

/// Slot kind for [`PartOf`]: the entity is left-joined, so its columns may be
/// `NULL`.
#[derive(Debug, Clone, Copy)]
pub struct Left;

/// Field shape for [`Admits`]: a field type that isn't an `Option`.
#[derive(Debug, Clone, Copy)]
pub struct Plain;

/// Field shape for [`Admits`]: an `Option` field.
#[derive(Debug, Clone, Copy)]
pub struct Opt;

/// The row type `Self` (a tuple of [`JoinPart`]s) has an `E` part at position
/// `I`, of kind `N`: [`Inner`] for `E`, [`Left`] for `Option<E>`. Inferred.
#[diagnostic::on_unimplemented(
    message = "`{E}` is not a source of this join",
    note = "a `#[derive(Joined)]` struct can only take fields from the join's sources"
)]
pub trait PartOf<E, I, N> {
    /// The source of the `E` part.
    const ID: SourceId;
}

macro_rules! part_of {
    ($( ($($before:ident),*) $p:literal ($($after:ident),*) );* $(;)?) => {$(
        impl<E, $($before,)* $($after,)*> PartOf<E, At<$p>, Inner> for ($($before,)* E, $($after,)*) {
            const ID: SourceId = SourceId($p);
        }
        impl<E, $($before,)* $($after,)*> PartOf<E, At<$p>, Left>
            for ($($before,)* Option<E>, $($after,)*)
        {
            const ID: SourceId = SourceId($p);
        }
    )*};
}

part_of! {
    () 0 (R1);
    (R0) 1 ();
    () 0 (R1, R2);
    (R0) 1 (R2);
    (R0, R1) 2 ();
    () 0 (R1, R2, R3);
    (R0) 1 (R2, R3);
    (R0, R1) 2 (R3);
    (R0, R1, R2) 3 ();
    () 0 (R1, R2, R3, R4);
    (R0) 1 (R2, R3, R4);
    (R0, R1) 2 (R3, R4);
    (R0, R1, R2) 3 (R4);
    (R0, R1, R2, R3) 4 ();
    () 0 (R1, R2, R3, R4, R5);
    (R0) 1 (R2, R3, R4, R5);
    (R0, R1) 2 (R3, R4, R5);
    (R0, R1, R2) 3 (R4, R5);
    (R0, R1, R2, R3) 4 (R5);
    (R0, R1, R2, R3, R4) 5 ();
    () 0 (R1, R2, R3, R4, R5, R6);
    (R0) 1 (R2, R3, R4, R5, R6);
    (R0, R1) 2 (R3, R4, R5, R6);
    (R0, R1, R2) 3 (R4, R5, R6);
    (R0, R1, R2, R3) 4 (R5, R6);
    (R0, R1, R2, R3, R4) 5 (R6);
    (R0, R1, R2, R3, R4, R5) 6 ();
    () 0 (R1, R2, R3, R4, R5, R6, R7);
    (R0) 1 (R2, R3, R4, R5, R6, R7);
    (R0, R1) 2 (R3, R4, R5, R6, R7);
    (R0, R1, R2) 3 (R4, R5, R6, R7);
    (R0, R1, R2, R3) 4 (R5, R6, R7);
    (R0, R1, R2, R3, R4) 5 (R6, R7);
    (R0, R1, R2, R3, R4, R5) 6 (R7);
    (R0, R1, R2, R3, R4, R5, R6) 7 ();
    () 0 (R1, R2, R3, R4, R5, R6, R7, R8);
    (R0) 1 (R2, R3, R4, R5, R6, R7, R8);
    (R0, R1) 2 (R3, R4, R5, R6, R7, R8);
    (R0, R1, R2) 3 (R4, R5, R6, R7, R8);
    (R0, R1, R2, R3) 4 (R5, R6, R7, R8);
    (R0, R1, R2, R3, R4) 5 (R6, R7, R8);
    (R0, R1, R2, R3, R4, R5) 6 (R7, R8);
    (R0, R1, R2, R3, R4, R5, R6) 7 (R8);
    (R0, R1, R2, R3, R4, R5, R6, R7) 8 ();
    () 0 (R1, R2, R3, R4, R5, R6, R7, R8, R9);
    (R0) 1 (R2, R3, R4, R5, R6, R7, R8, R9);
    (R0, R1) 2 (R3, R4, R5, R6, R7, R8, R9);
    (R0, R1, R2) 3 (R4, R5, R6, R7, R8, R9);
    (R0, R1, R2, R3) 4 (R5, R6, R7, R8, R9);
    (R0, R1, R2, R3, R4) 5 (R6, R7, R8, R9);
    (R0, R1, R2, R3, R4, R5) 6 (R7, R8, R9);
    (R0, R1, R2, R3, R4, R5, R6) 7 (R8, R9);
    (R0, R1, R2, R3, R4, R5, R6, R7) 8 (R9);
    (R0, R1, R2, R3, R4, R5, R6, R7, R8) 9 ();
}

/// A slot kind `Self` that can fill a field of shape `F`: anything fits an
/// `Option` field, but only an inner-joined entity fits a plain one.
#[diagnostic::on_unimplemented(
    message = "a field from a left-joined entity must be an `Option`",
    note = "the entity's columns are `NULL` when the left join finds no match"
)]
pub trait Admits<F> {}
impl Admits<Plain> for Inner {}
impl Admits<Opt> for Inner {}
impl Admits<Opt> for Left {}

/// A struct that joined rows decode into, with fields taken from several
/// entities (`#[derive(Joined)]`). `R` is the join's row type; `M` the
/// inferred positions of the struct's fields. Rows decode through `FromRow`,
/// in [`projection`](Self::projection) order.
pub trait Joined<R, M>: FromRow {
    /// The columns to select, each from its entity's source.
    fn projection() -> Vec<ColumnRef>;
}

/// A join whose rows decode into the `Joined` struct `V`; finish with
/// [`all`](Self::all), [`optional`](Self::optional) or [`one`](Self::one).
pub struct IntoJoined<R, V> {
    select: Select,
    _p: PhantomData<fn() -> (R, V)>,
}

impl<R, V> fmt::Debug for IntoJoined<R, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IntoJoined")
            .field("into", &std::any::type_name::<V>())
            .field("select", &self.select)
            .finish()
    }
}

impl<S: Sources, R: FromRow> JoinBuilder<S, R> {
    /// Decodes rows into `V`, a `#[derive(Joined)]` struct, instead of a
    /// tuple. Only `V`'s columns are selected.
    #[allow(
        clippy::should_implement_trait,
        reason = "a builder step, not a conversion"
    )]
    pub fn into<V: FromRow>(self) -> IntoJoined<R, V> {
        IntoJoined {
            select: self.select,
            _p: PhantomData,
        }
    }
}

impl<R, V: FromRow> IntoJoined<R, V> {
    /// Builds with the result shape taken from `Res`.
    pub fn build<Res: RowsResult<Item = V>, M>(mut self) -> Query<Res>
    where
        V: Joined<R, M>,
    {
        self.select.projection = V::projection();
        Query::new(Statement::Select(self.select), Res::EXPECT)
    }

    /// Every matching row.
    pub fn all<M>(self) -> Query<Vec<V>>
    where
        V: Joined<R, M>,
    {
        self.build::<_, M>()
    }

    /// At most one row; more is [`ResultError::TooManyRows`].
    pub fn optional<M>(self) -> Query<Option<V>>
    where
        V: Joined<R, M>,
    {
        self.build::<_, M>()
    }

    /// Exactly one row.
    pub fn one<M>(self) -> Query<V>
    where
        V: Joined<R, M>,
    {
        self.build::<_, M>()
    }
}
