//! Capability traits: what may be done with an entity's table.
//!
//! Each trait is generic over the target entity `E` and implemented by the
//! *value* that takes part in the operation:
//!
//! - `User: Insertable<User>`: the entity inserts itself (one model), and
//!   `NewUser: Insertable<User>` is a separate, typed input struct.
//! - `User: Updatable<User>` is a full update by id; `UserPatch: Updatable<User>`
//!   is a partial one.
//!
//! An entity with no capability impls has no write surface at all: the
//! builders that write require these traits.

use crate::entity::{Entity, IdValues};
use crate::value::Value;

/// Marks rows of `E` as readable by id (`get`). Implemented by `E` itself.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be fetched by id from `{E}`",
    note = "derive `Gettable` on `{E}` to allow it"
)]
pub trait Gettable<E: Entity> {}

/// A value that can be inserted as one row of `E`.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be inserted into `{E}`",
    note = "derive `Insertable` on `{E}`, or on an input struct with `#[insertable(entity = {E})]`"
)]
pub trait Insertable<E: Entity> {
    /// Column names and values to write. The same type always yields the
    /// same columns, in the same order.
    fn insert_values(&self) -> Vec<(&'static str, Value)>;
}

/// Marker for update values that do not identify a row; they apply to the
/// rows matched by an explicit filter instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoKey;

/// The id of the row an update value targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Keyed<I>(pub I);

/// A key that identifies one row: [`Keyed`], but not [`NoKey`].
#[diagnostic::on_unimplemented(
    message = "this update value does not identify a row (its key is `{Self}`)",
    note = "give the patch struct an `#[id]` field, or use `.apply(&value).filter(..)` to update the rows a filter matches"
)]
pub trait RowKey {
    fn key_values(&self) -> Vec<Value>;
}

impl<I: IdValues> RowKey for Keyed<I> {
    fn key_values(&self) -> Vec<Value> {
        self.0.id_values()
    }
}

/// A value that describes an update to rows of `E`.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be used to update `{E}`",
    note = "derive `Updatable` on `{E}`, or on a patch struct with `#[updatable(entity = {E})]`"
)]
pub trait Updatable<E: Entity> {
    /// [`Keyed<E::Id>`] when the value identifies its row (update by id),
    /// [`NoKey`] when it must be combined with a filter.
    type Key;

    fn key(&self) -> Self::Key;

    /// Columns to set. Fields a patch leaves as `None` are omitted.
    fn update_values(&self) -> Vec<(&'static str, Value)>;
}

/// A value that identifies a row of `E` to delete.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be used to delete from `{E}`",
    note = "derive `Deletable` on `{E}` to allow it"
)]
pub trait Deletable<E: Entity> {
    fn delete_key(&self) -> Vec<Value>;
}

/// Convenience for hand-written impls: the id values of an entity.
pub fn key_of<E: Entity>(entity: &E) -> Vec<Value> {
    entity.id().id_values()
}
