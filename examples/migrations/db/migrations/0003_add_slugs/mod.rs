//! Add a slug column derived from each title.

mod slug;

use rupa::migrate::Error;
use rupa::migrate::sync::{Context, Migration, Session};
use rupa::{raw, select};

/// The note shape as of version 3, frozen with this migration.
#[derive(rupa::Entity)]
#[entity(table = "notes")]
struct Note {
    #[id]
    id: i64,
    title: String,
}

/// Adds `notes.slug` and backfills it with a private helper.
#[derive(Debug)]
pub struct AddSlugs;

impl<S: Session> Migration<S> for AddSlugs {
    type Error = Error<S::Error>;

    fn up(&self, context: &mut Context<'_, S>) -> Result<(), Self::Error> {
        context.script("ALTER TABLE notes ADD COLUMN slug TEXT;")?;

        let notes = context.run(select::<Note>().all())?;

        for note in notes {
            context.run(
                raw()
                    .sql("UPDATE notes SET slug = ")
                    .bind(slug::slugify(&note.title))
                    .sql(" WHERE id = ")
                    .bind(note.id)
                    .execute::<u64>(),
            )?;
        }

        Ok(())
    }

    fn down(&self, context: &mut Context<'_, S>) -> Result<(), Self::Error> {
        context.script("ALTER TABLE notes DROP COLUMN slug;")
    }
}
