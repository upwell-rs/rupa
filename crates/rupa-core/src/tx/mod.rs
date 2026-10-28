//! Transactions.
//!
//! Two APIs over the same machinery:
//!
//! - **Guard:** `begin()` returns a transaction that is itself an executor,
//!   so code generic over [`Executor`] / [`AsyncExecutor`](crate::exec::AsyncExecutor) runs unchanged
//!   inside or outside a transaction. Calling `begin()` on a transaction opens
//!   a nested one (a savepoint).
//! - **Closure:** `transaction(|tx| ..)` commits on `Ok` and rolls back on `Err`.
//!
//! Every method takes `&self`: two repositories share one transaction by
//! borrowing it, and `commit(self)` cannot run while either still does.
//! Statements sent to a transaction while one of its savepoints is open run
//! inside that savepoint.
//!
//! # Who implements what
//!
//! Drivers implement [`TxConnection`] (sync) or [`AsyncTxConnection`]: "run
//! this transaction-control statement" plus the dirty-connection bookkeeping
//! below. Everything else lives here, once:
//!
//! - [`Tx`] / [`AsyncTx`], the transaction guards, generic over how they hold
//!   the connection: borrowed (`Tx<&Conn>`, what `begin()` returns) or
//!   owned (`AsyncTx<Box<Conn>>`, what a pool's `begin()` returns, so a
//!   transaction can outlive the borrow of its pool, e.g. in a request scope).
//! - [`Transactional`] / [`AsyncTransactional`] for the guards themselves;
//!   a driver implements them for its connection in four lines, returning
//!   `Tx::begin(self, options)`.
//!
//! # Dropping an open transaction
//!
//! A transaction dropped without `commit` or `rollback` is rolled back:
//!
//! - **Sync:** immediately, in `Drop`. If that fails (the connection broke),
//!   the rollback is deferred like an async one.
//! - **Async:** `Drop` cannot await. The connection is marked dirty instead
//!   ([`AsyncTxConnection::defer_rollback`]), and the driver sends the rollback
//!   before the connection's next statement. Pools check
//!   [`is_dirty`](AsyncTxConnection::is_dirty) and [`clean`](AsyncTxConnection::clean)
//!   connections when they are returned.

#[cfg(feature = "async")]
mod asynchronous;
mod state;
#[cfg(test)]
mod tests;

use std::ops::Deref;

use crate::error::TxError;
use crate::exec::{Engine, Executor, Immediate, Outcome};
pub use crate::ir::{IsolationLevel, TxOptions};
use crate::ir::{Statement, TxStatement};
use crate::query::Expect;

#[cfg(feature = "async")]
pub use asynchronous::{AsyncTransaction, AsyncTransactional, AsyncTx, AsyncTxConnection};
pub use state::{PendingRollback, TxSlot};

/// A transaction that is still open.
pub trait Transaction: Executor + Sized {
    /// Commits the transaction (or releases the savepoint).
    fn commit(self) -> Result<(), Self::Error>;
    /// Rolls the transaction (or savepoint) back.
    fn rollback(self) -> Result<(), Self::Error>;
}

/// An executor that can open transactions: a connection, a pool (whose
/// transactions own a pooled connection) or a transaction (whose
/// transactions are savepoints).
pub trait Transactional: Executor {
    /// The transaction type. `Send + Sync` so a transaction can be shared
    /// by reference, driven from async code and erased for DI like any
    /// connection; every supported sync driver's connection is.
    type Tx<'t>: Transaction
        + Transactional
        + Send
        + Sync
        + Executor<Dialect = Self::Dialect, Error = Self::Error>
        + 't
    where
        Self: 't;

    /// Starts a transaction with `options`. On a transaction, options must be
    /// the default (savepoints have none); anything else is an error.
    fn begin_with(&self, options: TxOptions) -> Result<Self::Tx<'_>, Self::Error>;

    /// Starts a transaction with default options.
    fn begin(&self) -> Result<Self::Tx<'_>, Self::Error> {
        self.begin_with(TxOptions::default())
    }

    /// Runs `f` in a transaction: commits if it returns `Ok`, rolls back if it
    /// returns `Err` or panics.
    ///
    /// The closure's transaction borrows from this call (`Tx<'a>`), rather than
    /// being higher-ranked: with a GAT `where Self: 't`, a higher-ranked
    /// lifetime would force `Self: 'static` and rule out nested calls on a `Tx`.
    fn transaction<'a, T, E, F>(&'a self, f: F) -> Result<T, E>
    where
        F: FnOnce(&Self::Tx<'a>) -> Result<T, E>,
        E: From<Self::Error>,
    {
        let tx = self.begin()?;
        match f(&tx) {
            Ok(value) => {
                tx.commit()?;
                Ok(value)
            }
            Err(e) => {
                // The original error wins; a failed rollback leaves the
                // connection marked for cleanup by the driver.
                let _ = tx.rollback();
                Err(e)
            }
        }
    }
}

/// A sync connection that can run transaction-control statements. Implementing
/// it makes the connection [`Transactional`], with [`Tx`] as its transaction.
///
/// # Contract
///
/// - [`control`](Self::control) runs `BEGIN`, `COMMIT`, `SAVEPOINT` and so
///   on (render them with `rupa_sql::render_tx`).
/// - A rollback that could not be sent is recorded by
///   [`defer_rollback`](Self::defer_rollback). The driver must send it
///   (see [`PendingRollback`]) before *any* later statement, including
///   control statements, so that a broken transaction never swallows the
///   next unit of work.
/// - Every method takes `&self`, like [`Executor::execute`]; a driver whose
///   native connection needs exclusive access serializes inside.
/// - [`slot`](Self::slot) returns a [`TxSlot`] the connection owns, so it
///   runs one top-level transaction at a time.
pub trait TxConnection: Executor<Error: From<TxError>> + Send {
    /// Runs a transaction-control statement now (after any deferred rollback).
    fn control(&self, statement: TxStatement) -> Result<(), Self::Error>;

