use rupa_core::exec::Blocking;

use super::Session;
use crate::{
    Adoption, CatalogEntry, Coverage, Covered, Direction, Error, HistoryEntry, Inspection, Policy,
    Recovery, VersionScheme, sync,
};

/// A sync session driven through the async contracts. Each call runs on the
/// calling thread and completes when first polled, with no added locking; it
/// lets async applications apply catalogs on sync-only drivers.
impl<S: sync::Session + Send> Session for Blocking<S> {
    const POLICIES: &'static [Policy] = S::POLICIES;

    async fn acquire(
        &mut self,
        stream: &str,
        scheme: VersionScheme,
    ) -> Result<Vec<i64>, Error<S::Error>> {
        self.0.acquire(stream, scheme)
    }

    async fn history(&mut self) -> Result<Vec<HistoryEntry>, Error<S::Error>> {
        self.0.history()
    }

    async fn coverage(&mut self) -> Result<Option<Coverage>, Error<S::Error>> {
        self.0.coverage()
    }

    async fn initialized(&mut self) -> Result<bool, Error<S::Error>> {
        self.0.initialized()
    }

    async fn inspect(&mut self) -> Result<Inspection, Error<S::Error>> {
        self.0.inspect()
    }

    async fn start_attempt(
        &mut self,
        entry: &CatalogEntry,
        direction: Direction,
    ) -> Result<i64, Error<S::Error>> {
        self.0.start_attempt(entry, direction)
    }

    async fn begin_migration(&mut self, policy: Policy) -> Result<(), Error<S::Error>> {
        self.0.begin_migration(policy)
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

    async fn recover(
        &mut self,
        stream: &str,
        scheme: VersionScheme,
        entries: &[CatalogEntry],
        recovery: &Recovery<'_>,
    ) -> Result<(), Error<S::Error>> {
        self.0.recover(stream, scheme, entries, recovery)
    }

    async fn adopt(
        &mut self,
        stream: &str,
        scheme: VersionScheme,
        adoption: &Adoption<'_>,
        members: &[Covered],
    ) -> Result<(), Error<S::Error>> {
        self.0.adopt(stream, scheme, adoption, members)
    }

    fn abandon(&mut self) {
        self.0.abandon();
    }
}
