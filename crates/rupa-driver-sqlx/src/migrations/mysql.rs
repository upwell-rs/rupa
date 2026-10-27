use std::time::Duration;

use rupa_core::exec::Outcome;
use rupa_core::ir::Statement;
use rupa_core::query::Expect;
use rupa_migrate::protocol::mysql::State;

use super::Engine;
use crate::{SqlxError, SqlxExecutor};

rupa_migrate::__session!(async, mysql, SqlxExecutor<sqlx::MySql>);

/// MySQL has no client-side authorizer, so typed statements are refused
/// by the same grammar as scripts.
impl Engine<sqlx::MySql> for State {
    fn is_active(&self) -> bool {
        State::is_active(self)
    }

    fn is_running(&self) -> bool {
        State::is_running(self)
    }

    fn lock_timeout(&mut self) -> &mut Duration {
        &mut self.lock_timeout
    }

    async fn user_statement<'e>(
        executor: &'e mut SqlxExecutor<sqlx::MySql>,
        statement: &'e Statement,
        expect: Expect,
    ) -> Result<Outcome, SqlxError> {
        let rendered = rupa_sql::render_cached(statement, &executor.dialect)?;

        executor.migration.user_statement(&rendered.sql)?;

        executor.dispatch(statement, expect).await
    }
}
