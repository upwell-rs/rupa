use rupa::migrate::{
    ArtifactInput, Context, Error, Metadata, Migration, MigrationInfo, MigrationKind, Policy,
    Session,
};
use rupa::{raw, select};

/// The typed result read by this data migration.
#[derive(rupa::Entity)]
#[entity(table = "notes")]
struct Note {
    #[id]
    id: i64,
    title: String,
}

/// Compiled Rust migration that transforms typed query results between SQL steps.
#[derive(Debug)]
pub struct Normalize;

impl MigrationInfo for Normalize {
    fn info(&self) -> &'static Metadata {
        const INFO: Metadata = Metadata {
            version: "2",
            description: "normalize note titles in Rust",
            policy: Policy::TransactionalRequired,
            kind: MigrationKind::Incremental,
            inputs: &[ArtifactInput::new(
                "normalize.rs",
                include_str!("normalize.rs"),
            )],
        };

        &INFO
    }
}

impl<S: Session> Migration<S> for Normalize {
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
