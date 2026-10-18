//! The registry entry, for run-time driver selection.

use std::fmt;

use rupa_core::config::{DbConfig, DriverConfig};
use rupa_core::error::ConnectError;
use rupa_core::exec::BoxFuture;
use rupa_core::registry::{Database, Driver};

use crate::MemoryDb;

type SetupFn = dyn Fn(&mut MemoryDb) + Send + Sync;

/// The registry entry for the memory backend: name `memory`, scheme
/// `memory` (`memory:` is a complete URL).
///
/// Each `connect` creates a fresh, empty database served as a single shared
/// connection ([`Database::single`]); pool settings are ignored. Tables must
/// be registered before use, so give the driver a [`setup`](Self::setup)
/// function:
///
/// ```ignore
/// let registry = Registry::new().with(MemoryDriver::new().setup(|db| {
///     db.register::<User>();
/// }));
/// ```
#[derive(Clone, Default)]
pub struct MemoryDriver {
    setup: Option<std::sync::Arc<SetupFn>>,
}

impl MemoryDriver {
    /// A driver whose databases start without tables.
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs `f` on every new database, e.g. to register tables or seed rows.
    pub fn setup(mut self, f: impl Fn(&mut MemoryDb) + Send + Sync + 'static) -> Self {
        self.setup = Some(std::sync::Arc::new(f));
        self
    }
}

impl fmt::Debug for MemoryDriver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoryDriver")
            .field("setup", &self.setup.is_some())
            .finish()
    }
}

impl Driver for MemoryDriver {
    fn name(&self) -> &'static str {
        "memory"
    }

    fn dialect(&self) -> rupa_core::DialectId {
        rupa_core::DialectId::Memory
    }

    fn schemes(&self) -> &'static [&'static str] {
        &["memory"]
    }

    fn connect<'a>(
        &'a self,
        config: &'a DbConfig,
    ) -> BoxFuture<'a, Result<Database, ConnectError>> {
        let database = match &config.driver {
            DriverConfig::Auto | DriverConfig::Memory => {
                let mut db = MemoryDb::new();
                if let Some(setup) = &self.setup {
                    setup(&mut db);
                }

                Ok(Database::single(db.into_async()))
            }
            other => Err(ConnectError::InvalidOption {
                key: "driver".into(),
                message: format!(
                    "the memory driver was given options for `{}`",
                    other.name().unwrap_or("auto")
                ),
            }),
        };
        Box::pin(std::future::ready(database))
    }
}
