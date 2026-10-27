//! The machine contract: one JSON document per outcome, tagged with
//! [`REPORT_FORMAT`]. Fields are only ever added within a format version.

use serde_json::{Value, json};

use super::{Failure, Outcome, Report, human};
use crate::{
    Attempt, Blocker, CatalogEntry, Classification, Coverage, CoverageSource, Direction, Exclusion,
    Execution, HistoryEntry, MigrationKind, RecoveryAction, Reversal, Status, Version,
};

/// The report format and its version.
pub const REPORT_FORMAT: &str = "rupa-migrate-report/1";

/// Renders `outcome` as one JSON document.
pub(super) fn render(outcome: &Outcome<'_>) -> Value {
    let exit = outcome.exit();

    json!({
        "format": REPORT_FORMAT,
        "command": outcome.command,
        "stream": outcome.stream,
        "scheme": outcome.scheme.as_str(),
        "target": outcome.target,
        "exit": { "code": exit.code(), "status": exit.as_str() },
        "report": outcome.report.as_ref().map(report),
        "failure": outcome.failure.as_ref().map(failure),
    })
}

fn report(report: &Report<'_>) -> Value {
    match report {
        Report::Created(created) => json!({
            "version": created.version.to_string(),
            "paths": created.paths.iter().map(|path| path.display().to_string()).collect::<Vec<_>>(),
        }),
        Report::Validated(engine) => json!({ "engine": engine.as_str() }),
        Report::Status(status) => self::status(status),
        Report::Plan(plan) => json!({
            "status": status(&plan.status),
            "steps": plan.steps.iter().map(|step| step_of(None, step.entry, &step.execution)).collect::<Vec<_>>(),
            "covered": versions(&plan.covered),
            "late": versions(&plan.late),
        }),
        Report::RevertPlan(plan) => json!({
            "status": status(&plan.status),
            "steps": plan.steps.iter().map(|undo| step_of(Some(undo.installation_rank), undo.entry, &undo.execution)).collect::<Vec<_>>(),
            "remaining": versions(&plan.remaining),
        }),
        Report::Applied(applied) => json!({
            "snapshot": applied.snapshot.map(|version| version.to_string()),
            "covered": versions(&applied.covered),
            "applied": versions(&applied.applied),
            "late": versions(&applied.late),
        }),
        Report::Reverted(reverted) => json!({
            "reverted": versions(&reverted.reverted),
            "remaining": versions(&reverted.remaining),
        }),
        Report::Adopted {
            report,
            through,
            reason,
            evidence,
        } => json!({
            "through": through.to_string(),
            "adopted": versions(&report.adopted),
            "reason": reason,
            "evidence": evidence,
            "verified": false,
        }),
        Report::Recovered {
            action,
            reason,
            evidence,
        } => {
            let action = match action {
                RecoveryAction::Completed(id) => json!({ "kind": "completed", "attempt": id }),
                RecoveryAction::Restored(id) => json!({ "kind": "restored", "attempt": id }),
                RecoveryAction::ReleaseClaim => json!({ "kind": "release-claim" }),
            };

            json!({
                "action": action,
                "reason": reason,
                "evidence": evidence,
                "verified": false,
            })
        }
    }
}

fn status(status: &Status<'_>) -> Value {
    let classification = match status.classification {
        Classification::Fresh => "fresh",
        Classification::Managed => "managed",
        Classification::Unmanaged => "unmanaged",
        Classification::Uncertain => "uncertain",
    };
    let exclusion = match status.exclusion {
        Exclusion::Free => "free",
        Exclusion::Locked => "locked",
        Exclusion::Claimed => "claimed",
    };

    json!({
        "classification": classification,
        "exclusion": exclusion,
        "initialized": status.initialized,
        "history": status.history.iter().map(history).collect::<Vec<_>>(),
        "coverage": status.coverage.as_ref().map(coverage),
        "attempts": status.attempts.iter().map(attempt).collect::<Vec<_>>(),
        "pending": status.pending.iter().map(|e| entry(e)).collect::<Vec<_>>(),
        "blockers": status.blockers.iter().map(blocker).collect::<Vec<_>>(),
    })
}

fn history(entry: &HistoryEntry) -> Value {
    json!({
        "installation_rank": entry.installation_rank,
        "version": entry.version,
        "checksum": entry.checksum.to_string(),
        "provenance": entry.provenance.as_str(),
    })
}

