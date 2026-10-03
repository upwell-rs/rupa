//! Synchronous RUPA executor on the [`postgres`] crate.
//!
//! Every query is rendered on each call; caching rendered SQL per query site
//! is a planned optimization.
//!
//! # Transactions
//!
//! [`PgExecutor`] is [`Transactional`]: `begin` returns a [`PgTx`], itself an
//! executor, and `begin` on a `PgTx` opens a savepoint. A `PgTx` dropped
//! without `commit`/`rollback` rolls back immediately. If that rollback fails
//! (e.g. the connection broke), the executor is marked dirty, and the rollback
//! is retried before the next statement.

use postgres::fallible_iterator::FallibleIterator;
use postgres::types::ToSql;
use rupa_core::Postgres;
use rupa_core::TxError;
use rupa_core::exec::{Executor, Outcome};
use rupa_core::ir::{Statement, TxStatement};
use rupa_core::query::Expect;
use rupa_core::tx::{Transaction, Transactional, TxOptions};
use rupa_driver_tokio_postgres::shared::{Param, PgRows};

pub use rupa_driver_tokio_postgres::PgError;

/// Executes queries on a `postgres::Client`.
pub struct PgExecutor {
    client: postgres::Client,
    dialect: Postgres,
    /// A rollback a dropped transaction could not complete: the depth to
    /// roll back to (`0` is the whole transaction).
    pending_rollback: Option<u32>,
}

impl PgExecutor {
    pub fn new(client: postgres::Client) -> Self {
        Self {
            client,
            dialect: Postgres,
            pending_rollback: None,
        }
    }

    /// The underlying client. Statements sent through it directly bypass the
    /// pending-rollback check; call [`clean`](Self::clean) first.
    pub fn client(&mut self) -> &mut postgres::Client {
        &mut self.client
    }

    pub fn into_inner(self) -> postgres::Client {
        self.client
    }

    /// Whether a dropped transaction still has to be rolled back.
    pub fn is_dirty(&self) -> bool {
        self.pending_rollback.is_some()
    }

    /// Sends the rollback owed by a dropped transaction, if any.
    pub fn clean(&mut self) -> Result<(), PgError> {
        if let Some(depth) = self.pending_rollback {
            self.client
                .batch_execute(&rupa_sql::render_tx(&rollback_to(depth), &self.dialect)?)?;
            self.pending_rollback = None;
        }
        Ok(())
    }

    fn mark_dirty(&mut self, depth: u32) {
        self.pending_rollback = Some(self.pending_rollback.map_or(depth, |d| d.min(depth)));
    }

    fn control(&mut self, statement: TxStatement) -> Result<(), PgError> {
        self.clean()?;
        let sql = rupa_sql::render_tx(&statement, &self.dialect)?;
        Ok(self.client.batch_execute(&sql)?)
    }

    fn run(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, PgError> {
        self.clean()?;
        let rendered = rupa_sql::render(statement, &self.dialect)?;
        let params: Vec<Param<'_>> = rendered.params.iter().map(Param).collect();
        let refs: Vec<&(dyn ToSql + Sync)> =
            params.iter().map(|p| p as &(dyn ToSql + Sync)).collect();
        let sql = rendered.sql.as_str();

        if expect == Expect::Affected {
            return Ok(Outcome::Affected(self.client.execute(sql, &refs)?));
        }
        let rows = match expect.fetch_limit() {
            None => self.client.query(sql, &refs)?,
            Some(limit) => self
                .client
                .query_raw(sql, refs.iter().copied())?
                .take(limit)
                .collect()?,
        };
        Ok(Outcome::Rows(Box::new(PgRows::new(rows))))
    }
}

fn rollback_to(depth: u32) -> TxStatement {
    match depth {
        0 => TxStatement::Rollback,
        d => TxStatement::RollbackToSavepoint(d),
    }
}

impl Executor for PgExecutor {
    type Dialect = Postgres;
    type Error = PgError;

    fn dialect(&self) -> &Postgres {
        &self.dialect
    }

    fn execute(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, PgError> {
        self.run(statement, expect)
    }
}

impl Transactional for PgExecutor {
    type Tx<'t>
        = PgTx<'t>
    where
        Self: 't;

    fn begin_with(&mut self, options: TxOptions) -> Result<PgTx<'_>, PgError> {
        self.control(TxStatement::Begin(options))?;
        Ok(PgTx {
            exec: self,
            depth: 0,
            done: false,
        })
    }
}

/// An open transaction (`depth == 0`) or savepoint (`depth > 0`). Rolled
/// back when dropped without `commit`.
pub struct PgTx<'t> {
    exec: &'t mut PgExecutor,
    depth: u32,
    done: bool,
}

impl PgTx<'_> {
    fn finish(&mut self, commit: bool) -> Result<(), PgError> {
        let statement = match (commit, self.depth) {
            (true, 0) => TxStatement::Commit,
            (true, d) => TxStatement::ReleaseSavepoint(d),
            (false, d) => rollback_to(d),
        };
        self.exec.control(statement)?;
        self.done = true;
        Ok(())
    }
}

impl Drop for PgTx<'_> {
    fn drop(&mut self) {
        if !self.done && self.finish(false).is_err() {
            self.exec.mark_dirty(self.depth);
        }
    }
}

impl Executor for PgTx<'_> {
    type Dialect = Postgres;
    type Error = PgError;

    fn dialect(&self) -> &Postgres {
        &self.exec.dialect
    }

    fn execute(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, PgError> {
        self.exec.run(statement, expect)
    }
}

impl Transaction for PgTx<'_> {
    fn commit(mut self) -> Result<(), PgError> {
        self.finish(true)
    }

    fn rollback(mut self) -> Result<(), PgError> {
        self.finish(false)
    }
}

impl Transactional for PgTx<'_> {
    type Tx<'s>
        = PgTx<'s>
    where
        Self: 's;

    fn begin_with(&mut self, options: TxOptions) -> Result<PgTx<'_>, PgError> {
        if !options.is_default() {
            return Err(PgError::Tx(TxError::OptionsOnNested));
        }
        let depth = self.depth + 1;
        self.exec.control(TxStatement::Savepoint(depth))?;
        Ok(PgTx {
            exec: &mut *self.exec,
            depth,
            done: false,
        })
    }
}
