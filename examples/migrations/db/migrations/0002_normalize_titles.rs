//! Trim and uppercase note titles in Rust.

use rupa::migrate::{Context, Error, Migration, Session};
use rupa::{raw, select};

/// The note shape as of version 2, frozen with this migration.
#[derive(rupa::Entity)]
#[entity(table = "notes")]
struct Note {
    #[id]
    id: i64,
    title: String,
}

/// Normalizes every note title.
#[derive(Debug)]
pub struct NormalizeTitles;

impl<S: Session> Migration<S> for NormalizeTitles {
    type Error = Error<S::Error>;

    fn up(&self, context: &mut Context<'_, S>) -> Result<(), Self::Error> {
        let notes = context.run(select::<Note>().all())?;

        for note in notes {
            let title = note.title.trim().to_uppercase();

            context.run(
                raw()
                    .sql("UPDATE notes SET title = ")
                    .bind(title)
                    .sql(" WHERE id = ")
                    .bind(note.id)
                    .execute::<u64>(),
            )?;
        }

        Ok(())
    }
}
