use rupa_core::exec::Blocking;

use super::Session;
use crate::{CatalogEntry, Error, HistoryEntry, VersionScheme, sync};

/// A sync session driven through the async contracts. Each call runs on the
/// calling thread and completes when first polled, with no added locking; it
/// lets async applications apply catalogs on sync-only drivers.
impl<S: sync::Session + Send> Session for Blocking<S> {
    async fn acquire(
        &mut self,
        stream: &str,
        scheme: VersionScheme,
    ) -> Result<(), Error<S::Error>> {
        self.0.acquire(stream, scheme)
    }

    async fn history(&mut self) -> Result<Vec<HistoryEntry>, Error<S::Error>> {
        self.0.history()
    }

    async fn start_attempt(&mut self, entry: &CatalogEntry) -> Result<i64, Error<S::Error>> {
        self.0.start_attempt(entry)
    }

    async fn begin_migration(&mut self) -> Result<(), Error<S::Error>> {
        self.0.begin_migration()
    }

    async fn complete(
        &mut self,
        attempt: i64,
        entry: &CatalogEntry,
    ) -> Result<(), Error<S::Error>> {
        self.0.complete(attempt, entry)
    }

    async fn abort(&mut self, attempt: i64) -> Result<(), Error<S::Error>> {
        self.0.abort(attempt)
    }

    async fn release(&mut self) -> Result<(), Error<S::Error>> {
        self.0.release()
    }

    async fn script(&mut self, sql: &str) -> Result<(), Error<S::Error>> {
        self.0.script(sql)
    }
}
