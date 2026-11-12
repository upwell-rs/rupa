//! Building joins: the join builder and source lookup.

use std::fmt;
use std::marker::PhantomData;

use crate::JoinPart;
use crate::Source;
use crate::Src;
use crate::builder::{SelectBuilder, and_filter, count_param};
use crate::column::Column;
use crate::entity::{Entity, FromRow};
use crate::expr::Expr;
use crate::ir::{
    ColumnRef, ExprNode, FromClause, Join, JoinKind, OrderBy, Select, SourceId, Statement,
};
use crate::query::{Expect, Query, RowsResult};

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
    pub(super) select: Select,
    pub(super) _p: PhantomData<fn() -> (S, R)>,
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
    pub(super) fn from_select(select: Select) -> Self {
        Self {
            select,
            _p: PhantomData,
        }
    }
}

/// Adds `J` as the next source: `kind JOIN j ON on`, and its columns to the
/// projection.
pub(super) fn add_join<J: Entity>(mut select: Select, kind: JoinKind, on: ExprNode) -> Select {
    let source =
        SourceId(u16::try_from(select.from.source_count()).expect("a source index fits in a u16"));
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
        .extend(J::COLUMNS.iter().map(|c| ColumnRef::of(source, c.name)));
    select
}

impl<E: Entity> SelectBuilder<E> {
    #[allow(
        clippy::type_complexity,
        reason = "the sources and the row, by position"
    )]
    /// `INNER JOIN`s `J` on a condition: each row pairs with every `J` row
    /// `on` accepts, and rows without a match are dropped. Rows decode as
    /// `(E, J)`.
    pub fn inner_join_on<J: Entity>(
        self,
        on: impl FnOnce((Src<E, 0>, Src<J, 1>)) -> Expr<bool>,
    ) -> JoinBuilder<(Src<E, 0>, Src<J, 1>), (E, J)> {
        let on = on(Sources::handles()).into_node();
        JoinBuilder::from_select(add_join::<J>(self.select, JoinKind::Inner, on))
    }

    #[allow(
        clippy::type_complexity,
        reason = "the sources and the row, by position"
    )]
    /// `LEFT JOIN`s `J` on a condition: like
    /// [`inner_join_on`](Self::inner_join_on), but rows without a match are
    /// kept, with `None` for `J`. Rows decode as `(E, Option<J>)`.
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
    /// [`ResultError::TooManyRows`](crate::error::ResultError::TooManyRows).
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
