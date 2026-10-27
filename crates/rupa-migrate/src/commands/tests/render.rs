use std::io;

use serde_json::Value;

use super::*;
use crate::{
    AdoptReport, ApplyReport, Checksum, Error, Exclusion, HistoryEntry, Inspection, Journal,
    Provenance, Records, RunError,
};

/// A connection string whose password a driver error might echo.
const URL: &str = "postgres://app:s3cret@db/app";

fn json(outcome: &Outcome<'_>, redactor: &Redactor) -> Value {
    serde_json::from_str(&outcome.render(Format::Json, redactor)).expect("one JSON document")
}

#[test]
fn exit_codes_are_stable() {
    let exits = [
        Exit::Success,
        Exit::Failed,
        Exit::Usage,
        Exit::Blocked,
        Exit::Busy,
        Exit::Uncertain,
        Exit::Reconciled,
        Exit::Migration,
    ];

    assert_eq!(exits.map(Exit::code), [0, 1, 2, 3, 4, 5, 6, 7]);
    assert_eq!(Exit::of(&Error::<io::Error>::Busy), Exit::Busy);
    assert_eq!(Exit::of(&Error::<io::Error>::Uncertain(7)), Exit::Uncertain);
    assert_eq!(
        Exit::of(&Error::<io::Error>::Reconciled(vec![7])),
        Exit::Reconciled
    );
    assert_eq!(Exit::of(&Error::<io::Error>::Unmanaged), Exit::Blocked);
    assert_eq!(
        Exit::of(&Error::<io::Error>::MissingDown(version("1"))),
        Exit::Migration
    );
    assert_eq!(
        Exit::of(&Error::Database(io::Error::other("lost"))),
        Exit::Failed
    );
}

#[test]
fn a_fresh_status_lists_pending_work_with_its_sources() {
    let entries = NUMBERED;
    let numbered = catalog(&entries);
    let status = numbered.classify(Inspection {
        exclusion: Exclusion::Free,
        journal: Journal::Fresh,
    });
    let outcome = Outcome::new("status", &numbered).report(Report::Status(status));
    let human = outcome.render(Format::Human, &Redactor::new());
    let json = json(&outcome, &Redactor::new());

    assert_eq!(outcome.exit(), Exit::Success, "pending work blocks nothing");
    assert!(
        human.starts_with("status: stream primary (numbered)\n"),
        "{human}"
    );
    assert!(
        human.contains("  target: fresh, never initialized, exclusion free\n"),
        "{human}"
    );
    assert!(
        human.contains("    1 (transactional) [0001_create_notes.sql]\n"),
        "{human}"
    );
    assert!(human.ends_with("result: success (exit 0)\n"), "{human}");
    assert_eq!(json["format"], REPORT_FORMAT);
    assert_eq!(json["exit"]["code"], 0);
    assert_eq!(json["report"]["classification"], "fresh");
    assert_eq!(json["report"]["pending"][1]["version"], "2");
    assert_eq!(json["report"]["pending"][1]["source"], "0002_add_slugs.sql");
    assert_eq!(json["failure"], Value::Null);
}

#[test]
fn a_status_with_blockers_exits_blocked_and_names_them() {
    let entries = NUMBERED;
    let numbered = catalog(&entries);
    let status = numbered.classify(Inspection {
        exclusion: Exclusion::Free,
        journal: Journal::Managed(Records {
            stream: "primary".into(),
            scheme: "numbered".into(),
            history: vec![HistoryEntry {
                installation_rank: 1,
                version: "3".into(),
                checksum: Checksum::from_bytes([7; 32]),
                provenance: Provenance::Executed,
            }],
            coverage: None,
            initialized: true,
            attempts: Vec::new(),
        }),
    });
    let outcome = Outcome::new("status", &numbered).report(Report::Status(status));
    let human = outcome.render(Format::Human, &Redactor::new());
    let json = json(&outcome, &Redactor::new());

    assert_eq!(outcome.exit(), Exit::Blocked);
    assert!(
        human.contains("    3 (070707070707) is recorded but missing from the catalog\n"),
        "{human}"
    );
    assert_eq!(
        json["exit"],
        serde_json::json!({ "code": 3, "status": "blocked" })
    );
    assert_eq!(json["report"]["blockers"][0]["kind"], "missing");
    assert_eq!(json["report"]["blockers"][0]["version"], "3");
}

