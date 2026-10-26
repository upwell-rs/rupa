mod coverage;
mod revert;

use rupa_core::DialectId;

use super::*;
use crate::{
    ApplyOptions, ArtifactInput, Engine, Metadata, MigrationInfo, MigrationKind, Order, Plan,
    Planned, PlannedMigration, Provenance, Reversal, ValidationError, VersionScheme,
};

/// An imperative entry.
struct Imperative(&'static Metadata);

impl MigrationInfo for Imperative {
    fn info(&self) -> &'static Metadata {
        self.0
    }
}

/// A planned entry with one script.
struct Scripted(&'static Metadata);

impl MigrationInfo for Scripted {
    fn info(&self) -> &'static Metadata {
        self.0
    }
}

impl PlannedMigration for Scripted {
    fn plan(&self) -> Plan {
        Plan::new().sql("CREATE TABLE planned (id INTEGER)\r\n")
    }
}

/// A planned entry with one script and a down script.
struct Undone(&'static Metadata);

impl MigrationInfo for Undone {
    fn info(&self) -> &'static Metadata {
        self.0
    }
}

impl PlannedMigration for Undone {
    fn plan(&self) -> Plan {
        Plan::new().sql("CREATE TABLE planned (id INTEGER)")
    }

    fn down(&self) -> Option<Plan> {
        Some(Plan::new().sql("DROP TABLE planned"))
    }
}

/// An entry of any kind, reversible or not.
enum Entry {
    Imperative(Imperative),
    Planned(Planned<Scripted>),
    /// An imperative entry declaring a down body.
    Reversible(Imperative),
    Undone(Planned<Undone>),
    /// An imperative snapshot.
    Snapshot(Imperative),
}

impl MigrationInfo for Entry {
    fn info(&self) -> &'static Metadata {
        match self {
            Self::Imperative(entry) | Self::Reversible(entry) | Self::Snapshot(entry) => {
                entry.info()
            }
            Self::Planned(entry) => entry.info(),
            Self::Undone(entry) => entry.info(),
        }
    }

    fn planned(&self) -> Option<Plan> {
        match self {
            Self::Imperative(entry) | Self::Reversible(entry) | Self::Snapshot(entry) => {
                entry.planned()
            }
            Self::Planned(entry) => entry.planned(),
            Self::Undone(entry) => entry.planned(),
        }
    }

    fn reversal(&self) -> Reversal {
        match self {
            Self::Imperative(_) | Self::Planned(_) | Self::Snapshot(_) => Reversal::Irreversible,
            Self::Reversible(_) => Reversal::Imperative,
            Self::Undone(entry) => entry.reversal(),
        }
    }
}

fn metadata(version: &'static str, inputs: &'static [ArtifactInput]) -> &'static Metadata {
    Box::leak(Box::new(Metadata {
        version,
        description: "fixture",
        policy: Policy::TransactionalRequired,
        kind: MigrationKind::Incremental,
        inputs,
    }))
}

fn imperative(version: &'static str) -> Entry {
    const INPUTS: &[ArtifactInput] = &[ArtifactInput::new("body.rs", "fn up() {}")];

    Entry::Imperative(Imperative(metadata(version, INPUTS)))
}

fn planned(version: &'static str) -> Entry {
    Entry::Planned(Planned(Scripted(metadata(version, &[]))))
}

fn reversible(version: &'static str) -> Entry {
    const INPUTS: &[ArtifactInput] = &[ArtifactInput::new("body.rs", "fn down() {}")];

    Entry::Reversible(Imperative(metadata(version, INPUTS)))
}

fn undone(version: &'static str) -> Entry {
    Entry::Undone(Planned(Undone(metadata(version, &[]))))
}

fn snapshot(version: &'static str) -> Entry {
    const INPUTS: &[ArtifactInput] = &[ArtifactInput::new("snapshot.rs", "fn up() {}")];

    Entry::Snapshot(Imperative(Box::leak(Box::new(Metadata {
        kind: MigrationKind::Snapshot,
        ..*metadata(version, INPUTS)
    }))))
}

fn version(text: &str) -> Version {
    VersionScheme::Numbered.parse(text).expect("version")
}

fn catalog(entries: &[Entry]) -> Catalog<'_, Entry> {
    Catalog::new("primary", VersionScheme::Numbered, entries).expect("valid catalog")
}

/// History recording `version` with the checksum `catalog` holds for it.
fn installed(catalog: &Catalog<'_, Entry>, version: &str, rank: u64) -> HistoryEntry {
    let checksum = catalog
        .entries()
        .find(|entry| entry.version.to_string() == version)
        .map_or(Checksum::from_bytes([0; 32]), |entry| entry.checksum);

    HistoryEntry {
        installation_rank: rank,
        version: version.into(),
        checksum,
        provenance: Provenance::Executed,
    }
}

