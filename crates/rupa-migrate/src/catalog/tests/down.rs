use rupa_core::ir::TableRef;
use rupa_core::schema::{ColumnDef, ColumnType};

use super::*;

const NOTES: TableRef = TableRef::new("notes");

/// A planned migration with a down plan.
struct Reversible(fn() -> Option<Plan>);

impl MigrationInfo for Reversible {
    fn info(&self) -> &'static Metadata {
        planless()
    }
}

impl PlannedMigration for Reversible {
    fn plan(&self) -> Plan {
        Plan::new().sql("CREATE TABLE notes (id INTEGER)")
    }

    fn down(&self) -> Option<Plan> {
        (self.0)()
    }
}

/// An imperative migration that declares a down body.
struct Declared(&'static Metadata);

impl MigrationInfo for Declared {
    fn info(&self) -> &'static Metadata {
        self.0
    }

    fn reversal(&self) -> Reversal {
        Reversal::Imperative
    }
}

fn planless() -> &'static Metadata {
    Box::leak(Box::new(Metadata {
        inputs: &[],
        ..*metadata("1", "")
    }))
}

#[test]
fn a_down_body_extends_the_checksum_and_its_absence_keeps_it() {
    let irreversible = digest(Planned(Reversible(|| None))).expect("irreversible");
    let without_down = digest(Planned(Fixed(planless(), || {
        Plan::new().sql("CREATE TABLE notes (id INTEGER)")
    })))
    .expect("plan only");
    let dropped = digest(Planned(Reversible(|| {
        Some(Plan::new().sql("DROP TABLE notes"))
    })))
    .expect("down plan");
    let renamed = digest(Planned(Reversible(|| {
        Some(Plan::new().rename_table(NOTES, "old_notes"))
    })))
    .expect("another down plan");
    let imperative = digest(Entry(metadata("1", "fn up() {}"))).expect("imperative");
    let declared = digest(Declared(metadata("1", "fn up() {}"))).expect("declared");

    assert_eq!(
        irreversible, without_down,
        "an artifact without a down body keeps the checksum it always had"
    );
    assert_ne!(dropped, irreversible);
    assert_ne!(dropped, renamed, "the down plan is part of the identity");
    assert_ne!(imperative, declared, "declaring a down body is a change");
}

#[test]
fn an_empty_down_plan_is_rejected() {
    assert!(matches!(
        digest(Planned(Reversible(|| Some(Plan::new())))),
        Err(ValidationError::EmptyPlan(version)) if version == "1"
    ));
}

#[test]
fn down_plans_are_validated_for_the_session_dialect() {
    let migrations = [Planned(Reversible(|| {
        Some(Plan::new().add_column_to(NOTES, ColumnDef::new("slug", ColumnType::Text)))
    }))];
    let catalog =
        Catalog::new("primary", VersionScheme::Numbered, &migrations).expect("valid catalog");

    assert!(
        catalog
            .check(DialectId::Postgres, &[Policy::TransactionalRequired])
            .is_ok()
    );
    assert!(
        matches!(
            catalog.check(DialectId::Sqlite, &[Policy::TransactionalRequired]),
            Err(ValidationError::Schema(_))
        ),
        "a down plan the dialect cannot render is refused before any target is touched"
    );
}