#[test]
fn a_failed_body_reports_its_commits_source_and_causes_without_secrets() {
    let entries = NUMBERED;
    let numbered = catalog(&entries);
    let committed = ApplyReport {
        applied: vec![version("1")],
        ..ApplyReport::default()
    };
    let failed: Result<ApplyReport, RunError<io::Error, io::Error>> = Err(RunError::Migration {
        version: version("2"),
        source: io::Error::other(format!("server at {URL} refused: password s3cret")),
        report: Box::new(committed),
        cleanup: Vec::new(),
    });
    let outcome = Outcome::new("apply", &numbered).ran(&numbered, failed, Report::Applied);
    let redactor = Redactor::new().url(URL);
    let human = outcome.render(Format::Human, &redactor);
    let json = json(&outcome, &redactor);

    assert_eq!(outcome.exit(), Exit::Migration);
    assert!(human.contains("  applied: 1\n"), "{human}");
    assert!(
        human.contains("error: migration 2 failed: server at postgres://app:***@db/app"),
        "{human}"
    );
    assert!(human.contains("  source: 0002_add_slugs.sql\n"), "{human}");
    assert!(!human.contains("s3cret") && !json.to_string().contains("s3cret"));
    assert_eq!(json["report"]["applied"], serde_json::json!(["1"]));
    assert_eq!(json["failure"]["status"], "migration");
    assert_eq!(json["failure"]["migration"], "2");
    assert_eq!(json["failure"]["source"], "0002_add_slugs.sql");
}

#[test]
fn a_refused_run_carries_its_blockers_and_causes() {
    let entries = NUMBERED;
    let numbered = catalog(&entries);
    let refused: Result<ApplyReport, RunError<io::Error, io::Error>> =
        Err(RunError::Infrastructure {
            source: Error::Blocked(vec![crate::Blocker::Duplicate(version("1"))]),
            report: Box::default(),
            cleanup: vec![Error::Database(io::Error::other("release failed"))],
        });
    let outcome = Outcome::new("apply", &numbered).ran(&numbered, refused, Report::Applied);
    let human = outcome.render(Format::Human, &Redactor::new());
    let json = json(&outcome, &Redactor::new());

    assert_eq!(outcome.exit(), Exit::Blocked);
    assert!(
        human.contains("    1 is recorded more than once\n"),
        "{human}"
    );
    assert!(
        human.contains("  cleanup failed: migration database operation failed: release failed\n")
    );
    assert_eq!(json["failure"]["blockers"][0]["kind"], "duplicate");
    assert_eq!(json["failure"]["cleanup"].as_array().map(Vec::len), Some(1));
}

#[test]
fn assertions_say_that_nothing_verified_them() {
    let entries = NUMBERED;
    let numbered = catalog(&entries);
    let outcome = Outcome::new("adopt", &numbered).report(Report::Adopted {
        report: AdoptReport {
            adopted: vec![version("1"), version("2")],
        },
        through: version("2"),
        reason: "restored from backup".into(),
        evidence: Some("TICKET-7".into()),
    });
    let human = outcome.render(Format::Human, &Redactor::new());
    let json = json(&outcome, &Redactor::new());

    assert!(human.contains("nothing verified it"), "{human}");
    assert!(
        human.contains("  reason: restored from backup\n  evidence: TICKET-7\n"),
        "{human}"
    );
    assert_eq!(json["report"]["verified"], false);
    assert_eq!(json["report"]["adopted"], serde_json::json!(["1", "2"]));
}
