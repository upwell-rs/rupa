use super::*;
use crate::{ArtifactInput, Metadata, MigrationInfo, MigrationKind, Policy};

#[derive(Debug)]
struct Entry(&'static Metadata);
impl MigrationInfo for Entry {
    fn info(&self) -> &'static Metadata {
        self.0
    }
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
    use crate::{Operation, Planned, PlannedMigration};

    struct Forward(&'static Metadata);
    struct Reverse(&'static Metadata);
    impl MigrationInfo for Forward {
        fn info(&self) -> &'static Metadata {
            self.0
        }
    }
    impl MigrationInfo for Reverse {
        fn info(&self) -> &'static Metadata {
            self.0
        }
    }
    impl PlannedMigration for Forward {
        const UP: &'static [Operation] = &[Operation::Sql("SELECT 1"), Operation::Sql("SELECT 2")];
    }
    impl PlannedMigration for Reverse {
        const UP: &'static [Operation] = &[Operation::Sql("SELECT 2"), Operation::Sql("SELECT 1")];
    }
    let info = metadata("1", "declared shared source");
    let forward = [Planned(Forward(info))];
    let reverse = [Planned(Reverse(info))];
    let imperative = [Forward(info)];

    let forward = Catalog::new("primary", VersionScheme::Numbered, &forward).expect("forward plan");
    let reverse = Catalog::new("primary", VersionScheme::Numbered, &reverse).expect("reverse plan");
    let imperative =
        Catalog::new("primary", VersionScheme::Numbered, &imperative).expect("imperative entry");
    let digest = |catalog: &[CatalogEntry]| catalog[0].checksum;

    assert_ne!(
        digest(&forward.entries),
        digest(&reverse.entries),
        "operation order changes executable work"
    );
    assert_ne!(
        digest(&forward.entries),
        digest(&imperative.entries),
        "selecting a plan versus imperative execution changes the artifact"
    );
}

#[test]
fn checksum_separates_operation_records_from_input_records() {
    use crate::{Operation, Planned, PlannedMigration};

    struct One(&'static Metadata);
    struct Two(&'static Metadata);
    impl MigrationInfo for One {
        fn info(&self) -> &'static Metadata {
            self.0
        }
    }
    impl MigrationInfo for Two {
        fn info(&self) -> &'static Metadata {
            self.0
        }
    }
    impl PlannedMigration for One {
        const UP: &'static [Operation] = &[Operation::Sql("SELECT 1")];
    }
    impl PlannedMigration for Two {
        const UP: &'static [Operation] = &[Operation::Sql("SELECT 1"), Operation::Sql("SELECT 2")];
    }
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
    let one = [Planned(One(one_info))];
    let two = [Planned(Two(two_info))];

    let one = Catalog::new("primary", VersionScheme::Numbered, &one).expect("one operation");
    let two = Catalog::new("primary", VersionScheme::Numbered, &two).expect("two operations");

    assert_ne!(
        one.entries[0].checksum, two.entries[0].checksum,
        "a source named sql must not alias another operation"
    );
}
