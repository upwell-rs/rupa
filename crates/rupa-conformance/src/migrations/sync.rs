//! The cases against `rupa::migrate::sync`.

use std::convert::Infallible;

use rupa::core::sync::Connection;
use rupa::migrate::Catalog;
use rupa::migrate::commands::{self, Command, Outcome};
use rupa::migrate::sync::{Adopt, Apply, Inspect, Migration, Recover, Revert, Session};

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

#[macro_use]
#[path = "cases/support.rs"]
mod support;
#[path = "cases/adoption.rs"]
mod adoption;
#[path = "cases/commands.rs"]
mod commands_cases;
#[path = "cases/control.rs"]
mod control;
#[path = "cases/exclusion.rs"]
mod exclusion;
#[path = "cases/history.rs"]
mod history;
#[path = "cases/inspection.rs"]
mod inspection;
#[path = "cases/nontransactional.rs"]
mod nontransactional;
#[path = "cases/ordering.rs"]
mod ordering;
#[path = "cases/recovery.rs"]
mod recovery;
#[path = "cases/reversal.rs"]
mod reversal;
#[path = "cases/schema.rs"]
mod schema;
#[path = "cases/snapshots.rs"]
mod snapshots;

/// Runs `command` through the operator commands, connecting to `h`.
fn operate<'c, H: Harness, M: Migration<H::Session>>(
    h: &H,
    catalog: &'c Catalog<'_, M>,
    command: &Command<'_>,
) -> Outcome<'c> {
    commands::sync::run(catalog, command, || Ok::<_, Infallible>(h.connect()))
}

/// Runs `command` through the operator commands against a target that
/// refuses connections.
fn unreachable<'c, H: Harness, M: Migration<H::Session>>(
    catalog: &'c Catalog<'_, M>,
    command: &Command<'_>,
) -> Outcome<'c> {
    commands::sync::run(catalog, command, || {
        Err::<H::Session, _>(std::io::Error::other("connection refused"))
    })
}

/// Runs every case through the sync contracts.
pub fn run<H: Harness + Sync>(h: &mut H) {
    all_cases!("sync", h);
}
