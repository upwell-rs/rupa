//! The same cases against `rupa::migrate::asynchronous`, from the same source.

use std::convert::Infallible;
use std::future::Future;

use rupa::core::asynchronous::Connection;
use rupa::core::exec::Blocking;
use rupa::migrate::Catalog;
use rupa::migrate::asynchronous::{Adopt, Apply, Inspect, Migration, Recover, Revert, Session};
use rupa::migrate::commands::{self, Command, Outcome};

pub use super::Target;
use super::{catalog, versions};

/// A migration target handing out independent dedicated async connections.
pub trait Harness: Target {
    /// The driver's async migration connection.
    type Session: Session + Send;

    /// Opens an independent connection to the target, with short lock waits.
    fn connect(&self) -> Self::Session;

    /// Drives `future` to completion on the harness's executor.
    fn block_on<F: Future>(&self, future: F) -> F::Output;
}

/// A sync harness whose sessions run through `Blocking`, driven by pollster:
/// the async contracts on sync drivers, with no runtime.
pub struct Blocked<H>(pub H);

impl<H: super::sync::Harness> Target for Blocked<H> {
    fn reset(&mut self) {
        self.0.reset();
    }

    fn execute(&self, sql: &str) {
        self.0.execute(sql);
    }

    fn count(&self, sql: &str) -> i64 {
        self.0.count(sql)
    }

    fn table_exists(&self, name: &str) -> bool {
        self.0.table_exists(name)
    }

    fn forbidden_scripts(&self) -> &'static [&'static str] {
        self.0.forbidden_scripts()
    }

    fn outside_transaction_script(&self) -> &'static str {
        self.0.outside_transaction_script()
    }
}

impl<H: super::sync::Harness> Harness for Blocked<H> {
    type Session = Blocking<H::Session>;

    fn connect(&self) -> Blocking<H::Session> {
        Blocking(self.0.connect())
    }

    fn block_on<F: Future>(&self, future: F) -> F::Output {
        pollster::block_on(future)
    }
}

/// An async call is awaited on the harness's executor.
macro_rules! wait {
    ($h:expr, $call:expr) => {
        $h.block_on($call)
    };
}

#[macro_use]
mod support {
    include!("cases/support.rs");
}
mod adoption {
    include!("cases/adoption.rs");
}
mod commands_cases {
    include!("cases/commands.rs");
}
mod control {
    include!("cases/control.rs");
}
mod exclusion {
    include!("cases/exclusion.rs");
}
mod history {
    include!("cases/history.rs");
}
mod inspection {
    include!("cases/inspection.rs");
}
mod nontransactional {
    include!("cases/nontransactional.rs");
}
mod ordering {
    include!("cases/ordering.rs");
}
mod recovery {
    include!("cases/recovery.rs");
}
mod reversal {
    include!("cases/reversal.rs");
}
mod schema {
    include!("cases/schema.rs");
}
mod snapshots {
    include!("cases/snapshots.rs");
}

/// Runs `command` through the operator commands on a connection to `h`,
/// opened first: a harness may block on its own executor to connect.
fn operate<'c, H: Harness, M: Migration<H::Session> + Send + Sync>(
    h: &H,
    catalog: &'c Catalog<'_, M>,
    command: &Command<'_>,
) -> Outcome<'c> {
    let session = h.connect();

    h.block_on(commands::asynchronous::run(catalog, command, || {
        std::future::ready(Ok::<_, Infallible>(session))
    }))
}

/// Runs `command` through the operator commands against a target that
/// refuses connections.
fn unreachable<'c, H: Harness, M: Migration<H::Session> + Send + Sync>(
    catalog: &'c Catalog<'_, M>,
    command: &Command<'_>,
) -> Outcome<'c> {
    pollster::block_on(commands::asynchronous::run(catalog, command, || async {
        Err::<H::Session, _>(std::io::Error::other("connection refused"))
    }))
}

/// Runs every case through the async contracts.
pub fn run<H: Harness + Sync>(h: &mut H) {
    all_cases!("async", h);
}
