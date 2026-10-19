//! The cases against `rupa::migrate::sync`.

use rupa::migrate::sync::{Apply, Session};

pub use super::Target;
use super::{catalog, versions};

/// A migration target handing out independent dedicated sync connections.
pub trait Harness: Target {
    /// The driver's native migration connection.
    type Session: Session + Send;

    /// Opens an independent connection to the target. Lock waits should be
    /// short, since exclusion cases expect a busy competitor to give up.
    fn connect(&self) -> Self::Session;
}

/// A sync call is its own result.
macro_rules! wait {
    ($h:expr, $call:expr) => {
        $call
    };
}

#[path = "cases/control.rs"]
mod control;
#[path = "cases/exclusion.rs"]
mod exclusion;
#[path = "cases/history.rs"]
mod history;
#[path = "cases/schema.rs"]
mod schema;

/// Runs every case through the sync contracts.
pub fn run<H: Harness + Sync>(h: &mut H) {
    all_cases!("sync", h);
}
