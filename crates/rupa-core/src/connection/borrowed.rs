//! A connection behind `&dyn`, for repository trait objects: what
//! `repo.tx(&tx)` runs on when `repo` is an `Arc<dyn Trait>`.

use std::fmt;
use std::marker::PhantomData;

use super::Connection;
use crate::error::DynError;
#[cfg(feature = "async")]
use crate::exec::{Awaited, BoxFuture};
use crate::exec::{ExecError, Immediate, Outcome, now};
use crate::ir::Statement;
use crate::query::{Expect, Query, QueryResult};

/// How a [`DynConnection`] of one mode returns a statement's outcome.
pub trait DynMode {
    /// What [`DynConnection::run_dyn`] returns.
    type Run<'a>;
}

impl DynMode for Immediate {
    type Run<'a> = Result<Outcome, DynError>;
}

#[cfg(feature = "async")]
impl DynMode for Awaited {
    type Run<'a> = BoxFuture<'a, Result<Outcome, DynError>>;
}

/// A [`Connection`] borrowed as a trait object, in mode `M`: every
/// connection is one for the modes it can serve. A sync connection's
/// statements return directly; nothing is allocated. An async one returns a
/// boxed future, the one allocation a trait object's future needs. Sync
/// connections serve async repositories too.
pub trait DynConnection<M: DynMode>: Sync {
    /// Runs one statement.
    fn run_dyn<'a>(&'a self, statement: &'a Statement, expect: Expect) -> M::Run<'a>;
}

impl<C: Connection<Mode = Immediate> + Sync + ?Sized> DynConnection<Immediate> for C {
    fn run_dyn<'a>(
        &'a self,
        statement: &'a Statement,
        expect: Expect,
    ) -> Result<Outcome, DynError> {
        now(self.run(statement, expect)).map_err(ExecError::into_dyn_error)
    }
}

#[cfg(feature = "async")]
impl<C: Connection + Sync + ?Sized> DynConnection<Awaited> for C {
    fn run_dyn<'a>(
        &'a self,
        statement: &'a Statement,
        expect: Expect,
    ) -> BoxFuture<'a, Result<Outcome, DynError>> {
        Box::pin(async move {
            self.run(statement, expect)
                .await
                .map_err(ExecError::into_dyn_error)
        })
    }
}

/// A repository trait object's queries on a borrowed connection: what
/// `repo.tx(&tx)` returns when `repo` is an `Arc<dyn Trait>`. It holds only
/// the reference; `#[repository]` implements the trait for it, so the
/// trait's methods run on the connection.
///
/// `R` is the repository trait object, `M` the repository's mode and `D` the
/// dialect statically gated query functions are checked against: the
/// connection's is only known at run time.
pub struct RepoOn<'a, R: ?Sized, M: DynMode = Immediate, D = crate::dialect::DynDialect> {
    connection: &'a dyn DynConnection<M>,
    repository: PhantomData<fn() -> (*const R, D)>,
}

impl<'a, R: ?Sized, M: DynMode, D> RepoOn<'a, R, M, D> {
    /// The repository's queries on `connection`.
    pub fn new(connection: &'a dyn DynConnection<M>) -> Self {
        Self {
            connection,
            repository: PhantomData,
        }
    }

    /// The connection the queries run on.
    pub fn connection(&self) -> &'a dyn DynConnection<M> {
        self.connection
    }
}

impl<R: ?Sized, M: DynMode, D> Clone for RepoOn<'_, R, M, D> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<R: ?Sized, M: DynMode, D> Copy for RepoOn<'_, R, M, D> {}

impl<R: ?Sized, M: DynMode, D> fmt::Debug for RepoOn<'_, R, M, D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RepoOn").finish_non_exhaustive()
    }
}

/// Runs `query` on a borrowed sync connection. What generated repository
/// trait objects do; not public API.
#[doc(hidden)]
pub fn complete_dyn<R: QueryResult>(
    connection: &dyn DynConnection<Immediate>,
    query: Query<R>,
) -> Result<R, DynError> {
    let outcome = connection.run_dyn(query.statement(), query.expect())?;

    Ok(outcome.decode()?)
}

/// Runs `query` on a borrowed connection, awaited. What generated repository
/// trait objects do; not public API.
#[cfg(feature = "async")]
#[doc(hidden)]
pub async fn complete_dyn_awaited<R: QueryResult>(
    connection: &dyn DynConnection<Awaited>,
    query: Query<R>,
) -> Result<R, DynError> {
    let outcome = connection
        .run_dyn(query.statement(), query.expect())
        .await?;

    Ok(outcome.decode()?)
}
