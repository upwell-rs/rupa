//! The registry entries, for run-time driver selection.

use std::fmt;
use std::marker::PhantomData;

use rupa_core::config::DbConfig;
use rupa_core::error::ConnectError;
use rupa_core::exec::BoxFuture;
use rupa_core::registry::{Database, Driver};
use rupa_core::{Dialect as _, DialectId};

use crate::{PoolBackend, SqlxPool};

/// The registry entry for `DB`: named `sqlx-postgres`, `sqlx-mysql` or
/// `sqlx-sqlite` (see the crate docs for the URL schemes). Builds a
/// [`SqlxPool`].
pub struct SqlxDriver<DB>(PhantomData<fn() -> DB>);

impl<DB> SqlxDriver<DB> {
    /// The driver.
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}

impl<DB> Default for SqlxDriver<DB> {
    fn default() -> Self {
        Self::new()
    }
}

impl<DB> Clone for SqlxDriver<DB> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<DB> Copy for SqlxDriver<DB> {}

impl<DB: crate::Backend> fmt::Debug for SqlxDriver<DB> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("SqlxDriver").field(&DB::DRIVER).finish()
    }
}

impl<DB: PoolBackend> Driver for SqlxDriver<DB> {
    fn name(&self) -> &'static str {
        DB::DRIVER
    }

    fn dialect(&self) -> DialectId {
        DB::Dialect::default().id()
    }

    fn schemes(&self) -> &'static [&'static str] {
        DB::SCHEMES
    }

    fn connect<'a>(
        &'a self,
        config: &'a DbConfig,
    ) -> BoxFuture<'a, Result<Database, ConnectError>> {
        Box::pin(async move { SqlxPool::<DB>::connect(config).await.map(Database::new) })
    }
}
