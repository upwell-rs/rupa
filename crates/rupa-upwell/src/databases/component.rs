use std::any::TypeId;
use std::collections::HashSet;
use std::fmt;
use std::ops::Deref;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use rupa_core::config::DbConfig;
use rupa_migrate::ApplyOptions;
use rupa_migrate::commands::{Command, Exit, Format, Redactor, Report};
use upwell::{Cfg, Component};

use super::{DatabaseError, DatabaseSettings, Declaration, Mode, Settings, Startup};

/// The targets this process already applied migrations to.
static APPLIED: OnceLock<Mutex<HashSet<(TypeId, String)>>> = OnceLock::new();

/// Database `D`'s pool, as a singleton component.
///
/// Its factory runs `D`'s startup migration policy before it hands the pool
/// out, so every component that injects `Arc<Database<D>>`, or a repository
/// it provides, is built after the schema was checked or applied. A failed
/// check stops startup before any service or listener starts.
pub struct Database<D: Declaration> {
    pool: D::Pool,
}

impl<D: Declaration> Database<D> {
    /// The pool.
    pub const fn pool(&self) -> &D::Pool {
        &self.pool
    }
}

impl<D: Declaration> Deref for Database<D> {
    type Target = D::Pool;

    fn deref(&self) -> &D::Pool {
        &self.pool
    }
}

impl<D: Declaration> fmt::Debug for Database<D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Database")
            .field("name", &D::NAME)
            .finish_non_exhaustive()
    }
}

impl<D: Declaration> Component for Database<D> {
    type Handle = Arc<Self>;

    const ID: &'static str = D::ID;
    const NAME: &'static str = D::NAME;

    fn into_handle(self) -> Arc<Self> {
        Arc::new(self)
    }
}

/// Builds database `D`: runs its startup policy on the migration
/// connection, then opens the runtime pool.
pub(super) async fn build<D: Declaration>(
    settings: Cfg<Settings<D>>,
) -> Result<Database<D>, DatabaseError> {
    let settings = settings.snapshot();
    let runtime = settings.runtime()?;

    gate::<D>(&settings).await?;

    let pool = D::Mode::pool(&runtime).await?;

    Ok(Database { pool })
}

/// Checks or applies `D`'s migrations as `settings` say. Apply runs once
/// per target and process; later builds, such as after a configuration
/// reload, only check. A failed or cancelled apply leaves the next build to
/// apply again.
async fn gate<D: Declaration>(settings: &DatabaseSettings) -> Result<(), DatabaseError> {
    let startup = settings.startup()?;

    if startup == Startup::Off {
        return Ok(());
    }

    let config = settings.migration()?;
    let catalog = D::catalog()?;
    let claim = match startup {
        Startup::Apply => ApplyClaim::take::<D>(&config),
        Startup::Off | Startup::Check => None,
    };
    let command = match claim {
        Some(_) => Command::Apply(ApplyOptions::DEFAULT),
        None => Command::Status,
    };
    let outcome = D::Mode::run(&catalog, &command, &config).await;
    let report = || outcome.render(Format::Human, &Redactor::new().url(&config.url));
    let pending = match &outcome.report {
        Some(Report::Status(status)) => status.pending.len(),
        _ => 0,
    };

    match outcome.exit() {
        Exit::Success if pending == 0 => {
            if let Some(claim) = claim {
                claim.keep();
            }

            Ok(())
        }
        Exit::Success => Err(DatabaseError::Pending {
            database: D::NAME,
            pending,
            report: report(),
        }),
        exit => Err(DatabaseError::Migrations {
            database: D::NAME,
            exit,
            report: report(),
        }),
    }
}

/// The right to apply migrations to one target for one declaration, taken
/// at most once per process. Dropped before [`keep`](Self::keep), as when the
/// apply failed or was cancelled, it lets a later build apply again.
struct ApplyClaim {
    key: Option<(TypeId, String)>,
}

impl ApplyClaim {
    /// The claim on `config`'s target for `D`, unless this process already
    /// holds or kept one.
    fn take<D: Declaration>(config: &DbConfig) -> Option<Self> {
        let key = (TypeId::of::<D>(), config.url.clone());
        let taken = applied().insert(key.clone());

        taken.then_some(Self { key: Some(key) })
    }

    /// Records the target as applied for the rest of the process.
    fn keep(mut self) {
        self.key = None;
    }
}

impl Drop for ApplyClaim {
    fn drop(&mut self) {
        if let Some(key) = &self.key {
            applied().remove(key);
        }
    }
}

/// The targets this process applied migrations to, or is applying them to.
/// A poisoned set is still a valid set, so poisoning is ignored.
fn applied() -> MutexGuard<'static, HashSet<(TypeId, String)>> {
    APPLIED
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}
