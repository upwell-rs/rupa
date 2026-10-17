//! Transactions.
//!
//! Two APIs over the same machinery:
//!
//! - **Guard:** `begin()` returns a transaction that is itself an executor,
//!   so code generic over [`Executor`] / [`AsyncExecutor`] runs unchanged
//!   inside or outside a transaction. Calling `begin()` on a transaction opens
//!   a nested one (a savepoint).
//! - **Closure:** `transaction(|tx| ..)` commits on `Ok` and rolls back on `Err`.
//!
//! # Who implements what
//!
//! Drivers implement [`TxConnection`] (sync) or [`AsyncTxConnection`]: "run
//! this transaction-control statement" plus the dirty-connection bookkeeping
//! below. Everything else lives here, once:
//!
//! - [`Tx`] / [`AsyncTx`], the transaction guards, generic over how they hold
//!   the connection: borrowed (`Tx<&mut Conn>`, what `begin()` returns) or
//!   owned (`AsyncTx<Box<Conn>>`, what a pool's `begin()` returns, so a
//!   transaction can outlive the borrow of its pool, e.g. in a request scope).
//! - Blanket [`Transactional`] / [`AsyncTransactional`] impls for every
//!   connection.
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

use std::future::Future;
use std::ops::DerefMut;

use crate::error::TxError;
use crate::exec::{AsyncExecutor, Executor, Outcome};
pub use crate::ir::{IsolationLevel, TxOptions};
use crate::ir::{Statement, TxStatement};
use crate::query::Expect;

// ---------------------------------------------------------------------------
// Public traits
// ---------------------------------------------------------------------------

/// A transaction that is still open.
pub trait Transaction: Executor + Sized {
    /// Commits the transaction (or releases the savepoint).
    fn commit(self) -> Result<(), Self::Error>;
    /// Rolls the transaction (or savepoint) back.
    fn rollback(self) -> Result<(), Self::Error>;
}

/// An executor that can open transactions. Transactions are themselves
/// transactional: `begin` on a transaction opens a savepoint.
pub trait Transactional: Executor {
    /// The transaction type. `Send` so a transaction can be driven from async
    /// code (and erased for DI) like any connection; every supported sync
    /// driver's connection is.
    type Tx<'t>: Transaction
        + Transactional
        + Send
        + Executor<Dialect = Self::Dialect, Error = Self::Error>
        + 't
    where
        Self: 't;

    /// Starts a transaction with `options`. On a transaction, options must be
    /// the default (savepoints have none); anything else is an error.
    fn begin_with(&mut self, options: TxOptions) -> Result<Self::Tx<'_>, Self::Error>;

