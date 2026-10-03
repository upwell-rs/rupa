//! Synchronous RUPA executor on the [`postgres`] crate.
//!
//! Every query is rendered on each call; caching rendered SQL per query site
//! is a planned optimization.

use postgres::fallible_iterator::FallibleIterator;
use postgres::types::ToSql;
use rupa_core::Postgres;
use rupa_core::exec::{Executor, Outcome};
use rupa_core::ir::Statement;
use rupa_core::query::Expect;
use rupa_driver_tokio_postgres::shared::{Param, PgRows};

pub use rupa_driver_tokio_postgres::PgError;

/// Executes queries on a `postgres::Client`.
pub struct PgExecutor {
    client: postgres::Client,
    dialect: Postgres,
}

impl PgExecutor {
    pub fn new(client: postgres::Client) -> Self {
        Self {
            client,
            dialect: Postgres,
        }
    }

    pub fn client(&mut self) -> &mut postgres::Client {
        &mut self.client
    }

    pub fn into_inner(self) -> postgres::Client {
        self.client
    }
}

impl Executor for PgExecutor {
    type Dialect = Postgres;
    type Error = PgError;

    fn dialect(&self) -> &Postgres {
        &self.dialect
    }

    fn execute(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, PgError> {
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
