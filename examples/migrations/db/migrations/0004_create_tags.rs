//! Create the tags table from its entity and index note slugs.

use rupa::Entity;
use rupa::migrate::schema::{Index, Table};
use rupa::migrate::{Plan, PlannedMigration};

use crate::{Note, Tag};

/// A planned migration: plain data, rendered for the session's dialect.
#[derive(Debug)]
pub struct CreateTags;

impl PlannedMigration for CreateTags {
    fn plan(&self) -> Plan {
        Plan::new()
            .create_table(Table::of::<Tag>())
            .create_index(Index::on("notes_slug", Note::SLUG).unique())
    }

    fn down(&self) -> Option<Plan> {
        Some(
            Plan::new()
                .drop_index(Note::TABLE, "notes_slug")
                .drop_table(Tag::TABLE),
        )
    }
}
