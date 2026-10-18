//! Joins: source handles, the join builder, and decoding joined rows.
//!
//! [`SelectBuilder::inner_join_on`](crate::builder::SelectBuilder::inner_join_on) and [`SelectBuilder::left_join_on`](crate::builder::SelectBuilder::left_join_on) turn
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

mod build;
mod decode;
mod merged;
mod relation;

use std::fmt;
use std::marker::PhantomData;

use crate::column::Column;
use crate::entity::Entity;
use crate::ir::SourceId;

pub use build::*;
pub use decode::*;
pub use merged::*;
pub use relation::*;

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
