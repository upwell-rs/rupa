//! The per-connection transaction state drivers embed: the rollback a
//! dropped transaction owes, and whether the top-level transaction is open.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use super::rollback_to;
use crate::error::TxError;
use crate::ir::TxStatement;

/// Bookkeeping for a deferred rollback, for drivers to embed. It is updated
/// through `&self`, without a lock, and publishes nothing but its own value,
/// so relaxed ordering suffices.
///
/// ```ignore
/// fn run(&self, ..) -> Result<Outcome, MyError> {
///     self.clean()?; // sends `self.pending.statement()` and clears it
///     ..
/// }
/// ```
#[derive(Debug)]
pub struct PendingRollback(AtomicU64);

impl PendingRollback {
    const NONE: u64 = u64::MAX;

    /// Records a rollback to `depth`. Several deferred rollbacks merge into
    /// the outermost one.
    pub fn defer(&self, depth: u32) {
        self.0.fetch_min(u64::from(depth), Ordering::Relaxed);
    }

    /// Whether a rollback is owed.
    pub fn is_pending(&self) -> bool {
        self.0.load(Ordering::Relaxed) != Self::NONE
    }

    /// The statement to send, if a rollback is owed. Call [`clear`](Self::clear)
    /// with it once it succeeded.
    pub fn statement(&self) -> Option<TxStatement> {
        let depth = self.0.load(Ordering::Relaxed);

        u32::try_from(depth).ok().map(rollback_to)
    }

    /// Marks `sent`, a statement [`statement`](Self::statement) returned, as
    /// sent. A rollback to an outer depth deferred while it was in flight
    /// stays owed.
    pub fn clear(&self, sent: TxStatement) {
        let depth = match sent {
            TxStatement::Rollback => 0,
            TxStatement::RollbackToSavepoint(depth) => depth,
            _ => return,
        };

        // A failed exchange means an outer rollback was deferred meanwhile,
        // which must stay owed.
        let _ = self.0.compare_exchange(
            u64::from(depth),
            Self::NONE,
            Ordering::Relaxed,
            Ordering::Relaxed,
        );
    }
}

impl Default for PendingRollback {
    fn default() -> Self {
        Self(AtomicU64::new(Self::NONE))
    }
}

/// How deep a connection's open transactions go, for drivers to embed and
/// return from [`TxConnection::slot`](super::TxConnection::slot): `0` with
/// none open, `n + 1` while the transaction or savepoint at depth `n` is.
///
/// A transaction or savepoint opens only at the innermost depth, one per
/// level: a second `BEGIN` would commit (MySQL) or join (Postgres) the
/// first, and two sibling savepoints would share one name, so either fails
/// with [`TxError::AlreadyOpen`].
#[derive(Debug, Default)]
pub struct TxSlot(AtomicU32);

impl TxSlot {
    /// Whether a top-level transaction is open.
    pub fn is_open(&self) -> bool {
        self.0.load(Ordering::Acquire) > 0
    }

    /// Takes the level at `depth`, which must be the innermost one.
    pub(crate) fn claim(&self, depth: u32) -> Result<Claim<'_>, TxError> {
        self.0
            .compare_exchange(depth, depth + 1, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| TxError::AlreadyOpen)?;

        Ok(Claim { slot: self, depth })
    }

    /// Frees the level at `depth` once its transaction or savepoint ended.
    pub(crate) fn release(&self, depth: u32) {
        self.0.store(depth, Ordering::Release);
    }
}

/// A taken [`TxSlot`] level whose transaction or savepoint is still being
/// opened. Dropped, as when `BEGIN` failed or its future was cancelled, it
/// frees the level again.
pub(crate) struct Claim<'s> {
    slot: &'s TxSlot,
    depth: u32,
}

impl Claim<'_> {
    /// Hands the level to the opened transaction, which frees it when
    /// dropped.
    pub(crate) fn keep(self) {
        std::mem::forget(self);
    }
}

impl Drop for Claim<'_> {
    fn drop(&mut self) {
        self.slot.release(self.depth);
    }
}
