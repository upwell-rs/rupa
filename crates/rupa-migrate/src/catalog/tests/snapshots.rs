use super::*;

/// Snapshot metadata at `version` with body `text`.
fn snapshot(version: &'static str, text: &'static str) -> Entry {
    Entry(Box::leak(Box::new(Metadata {
        kind: MigrationKind::Snapshot,
        ..*metadata(version, text)
    })))
}

/// A planned snapshot at version 1 that declares a down plan.
struct Undoable;

impl MigrationInfo for Undoable {
    fn info(&self) -> &'static Metadata {
        Box::leak(Box::new(Metadata {
            kind: MigrationKind::Snapshot,
            inputs: &[],
            ..*metadata("1", "")
        }))
    }
}

impl PlannedMigration for Undoable {
    fn plan(&self) -> Plan {
        Plan::new().sql("CREATE TABLE a (id INTEGER)")
    }

    fn down(&self) -> Option<Plan> {
        Some(Plan::new().sql("DROP TABLE a"))
    }
}

/// The checksums of the incrementals and the snapshot of `entries`, in
/// catalog order, and the versions the snapshot covers.
fn checksums(entries: &[Entry]) -> (Vec<Checksum>, Checksum, Vec<String>) {
    let catalog = Catalog::new("primary", VersionScheme::Numbered, entries).expect("valid catalog");
    let snapshot = catalog.snapshots().next().expect("one snapshot");
    let covers = snapshot
        .covers
        .iter()
        .map(|covered| covered.version.to_string())
        .collect();

    (
        catalog.incrementals().map(|e| e.checksum).collect(),
        snapshot.checksum,
        covers,
    )
}

#[test]
fn a_snapshot_covers_every_incremental_at_or_below_its_version() {
    let entries = [
        Entry(metadata("1", "a")),
        Entry(metadata("2", "b")),
        Entry(metadata("3", "c")),
        snapshot("2", "ab"),
    ];
    let catalog = Catalog::new("primary", VersionScheme::Numbered, &entries).expect("valid");

    let order: Vec<_> = catalog
        .entries()
        .map(|entry| (entry.version.to_string(), entry.metadata.kind))
        .collect();
    let snapshot = catalog.snapshots().next().expect("snapshot");
    let covered: Vec<_> = catalog
        .incrementals()
        .take(2)
        .map(|e| Covered {
            version: e.version,
            checksum: e.checksum,
        })
        .collect();

    assert_eq!(
        order,
        [
            ("1".to_owned(), MigrationKind::Incremental),
            ("2".to_owned(), MigrationKind::Incremental),
            ("2".to_owned(), MigrationKind::Snapshot),
            ("3".to_owned(), MigrationKind::Incremental),
        ],
        "a snapshot shares its version with an incremental and sorts after it"
    );
    assert_eq!(snapshot.covers, covered);
}

#[test]
fn the_manifest_is_part_of_the_snapshot_checksum_only() {
    let base = [
        Entry(metadata("1", "a")),
        Entry(metadata("3", "c")),
        snapshot("3", "ac"),
    ];
    let edited = [
        Entry(metadata("1", "edited")),
        Entry(metadata("3", "c")),
        snapshot("3", "ac"),
    ];
    let added_below = [
        Entry(metadata("1", "a")),
        Entry(metadata("2", "b")),
        Entry(metadata("3", "c")),
        snapshot("3", "ac"),
    ];
    let added_above = [
        Entry(metadata("1", "a")),
        Entry(metadata("3", "c")),
        Entry(metadata("4", "d")),
        snapshot("3", "ac"),
    ];
    let without = [Entry(metadata("1", "a")), Entry(metadata("3", "c"))];
    let catalog = Catalog::new("primary", VersionScheme::Numbered, &without).expect("valid");

    let (incrementals, base, covers) = checksums(&base);
    let (_, edited, _) = checksums(&edited);
    let (_, added_below, added_covers) = checksums(&added_below);
    let (_, added_above, _) = checksums(&added_above);

    assert_eq!(covers, ["1", "3"]);
    assert_eq!(added_covers, ["1", "2", "3"]);
    assert_ne!(
        base, edited,
        "editing a covered incremental changes the snapshot"
    );
    assert_ne!(
        base, added_below,
        "adding one below the boundary changes it"
    );
    assert_eq!(base, added_above, "adding one above the boundary does not");
    assert_eq!(
        incrementals,
        catalog
            .incrementals()
            .map(|e| e.checksum)
            .collect::<Vec<_>>(),
        "a snapshot never changes the checksum of an incremental"
    );
}

#[test]
fn snapshots_must_cover_something_be_irreversible_and_unique_per_version() {
    let empty = [Entry(metadata("2", "b")), snapshot("1", "a")];
    let reversible = [Planned(Undoable)];
    let duplicate = [
        Entry(metadata("1", "a")),
        snapshot("1", "a"),
        snapshot("1", "b"),
    ];

    assert!(matches!(
        Catalog::new("primary", VersionScheme::Numbered, &empty),
        Err(ValidationError::EmptySnapshot(version)) if version.to_string() == "1"
    ));
    assert!(matches!(
        Catalog::new("primary", VersionScheme::Numbered, &reversible),
        Err(ValidationError::ReversibleSnapshot(_))
    ));
    assert!(matches!(
        Catalog::new("primary", VersionScheme::Numbered, &duplicate),
        Err(ValidationError::Duplicate(_))
    ));
}