fn coverage(coverage: &Coverage) -> Value {
    let source = match coverage.source {
        CoverageSource::Snapshot {
            installation_rank,
            checksum,
            provenance,
        } => json!({
            "kind": "snapshot",
            "installation_rank": installation_rank,
            "checksum": checksum.to_string(),
            "provenance": provenance.as_str(),
        }),
        CoverageSource::Adoption { id } => json!({ "kind": "adoption", "event": id }),
    };
    let members = coverage
        .members
        .iter()
        .map(|member| json!({ "version": member.version, "checksum": member.checksum.to_string() }))
        .collect::<Vec<_>>();

    json!({
        "boundary": coverage.boundary,
        "source": source,
        "members": members,
    })
}

fn attempt(attempt: &Attempt) -> Value {
    let direction = match attempt.direction {
        Direction::Up => json!({ "kind": "up" }),
        Direction::Down { undoes } => json!({ "kind": "down", "undoes": undoes }),
    };

    json!({
        "id": attempt.id,
        "version": attempt.version,
        "checksum": attempt.checksum.to_string(),
        "policy": attempt.policy.as_str(),
        "kind": kind(attempt.kind),
        "direction": direction,
    })
}

fn entry(entry: &CatalogEntry) -> Value {
    json!({
        "version": entry.version.to_string(),
        "kind": kind(entry.metadata.kind),
        "description": entry.metadata.description,
        "policy": entry.metadata.policy.as_str(),
        "checksum": entry.checksum.to_string(),
        "reversible": entry.reversal != Reversal::Irreversible,
        "source": entry.metadata.inputs.first().map(|input| input.path),
    })
}

fn step_of(rank: Option<u64>, entry: &CatalogEntry, execution: &Execution) -> Value {
    let execution = match execution {
        Execution::Static(scripts) => json!({ "kind": "static", "scripts": scripts }),
        Execution::Opaque => json!({ "kind": "opaque" }),
    };

    json!({
        "installation_rank": rank,
        "entry": self::entry(entry),
        "execution": execution,
    })
}

fn blocker(blocker: &Blocker) -> Value {
    let (kind, fields) = match blocker {
        Blocker::Unmanaged => ("unmanaged", json!({})),
        Blocker::Stream { stream, scheme } => {
            ("stream", json!({ "stream": stream, "scheme": scheme }))
        }
        Blocker::Locked => ("locked", json!({})),
        Blocker::Claimed => ("claimed", json!({})),
        Blocker::Interrupted(attempt) => {
            ("interrupted", json!({ "attempt": self::attempt(attempt) }))
        }
        Blocker::Unresolved(attempt) => {
            ("unresolved", json!({ "attempt": self::attempt(attempt) }))
        }
        Blocker::Missing { version, checksum } => (
            "missing",
            json!({ "version": version, "checksum": checksum.to_string() }),
        ),
        Blocker::Changed {
            version,
            installed,
            catalog,
        } => (
            "changed",
            json!({
                "version": version.to_string(),
                "installed": installed.to_string(),
                "catalog": catalog.to_string(),
            }),
        ),
        Blocker::Duplicate(version) => ("duplicate", json!({ "version": version.to_string() })),
        Blocker::Late { version, checksum } => (
            "late",
            json!({ "version": version.to_string(), "checksum": checksum.to_string() }),
        ),
        Blocker::Uncovered { version, boundary } => (
            "uncovered",
            json!({ "version": version.to_string(), "boundary": boundary.to_string() }),
        ),
        Blocker::Irreversible(version) => {
            ("irreversible", json!({ "version": version.to_string() }))
        }
        Blocker::Interleaved(version) => ("interleaved", json!({ "version": version.to_string() })),
        Blocker::Exceeds {
            requested,
            installed,
        } => (
            "exceeds",
            json!({ "requested": requested, "installed": installed }),
        ),
        Blocker::Boundary {
            requested,
            boundary,
        } => (
            "boundary",
            json!({ "requested": requested.to_string(), "boundary": boundary.to_string() }),
        ),
    };
    let mut value = json!({ "kind": kind, "message": human::describe(blocker) });

    if let (Value::Object(value), Value::Object(fields)) = (&mut value, fields) {
        value.extend(fields);
    }

    value
}

fn failure(failure: &Failure) -> Value {
    json!({
        "status": failure.exit.as_str(),
        "message": failure.message,
        "causes": failure.causes,
        "migration": failure.migration.map(|version| version.to_string()),
        "source": failure.source,
        "blockers": failure.blockers.iter().map(blocker).collect::<Vec<_>>(),
        "cleanup": failure.cleanup,
    })
}

fn kind(kind: MigrationKind) -> &'static str {
    match kind {
        MigrationKind::Incremental => "incremental",
        MigrationKind::Snapshot => "snapshot",
    }
}

fn versions(versions: &[Version]) -> Vec<String> {
    versions.iter().map(ToString::to_string).collect()
}
