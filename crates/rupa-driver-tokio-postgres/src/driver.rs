//! The registry entry, for run-time driver selection.

use rupa_core::DialectId;
use rupa_core::config::DbConfig;
use rupa_core::error::ConnectError;
use rupa_core::exec::BoxFuture;
use rupa_core::registry::{Database, Driver};

use crate::PgPool;

/// The registry entry for this driver: name `postgres`, schemes
/// `postgres` and `postgresql`. Builds a [`PgPool`] without TLS.
#[derive(Debug, Clone, Copy, Default)]
pub struct PgDriver;

impl Driver for PgDriver {
    fn name(&self) -> &'static str {
        "postgres"
    }

    fn dialect(&self) -> DialectId {
        DialectId::Postgres
    }

    fn schemes(&self) -> &'static [&'static str] {
        &["postgres", "postgresql"]
    }

    fn connect<'a>(
        &'a self,
        config: &'a DbConfig,
    ) -> BoxFuture<'a, Result<Database, ConnectError>> {
        Box::pin(async move { PgPool::connect(config).await.map(Database::new) })
    }
}
