//! A declared database is a [`Connection`](rupa_core::connection::Connection)
//! like the pool it holds: statements, transactions and repositories run on
//! `Database<D>`, `&Database<D>` or the injected `Arc<Database<D>>` directly.

#[cfg(feature = "async")]
use std::future::Future;

use rupa_core::exec::{Engine, Executor, Outcome};
use rupa_core::ir::Statement;
use rupa_core::query::Expect;
use rupa_core::tx::{Transactional, TxOptions};
#[cfg(feature = "async")]
use {rupa_core::exec::AsyncExecutor, rupa_core::tx::AsyncTransactional};

use super::{Database, Declaration};

impl<D: Declaration> Engine for Database<D>
where
    D::Pool: Engine,
{
    type Completion = <D::Pool as Engine>::Completion;
}

impl<D: Declaration> Executor for Database<D>
where
    D::Pool: Executor,
{
    type Dialect = <D::Pool as Executor>::Dialect;
    type Error = <D::Pool as Executor>::Error;

    fn dialect(&self) -> &Self::Dialect {
        self.pool().dialect()
    }

    fn execute(&self, statement: &Statement, expect: Expect) -> Result<Outcome, Self::Error> {
        self.pool().execute(statement, expect)
    }
}

impl<D: Declaration> Transactional for Database<D>
where
    D::Pool: Transactional,
{
    type Tx<'t> = <D::Pool as Transactional>::Tx<'t>;

    fn begin_with(&self, options: TxOptions) -> Result<Self::Tx<'_>, Self::Error> {
        self.pool().begin_with(options)
    }
}

#[cfg(feature = "async")]
impl<D: Declaration> AsyncExecutor for Database<D>
where
    D::Pool: AsyncExecutor,
{
    type Dialect = <D::Pool as AsyncExecutor>::Dialect;
    type Error = <D::Pool as AsyncExecutor>::Error;

    fn dialect(&self) -> &Self::Dialect {
        self.pool().dialect()
    }

    fn execute(
        &self,
        statement: &Statement,
        expect: Expect,
    ) -> impl Future<Output = Result<Outcome, Self::Error>> + Send {
        self.pool().execute(statement, expect)
    }
}

#[cfg(feature = "async")]
impl<D: Declaration> AsyncTransactional for Database<D>
where
    D::Pool: AsyncTransactional,
{
    type Tx<'t> = <D::Pool as AsyncTransactional>::Tx<'t>;

    fn begin_with(
        &self,
        options: TxOptions,
    ) -> impl Future<Output = Result<Self::Tx<'_>, Self::Error>> + Send {
        self.pool().begin_with(options)
    }
}