fn managed(history: Vec<HistoryEntry>, attempts: Vec<Attempt>) -> Journal {
    journal(history, None, attempts)
}

fn journal(
    history: Vec<HistoryEntry>,
    coverage: Option<Coverage>,
    attempts: Vec<Attempt>,
) -> Journal {
    Journal::Managed(Records {
        stream: "primary".into(),
        scheme: "numbered".into(),
        history,
        coverage,
        attempts,
    })
}

fn inspection(exclusion: Exclusion, journal: Journal) -> Inspection {
    Inspection { exclusion, journal }
}

fn attempt(id: i64, policy: Policy) -> Attempt {
    Attempt {
        id,
        version: "2".into(),
        checksum: Checksum::from_bytes([7; 32]),
        policy,
        kind: MigrationKind::Incremental,
        direction: Direction::Up,
    }
}

fn pending(status: &Status<'_>) -> Vec<String> {
    status
        .pending
        .iter()
        .map(|entry| entry.version.to_string())
        .collect()
}

#[test]
fn a_fresh_target_has_every_entry_pending() {
    let entries = [imperative("2"), planned("1")];
    let catalog = catalog(&entries);

    let status = catalog.classify(inspection(Exclusion::Free, Journal::Fresh));

    assert_eq!(status.classification, Classification::Fresh);
    assert_eq!(pending(&status), ["1", "2"]);
    assert!(status.blockers.is_empty());
}

#[test]
fn an_unmanaged_target_is_blocked_and_has_nothing_pending() {
    let entries = [imperative("1")];
    let catalog = catalog(&entries);

    let status = catalog.classify(inspection(Exclusion::Free, Journal::Unmanaged));

    assert_eq!(status.classification, Classification::Unmanaged);
    assert!(status.pending.is_empty());
    assert_eq!(status.blockers, [Blocker::Unmanaged]);
}

#[test]
fn installed_history_leaves_the_rest_pending() {
    let entries = [imperative("1"), planned("2")];
    let catalog = catalog(&entries);
    let history = vec![installed(&catalog, "1", 1)];

    let status = catalog.classify(inspection(Exclusion::Free, managed(history, Vec::new())));

    assert_eq!(status.classification, Classification::Managed);
    assert_eq!(pending(&status), ["2"]);
    assert_eq!(status.history.len(), 1);
    assert!(status.blockers.is_empty());
}

#[test]
fn missing_changed_duplicate_and_late_artifacts_are_each_reported() {
    let entries = [imperative("1"), imperative("2"), imperative("3")];
    let catalog = catalog(&entries);
    let changed = HistoryEntry {
        checksum: Checksum::from_bytes([1; 32]),
        ..installed(&catalog, "3", 3)
    };
    let changed_duplicate = HistoryEntry {
        checksum: Checksum::from_bytes([2; 32]),
        ..installed(&catalog, "1", 4)
    };
    let history = vec![
        installed(&catalog, "9", 1),
        installed(&catalog, "1", 2),
        changed_duplicate,
        changed,
    ];
    let catalog_checksum = |version: &str| {
        catalog
            .entries()
            .find(|entry| entry.version.to_string() == version)
            .expect("catalog entry")
            .checksum
    };

    let status = catalog.classify(inspection(Exclusion::Free, managed(history, Vec::new())));

    assert_eq!(
        status.blockers,
        [
            Blocker::Missing {
                version: "9".into(),
                checksum: Checksum::from_bytes([0; 32]),
            },
            Blocker::Duplicate(version("1")),
            Blocker::Changed {
                version: version("1"),
                installed: Checksum::from_bytes([2; 32]),
                catalog: catalog_checksum("1"),
            },
            Blocker::Changed {
                version: version("3"),
                installed: Checksum::from_bytes([1; 32]),
                catalog: catalog_checksum("3"),
            },
            Blocker::Late {
                version: version("2"),
                checksum: catalog_checksum("2"),
            },
        ]
    );
    assert_eq!(pending(&status), ["2"], "a late entry is still pending");
}

#[test]
fn running_attempts_make_the_target_uncertain() {
    let entries = [imperative("1"), imperative("2")];
    let catalog = catalog(&entries);
    let attempts = vec![
        attempt(4, Policy::TransactionalRequired),
        attempt(5, Policy::NonTransactional),
    ];

    let free = catalog.classify(inspection(
        Exclusion::Free,
        managed(Vec::new(), attempts.clone()),
    ));
    let locked = catalog.classify(inspection(Exclusion::Locked, managed(Vec::new(), attempts)));

    assert_eq!(free.classification, Classification::Uncertain);
    assert_eq!(
        free.blockers,
        [
            Blocker::Interrupted(attempt(4, Policy::TransactionalRequired)),
            Blocker::Unresolved(attempt(5, Policy::NonTransactional)),
        ]
    );
    assert_eq!(
        locked.blockers,
        [Blocker::Locked],
        "a live runner's attempts may be in progress"
    );
    assert_eq!(locked.attempts.len(), 2);
}

