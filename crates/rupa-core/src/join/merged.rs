//! `#[derive(Joined)]` support: merged entities.

use std::fmt;
use std::marker::PhantomData;

use crate::At;
use crate::JoinBuilder;
use crate::Sources;
use crate::entity::FromRow;
use crate::ir::{ColumnRef, Select, SourceId, Statement};
use crate::query::{Query, RowsResult};

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

/// The row type `Self` (a tuple of [`JoinPart`](crate::join::JoinPart)s) has an `E` part at position
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

/// Never a slot kind ([`PartOf`] only yields [`Inner`] or [`Left`]). It
/// exists so that `Admits` never has a single impl for a field shape: with
/// one, the compiler would infer the slot kind from it, and a plain field of
/// an entity joined twice (`(Item, Option<Item>)`) would silently pick the
/// inner source instead of being ambiguous. Only `PartOf` decides the slot.
#[doc(hidden)]
#[derive(Debug, Clone, Copy)]
pub enum NoSlot {}
impl Admits<Plain> for NoSlot {}
impl Admits<Opt> for NoSlot {}

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
    pub(super) select: Select,
    pub(super) _p: PhantomData<fn() -> (R, V)>,
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

    /// At most one row; more is [`ResultError::TooManyRows`](crate::error::ResultError::TooManyRows).
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
