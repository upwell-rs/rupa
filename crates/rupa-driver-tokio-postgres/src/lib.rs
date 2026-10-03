//! Async RUPA executor on [`tokio_postgres`].
//!
//! Every query is rendered on each call; caching rendered SQL per query site
//! is a planned optimization.
//!
//! # Transactions
//!
//! [`PgExecutor`] is [`AsyncTransactional`]: `begin` returns a [`PgTx`],
//! itself an executor, and `begin` on a `PgTx` opens a savepoint. Rust has no
//! async `Drop`, so a `PgTx` dropped without `commit`/`rollback` cannot roll
//! back on the spot. Instead it marks the executor dirty, and the rollback is
//! sent before the connection's next statement. A pool must check
//! [`PgExecutor::is_dirty`] (or call [`PgExecutor::clean`]) before reusing a
//! connection.

#[doc(hidden)]
pub mod shared;

use futures_util::{StreamExt, TryStreamExt, pin_mut};
use postgres_types::ToSql;
use rupa_core::Postgres;
use rupa_core::TxError;
use rupa_core::exec::{AsyncExecutor, Outcome};
use rupa_core::ir::{Statement, TxStatement};
use rupa_core::query::Expect;
use rupa_core::tx::{AsyncTransaction, AsyncTransactional, TxOptions};

pub use shared::PgError;
use shared::{Param, PgRows};

/// Executes queries on a `tokio_postgres::Client`.
pub struct PgExecutor {
    client: tokio_postgres::Client,
    dialect: Postgres,
    /// A rollback owed by a dropped transaction: the depth to roll back to
    /// (`0` is the whole transaction). Sent before the next statement.
    pending_rollback: Option<u32>,
}

impl PgExecutor {
    pub fn new(client: tokio_postgres::Client) -> Self {
        Self {
            client,
            dialect: Postgres,
            pending_rollback: None,
        }
    }

    /// The underlying client. Statements sent through it directly bypass the
    /// pending-rollback check; call [`clean`](Self::clean) first.
    pub fn client(&self) -> &tokio_postgres::Client {
        &self.client
    }

    pub fn into_inner(self) -> tokio_postgres::Client {
        self.client
    }

    /// Whether a dropped transaction still has to be rolled back.
    pub fn is_dirty(&self) -> bool {
        self.pending_rollback.is_some()
    }

    /// Sends the rollback owed by a dropped transaction, if any.
    pub async fn clean(&mut self) -> Result<(), PgError> {
        if let Some(depth) = self.pending_rollback {
            let statement = match depth {
                0 => TxStatement::Rollback,
                d => TxStatement::RollbackToSavepoint(d),
            };
            self.client
                .batch_execute(&rupa_sql::render_tx(&statement, &self.dialect)?)
                .await?;
            self.pending_rollback = None;
        }
        Ok(())
    }

    fn mark_dirty(&mut self, depth: u32) {
        self.pending_rollback = Some(self.pending_rollback.map_or(depth, |d| d.min(depth)));
    }

    async fn control(&mut self, statement: TxStatement) -> Result<(), PgError> {
        self.clean().await?;
        let sql = rupa_sql::render_tx(&statement, &self.dialect)?;
        Ok(self.client.batch_execute(&sql).await?)
    }

    async fn run(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, PgError> {
        self.clean().await?;
        let rendered = rupa_sql::render(statement, &self.dialect)?;
        let params: Vec<Param<'_>> = rendered.params.iter().map(Param).collect();
        let refs: Vec<&(dyn ToSql + Sync)> =
            params.iter().map(|p| p as &(dyn ToSql + Sync)).collect();
        let sql = rendered.sql.as_str();

        if expect == Expect::Affected {
            return Ok(Outcome::Affected(self.client.execute(sql, &refs).await?));
        }
        let rows = match expect.fetch_limit() {
            None => self.client.query(sql, &refs).await?,
            Some(limit) => {
                let stream = self.client.query_raw(sql, refs.iter().copied()).await?;
                pin_mut!(stream);
                stream.take(limit).try_collect().await?
            }
        };
        Ok(Outcome::Rows(Box::new(PgRows::new(rows))))
    }
}

impl AsyncExecutor for PgExecutor {
    type Dialect = Postgres;
    type Error = PgError;

    fn dialect(&self) -> &Postgres {
        &self.dialect
    }

    async fn execute(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, PgError> {
        self.run(statement, expect).await
    }
}

impl AsyncTransactional for PgExecutor {
    type Tx<'t>
        = PgTx<'t>
    where
        Self: 't;

    async fn begin_with(&mut self, options: TxOptions) -> Result<PgTx<'_>, PgError> {
        self.control(TxStatement::Begin(options)).await?;
        Ok(PgTx {
            exec: self,
            depth: 0,
            done: false,
        })
    }
}

/// An open transaction (`depth == 0`) or savepoint (`depth > 0`).
pub struct PgTx<'t> {
    exec: &'t mut PgExecutor,
    depth: u32,
    /// Set only once `COMMIT`/`ROLLBACK` succeeded, so a cancelled or failed
    /// finish still leaves the rollback to `Drop`.
    done: bool,
}

impl PgTx<'_> {
    async fn finish(mut self, commit: bool) -> Result<(), PgError> {
        let statement = match (commit, self.depth) {
            (true, 0) => TxStatement::Commit,
            (true, d) => TxStatement::ReleaseSavepoint(d),
            (false, 0) => TxStatement::Rollback,
            (false, d) => TxStatement::RollbackToSavepoint(d),
        };
        self.exec.control(statement).await?;
        self.done = true;
        Ok(())
    }
}

impl Drop for PgTx<'_> {
    fn drop(&mut self) {
        if !self.done {
            self.exec.mark_dirty(self.depth);
        }
    }
}

impl AsyncExecutor for PgTx<'_> {
    type Dialect = Postgres;
    type Error = PgError;

    fn dialect(&self) -> &Postgres {
        &self.exec.dialect
    }

    async fn execute(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, PgError> {
        self.exec.run(statement, expect).await
    }
}

impl AsyncTransaction for PgTx<'_> {
    async fn commit(self) -> Result<(), PgError> {
        self.finish(true).await
    }

    async fn rollback(self) -> Result<(), PgError> {
        self.finish(false).await
    }
}

impl AsyncTransactional for PgTx<'_> {
    type Tx<'s>
        = PgTx<'s>
    where
        Self: 's;

    async fn begin_with(&mut self, options: TxOptions) -> Result<PgTx<'_>, PgError> {
        if !options.is_default() {
            return Err(PgError::Tx(TxError::OptionsOnNested));
        }
        let depth = self.depth + 1;
        self.exec.control(TxStatement::Savepoint(depth)).await?;
        Ok(PgTx {
            exec: &mut *self.exec,
            depth,
            done: false,
        })
    }
}
