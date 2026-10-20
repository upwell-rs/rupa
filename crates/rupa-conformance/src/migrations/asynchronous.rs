//! The same cases against `rupa::migrate::asynchronous`, from the same source.

use std::future::Future;

use rupa::core::exec::Blocking;
use rupa::migrate::asynchronous::{Apply, Session};

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

mod control {
    include!("cases/control.rs");
}
mod exclusion {
    include!("cases/exclusion.rs");
}
mod history {
    include!("cases/history.rs");
}
mod nontransactional {
    include!("cases/nontransactional.rs");
}
mod schema {
    include!("cases/schema.rs");
}

/// Runs every case through the async contracts.
pub fn run<H: Harness + Sync>(h: &mut H) {
    all_cases!("async", h);
}