#[test]
fn an_owner_claim_makes_the_target_uncertain() {
    let entries = [imperative("1")];
    let catalog = catalog(&entries);

    let status = catalog.classify(inspection(
        Exclusion::Claimed,
        managed(Vec::new(), Vec::new()),
    ));

    assert_eq!(status.classification, Classification::Uncertain);
    assert_eq!(status.blockers, [Blocker::Claimed]);
    assert_eq!(pending(&status), ["1"]);
}

#[test]
fn another_stream_is_not_compared() {
    let entries = [imperative("1")];
    let catalog = catalog(&entries);
    let journal = Journal::Managed(Records {
        stream: "other".into(),
        scheme: "numbered".into(),
        history: vec![installed(&catalog, "7", 1)],
        coverage: None,
        attempts: Vec::new(),
    });

    let status = catalog.classify(inspection(Exclusion::Free, journal));

    assert_eq!(
        status.blockers,
        [Blocker::Stream {
            stream: "other".into(),
            scheme: "numbered".into(),
        }]
    );
    assert!(status.pending.is_empty());
}

#[test]
fn a_plan_renders_planned_bodies_and_keeps_imperative_ones_opaque() {
    let entries = [planned("1"), imperative("2")];
    let catalog = catalog(&entries);
    let status = catalog.classify(inspection(Exclusion::Free, Journal::Fresh));

    let plan = catalog
        .plan_for(status, DialectId::Postgres, &ApplyOptions::DEFAULT)
        .expect("renderable plan");
    let steps: Vec<_> = plan
        .steps
        .iter()
        .map(|step| (step.entry.version.to_string(), &step.execution))
        .collect();

    assert_eq!(
        steps,
        [
            (
                "1".to_owned(),
                &Execution::Static(vec!["CREATE TABLE planned (id INTEGER)\n".into()])
            ),
            ("2".to_owned(), &Execution::Opaque),
        ]
    );
}

#[test]
fn late_permission_plans_late_entries_and_keeps_every_other_blocker() {
    let entries = [imperative("1"), imperative("2"), imperative("3")];
    let catalog = catalog(&entries);
    let changed = HistoryEntry {
        checksum: Checksum::from_bytes([1; 32]),
        ..installed(&catalog, "1", 1)
    };
    let history = vec![changed, installed(&catalog, "3", 2)];
    let allow_late = ApplyOptions {
        order: Order::AllowLate,
        ..ApplyOptions::DEFAULT
    };
    let plan = |options: &ApplyOptions| {
        let journal = managed(history.clone(), Vec::new());
        let status = catalog.classify(inspection(Exclusion::Free, journal));

        catalog
            .plan_for(status, DialectId::Postgres, options)
            .expect("renderable plan")
    };

    let strict = plan(&ApplyOptions::DEFAULT);
    let permitted = plan(&allow_late);

    assert!(strict.late.is_empty());
    assert!(matches!(
        strict.status.blockers.as_slice(),
        [Blocker::Changed { .. }, Blocker::Late { version: late, .. }] if *late == version("2")
    ));
    assert_eq!(permitted.late, [version("2")]);
    assert!(
        matches!(
            permitted.status.blockers.as_slice(),
            [Blocker::Changed { .. }]
        ),
        "permission lifts only late blockers: {:?}",
        permitted.status.blockers
    );
    assert_eq!(
        permitted
            .steps
            .iter()
            .map(|step| step.entry.version)
            .collect::<Vec<_>>(),
        [version("2")]
    );
}

#[test]
fn validation_checks_the_engine_offline() {
    let entries = [planned("1")];
    let supported = catalog(&entries);
    let nontransactional = Box::leak(Box::new(Metadata {
        policy: Policy::NonTransactional,
        ..*metadata("1", &[])
    }));
    let unsupported = [Entry::Planned(Planned(Scripted(nontransactional)))];
    let unsupported = catalog(&unsupported);

    assert!(supported.validate(Engine::Sqlite).is_ok());
    assert!(matches!(
        unsupported.validate(Engine::Sqlite),
        Err(ValidationError::Unsupported(version)) if version == "1"
    ));
    assert!(unsupported.validate(Engine::Postgres).is_ok());
}
