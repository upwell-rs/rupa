//! Repositories: what `#[repository]` builds on.
//!
//! A repository handle holds one [`Connection`](crate::connection::Connection),
//! a pool, a connection or a transaction, in any wrapper, and runs every
//! query on it. A repository opens a transaction on its connection with
//! `open_tx` and joins an open one with `tx`, staying the same repository.

/// A repository trait object that can be built over a connection `C`.
///
/// `#[repository]` implements it for `dyn Trait` of every dyn-compatible
/// repository, so DI integrations can provide `Arc<dyn Trait>` over their
/// connections without naming the generated repository types.
pub trait DynRepository<C> {
    /// The repository over `connection`.
    fn over(connection: C) -> std::sync::Arc<Self>;
}
