//! Async RUPA executor on [`tokio_postgres`].
//!
//! Every query is rendered on each call; caching rendered SQL per query site
//! is a planned optimization.

#[doc(hidden)]
pub mod shared;

use futures_util::{StreamExt, TryStreamExt, pin_mut};
use postgres_types::ToSql;
use rupa_core::Postgres;
use rupa_core::exec::{AsyncExecutor, Outcome};
use rupa_core::ir::Statement;
use rupa_core::query::Expect;

pub use shared::PgError;
use shared::{Param, PgRows};

/// Executes queries on a `tokio_postgres::Client`.
pub struct PgExecutor {
    client: tokio_postgres::Client,
    dialect: Postgres,
}

impl PgExecutor {
    pub fn new(client: tokio_postgres::Client) -> Self {
        Self {
            client,
            dialect: Postgres,
        }
    }

    pub fn client(&self) -> &tokio_postgres::Client {
        &self.client
    }

    pub fn into_inner(self) -> tokio_postgres::Client {
        self.client
    }
}

impl AsyncExecutor for PgExecutor {
    type Dialect = Postgres;
    type Error = PgError;

    fn dialect(&self) -> &Postgres {
        &self.dialect
    }

    async fn execute(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, PgError> {
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