    /// Records a rollback to `depth` (`0` = the whole transaction) that must
    /// be sent before the next statement.
    fn defer_rollback(&self, depth: u32);

    /// Whether a deferred rollback has not been sent yet.
    fn is_dirty(&self) -> bool;

    /// Sends the deferred rollback, if any. Pools call this before reusing a
    /// connection.
    fn clean(&self) -> Result<(), Self::Error>;

    /// The connection's top-level transaction slot.
    fn slot(&self) -> &TxSlot;
}

/// The statement that ends a transaction (`depth == 0`) or savepoint.
pub fn rollback_to(depth: u32) -> TxStatement {
    match depth {
        0 => TxStatement::Rollback,
        d => TxStatement::RollbackToSavepoint(d),
    }
}

pub(crate) fn finish_statement(commit: bool, depth: u32) -> TxStatement {
    match (commit, depth) {
        (true, 0) => TxStatement::Commit,
        (true, d) => TxStatement::ReleaseSavepoint(d),
        (false, d) => rollback_to(d),
    }
}

/// The statement that opens a transaction (`depth == 0`) or savepoint.
pub(crate) fn open_statement<E: From<TxError>>(
    depth: u32,
    options: TxOptions,
) -> Result<TxStatement, E> {
    match depth {
        0 => Ok(TxStatement::Begin(options)),
        _ if !options.is_default() => Err(TxError::OptionsOnNested.into()),
        d => Ok(TxStatement::Savepoint(d)),
    }
}

/// An open sync transaction (`depth == 0`) or savepoint (`depth > 0`) on the
/// connection `C` holds: `&Conn` when borrowed, or an owning handle (a
/// `Box`, a pooled connection) when the transaction must be `'static`.
///
/// Rolled back when dropped without `commit` or `rollback`. Its statements
/// run through `&self`, so repositories share it by reference.
pub struct Tx<C>
where
    C: Deref<Target: TxConnection>,
{
    conn: C,
    depth: u32,
    /// Set only once `COMMIT`/`ROLLBACK` succeeded, so a failed finish still
    /// leaves the rollback to `Drop`.
    done: bool,
}

/// The [`Executor`] a [`Tx`] runs on.
pub(crate) type TxConn<C> = <C as std::ops::Deref>::Target;
type TxErr<C> = <TxConn<C> as Executor>::Error;

impl<C> Tx<C>
where
    C: Deref<Target: TxConnection>,
{
    /// Opens a top-level transaction on `conn`.
    pub fn begin(conn: C, options: TxOptions) -> Result<Self, TxErr<C>> {
        Self::open(conn, 0, options)
    }

    fn open(conn: C, depth: u32, options: TxOptions) -> Result<Self, TxErr<C>> {
        let statement = open_statement(depth, options)?;
        let claim = conn.slot().claim(depth)?;

        conn.control(statement)?;
        claim.keep();

        Ok(Self {
            conn,
            depth,
            done: false,
        })
    }

    /// `0` for a transaction, `n` for the `n`th nested savepoint.
    pub fn depth(&self) -> u32 {
        self.depth
    }

    /// The connection the transaction runs on. Statements sent through the
    /// driver's own API bypass the transaction guard; use with care.
    pub fn connection(&self) -> &TxConn<C> {
        &self.conn
    }

    fn finish(&mut self, commit: bool) -> Result<(), TxErr<C>> {
        self.conn.control(finish_statement(commit, self.depth))?;
        self.done = true;
        Ok(())
    }
}

impl<C> Drop for Tx<C>
where
    C: Deref<Target: TxConnection>,
{
    fn drop(&mut self) {
        if !self.done && self.finish(false).is_err() {
            self.conn.defer_rollback(self.depth);
        }

        if self.depth == 0 {
            self.conn.slot().release();
        }
    }
}

impl<C> Engine for Tx<C>
where
    C: Deref<Target: TxConnection>,
{
    type Completion = Immediate;
}

impl<C> Executor for Tx<C>
where
    C: Deref<Target: TxConnection>,
{
    type Dialect = <TxConn<C> as Executor>::Dialect;
    type Error = TxErr<C>;

    fn dialect(&self) -> &Self::Dialect {
        self.conn.dialect()
    }

    fn execute(&self, statement: &Statement, expect: Expect) -> Result<Outcome, Self::Error> {
        self.conn.execute(statement, expect)
    }
}

impl<C> Transaction for Tx<C>
where
    C: Deref<Target: TxConnection>,
{
    fn commit(mut self) -> Result<(), Self::Error> {
        self.finish(true)
    }

    fn rollback(mut self) -> Result<(), Self::Error> {
        self.finish(false)
    }
}

impl<C> Transactional for Tx<C>
where
    C: Deref<Target: TxConnection + Sync> + Send,
{
    type Tx<'s>
        = Tx<&'s TxConn<C>>
    where
        Self: 's;

    fn begin_with(&self, options: TxOptions) -> Result<Self::Tx<'_>, Self::Error> {
        Tx::open(&*self.conn, self.depth + 1, options)
    }
}
