//! An Upwell application with two declared databases over the `db` crate's
//! migration streams.
//!
//! - `Primary` (numbered stream) is configured field by field and applies its
//!   migrations at startup. `NoteRepository` declares itself over it, so
//!   `Arc<dyn NoteRepository>` is injectable like any component.
//! - `Reporting` (timestamped stream) is configured with a templated `url`
//!   and only checks its migrations at startup.
//!
//! Both register themselves like Upwell components; the one `Migrations`
//! plugin adds their migration commands, `migrate` and `reporting`.

use std::sync::Arc;

use db::Note;
use rupa::DynError;
use rupa::drivers::sqlite::SqlitePool;
use rupa::prelude::*;
use rupa::upwell::{Database, Migrations};
use upwell::{app, component};

/// The notes of the primary database.
#[repository(database = Primary)]
pub trait NoteRepository: Send + Sync {
    /// Adds `note`.
    #[query(insert)]
    fn add(&self, note: &Note) -> Result<u64, DynError>;

    /// Every note, oldest first.
    #[query(order_by = id)]
    fn all(&self) -> Result<Vec<Note>, DynError>;

    /// The note with `title`, if any.
    #[query(filter = title == $title)]
    fn titled(&self, title: &str) -> Result<Option<Note>, DynError>;
}

rupa::upwell::database! {
    /// Notes and tags: the numbered `primary` stream.
    pub Primary {
        pool: SqlitePool,
        config: "database.primary",
        migrations: db::migrations,
        command: "migrate",
    }
}

rupa::upwell::database! {
    /// Usage events: the timestamped `reporting` stream.
    pub Reporting {
        pool: SqlitePool,
        config: "database.reporting",
        migrations: db::reporting,
        command: "reporting",
    }
}

/// An application service over the notes; built only after `Primary`'s
/// migrations are checked or applied.
#[component]
pub struct Notes {
    repository: Arc<dyn NoteRepository>,
    database: Arc<Database<Primary>>,
}

impl Notes {
    /// How many notes exist.
    pub fn count(&self) -> Result<usize, DynError> {
        Ok(self.repository.all()?.len())
    }

    /// Adds every note in one transaction: all of them or none.
    pub fn import(&self, notes: &[Note]) -> Result<(), DynError> {
        let tx = self.database.begin()?;

        for note in notes {
            self.repository.tx(&tx).add(note)?;
        }

        tx.commit()?;

        Ok(())
    }
}

app! {
    /// The example application. Its databases register themselves; the
    /// `Migrations` plugin adds their commands.
    pub app Example {
        name: "rupa-example-upwell",
        protocol: (),
        plugins: [Migrations],
    }
}
