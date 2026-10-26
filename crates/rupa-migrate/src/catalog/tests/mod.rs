mod down;
mod plans;

use super::*;
use crate::{
    ArtifactInput, Metadata, MigrationInfo, MigrationKind, Planned, PlannedMigration, Policy,
};

#[derive(Debug)]
struct Entry(&'static Metadata);
impl MigrationInfo for Entry {
    fn info(&self) -> &'static Metadata {
        self.0
    }
}

/// A planned migration whose plan comes from a function.
struct Fixed(&'static Metadata, fn() -> Plan);

impl MigrationInfo for Fixed {
    fn info(&self) -> &'static Metadata {
        self.0
    }
}

impl PlannedMigration for Fixed {
    fn plan(&self) -> Plan {
        (self.1)()
    }
}

/// The checksum of the only entry of a one-migration catalog.
fn digest<M: MigrationInfo>(migration: M) -> Result<Checksum, ValidationError> {
    let migrations = [migration];
    let catalog = Catalog::new("primary", VersionScheme::Numbered, &migrations)?;

    Ok(catalog.entries[0].checksum)
}

fn metadata(version: &'static str, text: &'static str) -> &'static Metadata {
    Box::leak(Box::new(Metadata {
        version,
        description: "fixture",
        policy: Policy::TransactionalRequired,
        kind: MigrationKind::Incremental,
        inputs: Box::leak(vec![ArtifactInput::new("body.sql", text)].into_boxed_slice()),
    }))
}

#[test]
fn catalog_sorts_versions_and_rejects_duplicates() {
    let entries = [Entry(metadata("10", "a")), Entry(metadata("2", "b"))];
    let duplicate = [Entry(metadata("2", "a")), Entry(metadata("2", "b"))];

    let catalog =
        Catalog::new("primary", VersionScheme::Numbered, &entries).expect("valid catalog");

    assert_eq!(
        catalog
            .entries()
            .map(|e| e.version.to_string())
            .collect::<Vec<_>>(),
        ["2", "10"]
    );
    assert!(matches!(
        Catalog::new("primary", VersionScheme::Numbered, &duplicate),
        Err(ValidationError::Duplicate(_))
    ));
}

#[test]
fn checksum_normalizes_line_endings_but_detects_body_and_policy_changes() {
    let lf = Entry(metadata("1", "SELECT 1;\n"));
    let crlf = Entry(metadata("1", "\u{feff}SELECT 1;\r\n"));
    let edited = Entry(metadata("1", "SELECT 2;\n"));
    let changed_policy = Entry(Box::leak(Box::new(Metadata {
        policy: Policy::NonTransactional,
        ..*lf.0
    })));

    let digest = |entry| {
        Catalog::new(
            "primary",
            VersionScheme::Numbered,
            std::slice::from_ref(entry),
        )
        .expect("valid catalog")
        .entries()
        .next()
        .expect("one entry")
        .checksum
    };

    assert_eq!(digest(&lf), digest(&crlf));
    assert_ne!(digest(&lf), digest(&edited));
    assert_ne!(digest(&lf), digest(&changed_policy));
}

#[test]
fn checksum_rejects_ambiguous_inputs_and_does_not_hash_description() {
    let original = Entry(metadata("1", "SELECT 1;"));
    let renamed = Entry(Box::leak(Box::new(Metadata {
        description: "different description",
        ..*original.0
    })));
    let invalid = Entry(metadata("1", "SELECT\r1;"));
    let unsafe_path = Entry(Box::leak(Box::new(Metadata {
        inputs: Box::leak(vec![ArtifactInput::new("../helper.rs", "x")].into_boxed_slice()),
        ..*original.0
    })));

    let digest = |entry| {
        Catalog::new(
            "primary",
            VersionScheme::Numbered,
            std::slice::from_ref(entry),
        )
        .expect("valid catalog")
        .entries()
        .next()
        .expect("one entry")
        .checksum
    };

    assert_eq!(digest(&original), digest(&renamed));
    assert!(Catalog::new("primary", VersionScheme::Numbered, &[invalid]).is_err());
    assert!(Catalog::new("primary", VersionScheme::Numbered, &[unsafe_path]).is_err());
}

#[test]
fn planned_operation_order_and_adapter_selection_are_checksummed() {
    let info = metadata("1", "declared shared source");
    let forward = digest(Planned(Fixed(info, || {
        Plan::new().sql("SELECT 1").sql("SELECT 2")
    })));
    let reverse = digest(Planned(Fixed(info, || {
        Plan::new().sql("SELECT 2").sql("SELECT 1")
    })));
    let imperative = digest(Fixed(info, Plan::new));

    assert_ne!(
        forward.as_ref().expect("forward plan"),
        reverse.as_ref().expect("reverse plan"),
        "operation order changes executable work"
    );
    assert_ne!(
        forward.as_ref().expect("forward plan"),
        imperative.as_ref().expect("imperative entry"),
        "selecting a plan versus imperative execution changes the artifact"
    );
}

#[test]
fn checksum_separates_operation_records_from_input_records() {
    let base = metadata("1", "same");
    let one_info = Box::leak(Box::new(Metadata {
        inputs: Box::leak(
            vec![
                ArtifactInput::new("sql", "SELECT 2"),
                ArtifactInput::new("z.rs", "same"),
            ]
            .into_boxed_slice(),
        ),
        ..*base
    }));
    let two_info = Box::leak(Box::new(Metadata {
        inputs: Box::leak(vec![ArtifactInput::new("z.rs", "same")].into_boxed_slice()),
        ..*base
    }));

    let one = digest(Planned(Fixed(one_info, || Plan::new().sql("SELECT 1"))));
    let two = digest(Planned(Fixed(two_info, || {
        Plan::new().sql("SELECT 1").sql("SELECT 2")
    })));

    assert_ne!(
        one.expect("one operation"),
        two.expect("two operations"),
        "a source named sql must not alias another operation"
    );
}

crate::sql_migration! {
    /// Declares no policy, so it requires a transaction.
    struct DefaultPolicy {
        version: "1",
        description: "default policy",
        up: "body.sql",
    }
}

crate::sql_migration! {
    /// Declares nontransactional execution.
    struct DeclaredPolicy {
        version: "1",
        description: "declared policy",
        policy: NonTransactional,
        up: "body.sql",
    }
}

#[test]
fn sql_migrations_declare_their_policy_into_the_checksum() {
    let default = DefaultPolicy.info();
    let declared = DeclaredPolicy.info();

    assert_eq!(default.policy, Policy::TransactionalRequired);
    assert_eq!(declared.policy, Policy::NonTransactional);
    assert_ne!(
        digest(Planned(DefaultPolicy)).expect("default checksum"),
        digest(Planned(DeclaredPolicy)).expect("declared checksum")
    );
}