    /// Starts a transaction with default options.
    fn begin(&mut self) -> Result<Self::Tx<'_>, Self::Error> {
        self.begin_with(TxOptions::default())
    }

    /// Runs `f` in a transaction: commits if it returns `Ok`, rolls back if it
    /// returns `Err` or panics.
    ///
    /// The closure's transaction borrows from this call (`Tx<'a>`), rather than
    /// being higher-ranked: with a GAT `where Self: 't`, a higher-ranked
    /// lifetime would force `Self: 'static` and rule out nested calls on a `Tx`.
    fn transaction<'a, T, E, F>(&'a mut self, f: F) -> Result<T, E>
    where
        F: FnOnce(&mut Self::Tx<'a>) -> Result<T, E>,
        E: From<Self::Error>,
    {
        let mut tx = self.begin()?;
        match f(&mut tx) {
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

/// An open async transaction.
pub trait AsyncTransaction: AsyncExecutor + Sized {
    /// Commits the transaction (or releases the savepoint).
    fn commit(self) -> impl Future<Output = Result<(), Self::Error>> + Send;
    /// Rolls the transaction (or savepoint) back.
    fn rollback(self) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

/// An async executor that can open transactions.
pub trait AsyncTransactional: AsyncExecutor {
    /// The transaction type.
    type Tx<'t>: AsyncTransaction
        + AsyncTransactional
        + AsyncExecutor<Dialect = Self::Dialect, Error = Self::Error>
        + 't
    where
        Self: 't;

    /// Starts a transaction with `options`. On a transaction, options must be
    /// the default (savepoints have none); anything else is an error.
    fn begin_with(
        &mut self,
        options: TxOptions,
    ) -> impl Future<Output = Result<Self::Tx<'_>, Self::Error>> + Send;

    /// Starts a transaction with default options.
    fn begin(&mut self) -> impl Future<Output = Result<Self::Tx<'_>, Self::Error>> + Send {
        self.begin_with(TxOptions::default())
    }

    /// Runs `f` in a transaction: commits if it returns `Ok`, rolls back if it
    /// returns `Err`.
    ///
    /// The returned future is `Send` whenever `f`'s future is; with a concrete
    /// executor type the compiler sees that. Generic code that must spawn the
    /// future can use the guard API (`begin` / `commit`) instead.
    fn transaction<'a, T, E, F>(&'a mut self, f: F) -> impl Future<Output = Result<T, E>>
    where
        F: AsyncFnOnce(&mut Self::Tx<'a>) -> Result<T, E>,
        E: From<Self::Error>,
    {
        async move {
            let mut tx = self.begin().await?;
            match f(&mut tx).await {
                Ok(value) => {
                    tx.commit().await?;
                    Ok(value)
                }
                Err(e) => {
                    let _ = tx.rollback().await;
                    Err(e)
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// What drivers implement
// ---------------------------------------------------------------------------

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
pub trait TxConnection: Executor<Error: From<TxError>> + Send {
    /// Runs a transaction-control statement now (after any deferred rollback).
    fn control(&mut self, statement: TxStatement) -> Result<(), Self::Error>;

    /// Records a rollback to `depth` (`0` = the whole transaction) that must
    /// be sent before the next statement.
    fn defer_rollback(&mut self, depth: u32);

    /// Whether a deferred rollback has not been sent yet.
    fn is_dirty(&self) -> bool;

    /// Sends the deferred rollback, if any. Pools call this before reusing a
    /// connection.
    fn clean(&mut self) -> Result<(), Self::Error>;
}

/// Async counterpart of [`TxConnection`]. Implementing it makes the connection
/// [`AsyncTransactional`], with [`AsyncTx`] as its transaction.
///
/// Dropping an [`AsyncTx`] calls [`defer_rollback`](Self::defer_rollback),
/// which cannot await: the rollback is sent before the next statement, or by
/// [`clean`](Self::clean) when a pool takes the connection back.
pub trait AsyncTxConnection: AsyncExecutor<Error: From<TxError>> {
    /// Runs a transaction-control statement now (after any deferred rollback).
    fn control(
        &mut self,
        statement: TxStatement,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// Records a rollback to `depth` (`0` = the whole transaction) that must
    /// be sent before the next statement.
    fn defer_rollback(&mut self, depth: u32);

    /// Whether a deferred rollback has not been sent yet.
    fn is_dirty(&self) -> bool;

    /// Sends the deferred rollback, if any. Pools call this before reusing a
    /// connection.
    fn clean(&mut self) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

/// Bookkeeping for a deferred rollback, for drivers to embed.
///
/// ```ignore
/// fn run(&mut self, ..) -> Result<Outcome, MyError> {
///     self.clean()?; // sends `self.pending.statement()` and clears it
///     ..
/// }
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PendingRollback(Option<u32>);

impl PendingRollback {
    /// Records a rollback to `depth`. Several deferred rollbacks merge into
    /// the outermost one.
    pub fn defer(&mut self, depth: u32) {
        self.0 = Some(self.0.map_or(depth, |d| d.min(depth)));
    }

    /// Whether a rollback is owed.
    pub fn is_pending(&self) -> bool {
        self.0.is_some()
    }

    /// The statement to send, if a rollback is owed. Call [`clear`](Self::clear)
    /// once it succeeded.
    pub fn statement(&self) -> Option<TxStatement> {
        self.0.map(rollback_to)
    }

    /// Marks the owed rollback as sent.
    pub fn clear(&mut self) {
        self.0 = None;
    }
}

/// The statement that ends a transaction (`depth == 0`) or savepoint.
pub fn rollback_to(depth: u32) -> TxStatement {
    match depth {
        0 => TxStatement::Rollback,
        d => TxStatement::RollbackToSavepoint(d),
    }
}

fn finish_statement(commit: bool, depth: u32) -> TxStatement {
    match (commit, depth) {
        (true, 0) => TxStatement::Commit,
        (true, d) => TxStatement::ReleaseSavepoint(d),
        (false, d) => rollback_to(d),
    }
}

/// The statement that opens a transaction (`depth == 0`) or savepoint.
fn open_statement<E: From<TxError>>(depth: u32, options: TxOptions) -> Result<TxStatement, E> {
    match depth {
        0 => Ok(TxStatement::Begin(options)),
        _ if !options.is_default() => Err(TxError::OptionsOnNested.into()),
        d => Ok(TxStatement::Savepoint(d)),
    }
}

// ---------------------------------------------------------------------------
// Sync transaction guard
// ---------------------------------------------------------------------------

/// An open sync transaction (`depth == 0`) or savepoint (`depth > 0`) on the
/// connection `C` holds: `&mut Conn` when borrowed, or an owning handle (a
/// `Box`, a pooled connection) when the transaction must be `'static`.
///
/// Rolled back when dropped without `commit` or `rollback`.
pub struct Tx<C>
where
    C: DerefMut<Target: TxConnection>,
{
    conn: C,
    depth: u32,
    /// Set only once `COMMIT`/`ROLLBACK` succeeded, so a failed finish still
    /// leaves the rollback to `Drop`.
    done: bool,
}

/// The [`Executor`] a [`Tx`] runs on.
type TxConn<C> = <C as std::ops::Deref>::Target;
type TxErr<C> = <TxConn<C> as Executor>::Error;

impl<C> Tx<C>
where
    C: DerefMut<Target: TxConnection>,
{
    /// Opens a top-level transaction on `conn`.
    pub fn begin(conn: C, options: TxOptions) -> Result<Self, TxErr<C>> {
        Self::open(conn, 0, options)
    }

    fn open(mut conn: C, depth: u32, options: TxOptions) -> Result<Self, TxErr<C>> {
        conn.control(open_statement(depth, options)?)?;
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
    pub fn connection(&mut self) -> &mut TxConn<C> {
        &mut self.conn
    }

    fn finish(&mut self, commit: bool) -> Result<(), TxErr<C>> {
        self.conn.control(finish_statement(commit, self.depth))?;
        self.done = true;
        Ok(())
    }
}

impl<C> Drop for Tx<C>
where
    C: DerefMut<Target: TxConnection>,
{
    fn drop(&mut self) {
        if !self.done && self.finish(false).is_err() {
            self.conn.defer_rollback(self.depth);
        }
    }
}

impl<C> Executor for Tx<C>
where
    C: DerefMut<Target: TxConnection>,
{
    type Dialect = <TxConn<C> as Executor>::Dialect;
    type Error = TxErr<C>;

    fn dialect(&self) -> &Self::Dialect {
        self.conn.dialect()
    }

    fn execute(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, Self::Error> {
        self.conn.execute(statement, expect)
    }
}

impl<C> Transaction for Tx<C>
where
    C: DerefMut<Target: TxConnection>,
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
    C: DerefMut<Target: TxConnection> + Send,
{
    type Tx<'s>
        = Tx<&'s mut TxConn<C>>
    where
        Self: 's;

    fn begin_with(&mut self, options: TxOptions) -> Result<Self::Tx<'_>, Self::Error> {
        Tx::open(&mut *self.conn, self.depth + 1, options)
    }
}

impl<E: TxConnection> Transactional for E {
    type Tx<'t>
        = Tx<&'t mut E>
    where
        Self: 't;

    fn begin_with(&mut self, options: TxOptions) -> Result<Tx<&mut E>, E::Error> {
        Tx::begin(self, options)
    }
}

// ---------------------------------------------------------------------------
// Async transaction guard
// ---------------------------------------------------------------------------

/// An open async transaction (`depth == 0`) or savepoint (`depth > 0`) on
/// the connection `C` holds; see [`Tx`].
///
/// Dropped without `commit` or `rollback`, it defers the rollback to the
/// connection (see the [module docs](self)).
pub struct AsyncTx<C>
where
    C: DerefMut<Target: AsyncTxConnection>,
{
    conn: C,
    depth: u32,
    /// Set only once `COMMIT`/`ROLLBACK` succeeded, so a cancelled or failed
    /// finish still leaves the rollback to `Drop`.
    done: bool,
}

type AsyncErr<C> = <TxConn<C> as AsyncExecutor>::Error;

impl<C> AsyncTx<C>
where
    C: DerefMut<Target: AsyncTxConnection> + Send,
{
    /// Opens a top-level transaction on `conn`.
    pub async fn begin(conn: C, options: TxOptions) -> Result<Self, AsyncErr<C>> {
        Self::open(conn, 0, options).await
    }

    async fn open(mut conn: C, depth: u32, options: TxOptions) -> Result<Self, AsyncErr<C>> {
        conn.control(open_statement(depth, options)?).await?;
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
    pub fn connection(&mut self) -> &mut TxConn<C> {
        &mut self.conn
    }

    async fn finish(mut self, commit: bool) -> Result<(), AsyncErr<C>> {
        self.conn
            .control(finish_statement(commit, self.depth))
            .await?;
        self.done = true;
        Ok(())
    }
}

impl<C> Drop for AsyncTx<C>
where
    C: DerefMut<Target: AsyncTxConnection>,
{
    fn drop(&mut self) {
        if !self.done {
            self.conn.defer_rollback(self.depth);
        }
    }
}

impl<C> AsyncExecutor for AsyncTx<C>
where
    C: DerefMut<Target: AsyncTxConnection> + Send,
{
    type Dialect = <TxConn<C> as AsyncExecutor>::Dialect;
    type Error = AsyncErr<C>;

    fn dialect(&self) -> &Self::Dialect {
        self.conn.dialect()
    }

    fn execute(
        &mut self,
        statement: &Statement,
        expect: Expect,
    ) -> impl Future<Output = Result<Outcome, Self::Error>> + Send {
        self.conn.execute(statement, expect)
    }
}

impl<C> AsyncTransaction for AsyncTx<C>
where
    C: DerefMut<Target: AsyncTxConnection> + Send,
{
    fn commit(self) -> impl Future<Output = Result<(), Self::Error>> + Send {
        self.finish(true)
    }

    fn rollback(self) -> impl Future<Output = Result<(), Self::Error>> + Send {
        self.finish(false)
    }
}

impl<C> AsyncTransactional for AsyncTx<C>
where
    C: DerefMut<Target: AsyncTxConnection> + Send,
{
    type Tx<'s>
        = AsyncTx<&'s mut TxConn<C>>
    where
        Self: 's;

    fn begin_with(
        &mut self,
        options: TxOptions,
    ) -> impl Future<Output = Result<Self::Tx<'_>, Self::Error>> + Send {
        AsyncTx::open(&mut *self.conn, self.depth + 1, options)
    }
}

impl<E: AsyncTxConnection> AsyncTransactional for E {
    type Tx<'t>
        = AsyncTx<&'t mut E>
    where
        Self: 't;

    fn begin_with(
        &mut self,
        options: TxOptions,
    ) -> impl Future<Output = Result<AsyncTx<&mut E>, E::Error>> + Send {
        AsyncTx::begin(self, options)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_rollbacks_merge_into_the_outermost() {
        let mut p = PendingRollback::default();
        assert_eq!(p.statement(), None);
        p.defer(2);
        p.defer(1);
        p.defer(3);
        assert_eq!(p.statement(), Some(TxStatement::RollbackToSavepoint(1)));
        p.defer(0);
        assert_eq!(p.statement(), Some(TxStatement::Rollback));
        p.clear();
        assert!(!p.is_pending());
    }

    #[test]
    fn nested_transactions_refuse_options() {
        assert_eq!(
            open_statement::<TxError>(1, TxOptions::default().read_only()),
            Err(TxError::OptionsOnNested)
        );
        assert_eq!(
            open_statement::<TxError>(2, TxOptions::default()),
            Ok(TxStatement::Savepoint(2))
        );
    }
}
