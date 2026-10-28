//! The per-connection transaction state drivers embed: the rollback a
//! dropped transaction owes, and whether the top-level transaction is open.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

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

/// Whether a connection's top-level transaction is open, for drivers to
/// embed and return from [`TxConnection::slot`](super::TxConnection::slot).
///
/// A connection runs one top-level transaction at a time: a second `BEGIN`
/// would commit (MySQL) or join (Postgres) the first, so opening one while
/// the slot is taken fails with [`TxError::AlreadyOpen`]. Savepoints do not
/// take it.
#[derive(Debug, Default)]
pub struct TxSlot(AtomicBool);

impl TxSlot {
    /// Whether a top-level transaction holds the slot.
    pub fn is_open(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    /// Takes the slot for a transaction at `depth`: only a top-level one
    /// (`depth == 0`) needs it, so a savepoint's claim holds nothing.
    pub(crate) fn claim(&self, depth: u32) -> Result<Claim<'_>, TxError> {
        if depth > 0 {
            return Ok(Claim(None));
        }

        if self.0.swap(true, Ordering::AcqRel) {
            return Err(TxError::AlreadyOpen);
        }

        Ok(Claim(Some(self)))
    }

    /// Frees the slot once the top-level transaction ended.
    pub(crate) fn release(&self) {
        self.0.store(false, Ordering::Release);
    }
}

/// A taken [`TxSlot`] (none for a savepoint) whose transaction is still
/// being opened. Dropped, as when `BEGIN` failed or its future was
/// cancelled, it frees the slot again.
pub(crate) struct Claim<'s>(Option<&'s TxSlot>);

impl Claim<'_> {
    /// Hands the slot to the opened transaction, which frees it when dropped.
    pub(crate) fn keep(self) {
        std::mem::forget(self);
    }
}

impl Drop for Claim<'_> {
    fn drop(&mut self) {
        if let Some(slot) = self.0 {
            slot.release();
        }
    }
}
