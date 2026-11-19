//! Lines for people. Every line is plain text; the JSON report is the
//! machine contract.

mod target;

use std::fmt::Write as _;

use target::{blockers, status, steps};

pub(super) use target::describe;

use super::{Failure, Outcome, Report};
use crate::{Checksum, RecoveryAction, Version};

/// Hex digits of a checksum shown to people.
const SHORT_CHECKSUM: usize = 12;

/// Renders `outcome` as lines for people.
pub(super) fn render(outcome: &Outcome<'_>) -> String {
    let mut out = String::new();
    let target = outcome
        .target
        .as_deref()
        .map(|target| format!(", target {target}"))
        .unwrap_or_default();

    line(
        &mut out,
        0,
        format_args!(
            "{}: stream {} ({}){target}",
            outcome.command,
            outcome.stream,
            outcome.scheme.as_str()
        ),
    );

    if let Some(report) = &outcome.report {
        self::report(&mut out, report);
    }

    if let Some(failure) = &outcome.failure {
        self::failure(&mut out, failure);
    }

    line(
        &mut out,
        0,
        format_args!(
            "result: {} (exit {})",
            outcome.exit().as_str(),
            outcome.exit().code()
        ),
    );

    out
}

fn report(out: &mut String, report: &Report<'_>) {
    match report {
        Report::Created(created) => {
            line(out, 1, format_args!("created version {}", created.version));

            for path in &created.paths {
                line(out, 2, format_args!("{}", path.display()));
            }
        }
        Report::Validated(engine) => {
            line(
                out,
                1,
                format_args!("the catalog is valid for {}", engine.as_str()),
            );
        }
        Report::Status(found) => status(out, found),
        Report::Plan(plan) => {
            status(out, &plan.status);
            versions(out, "covered by the snapshot", &plan.covered);
            versions(out, "late, by permission", &plan.late);
            steps(
                out,
                "would run",
                plan.steps
                    .iter()
                    .map(|step| (None, step.entry, &step.execution)),
            );
        }
        Report::RevertPlan(plan) => {
            status(out, &plan.status);
            steps(
                out,
                "would undo",
                plan.steps
                    .iter()
                    .map(|undo| (Some(undo.installation_rank), undo.entry, &undo.execution)),
            );
            versions(out, "would remain", &plan.remaining);
        }
        Report::Applied(applied) => {
            if applied.snapshot.is_none() && applied.applied.is_empty() {
                line(out, 1, format_args!("nothing to apply"));
            }

            if let Some(snapshot) = applied.snapshot {
                line(out, 1, format_args!("initialized from snapshot {snapshot}"));
            }

            versions(out, "covered by the snapshot, not run", &applied.covered);
            versions(out, "applied", &applied.applied);
            versions(out, "ran late, by permission", &applied.late);
        }
        Report::Reverted(reverted) => {
            if reverted.reverted.is_empty() {
                line(out, 1, format_args!("nothing to revert"));
            }

            versions(out, "reverted", &reverted.reverted);
            versions(out, "remaining", &reverted.remaining);
        }
        Report::Adopted {
            report,
            through,
            reason,
            evidence,
        } => {
            line(
                out,
                1,
                format_args!(
                    "asserted: the target already holds every incremental through {through}; nothing verified it"
                ),
            );
            versions(out, "recorded as covered, not run", &report.adopted);
            audit(out, reason, evidence.as_deref());
        }
        Report::Recovered {
            action,
            reason,
            evidence,
        } => {
            let assertion = match action {
                RecoveryAction::Completed(id) => format!(
                    "attempt {id} completed: recorded as installed, provenance recovered; its body did not run again"
                ),
                RecoveryAction::Restored(id) => format!(
                    "attempt {id} was manually undone: abandoned; only a new apply runs its artifact"
                ),
                RecoveryAction::ReleaseClaim => {
                    "the runner that left the owner claim has stopped: the claim is cleared".into()
                }
            };

            line(out, 1, format_args!("asserted: {assertion}"));
            audit(out, reason, evidence.as_deref());
        }
    }
}

fn failure(out: &mut String, failure: &Failure) {
    line(out, 0, format_args!("error: {}", failure.message));

    if let Some(source) = failure.source {
        line(out, 1, format_args!("source: {source}"));
    }

    for cause in &failure.causes {
        line(out, 1, format_args!("caused by: {cause}"));
    }

    blockers(out, &failure.blockers);

    for cleanup in &failure.cleanup {
        line(out, 1, format_args!("cleanup failed: {cleanup}"));
    }
}

fn audit(out: &mut String, reason: &str, evidence: Option<&str>) {
    line(out, 1, format_args!("reason: {reason}"));

    if let Some(evidence) = evidence {
        line(out, 1, format_args!("evidence: {evidence}"));
    }

    line(out, 1, format_args!("the journal keeps this assertion"));
}

fn versions(out: &mut String, label: &str, versions: &[Version]) {
    if !versions.is_empty() {
        let list = versions
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");

        line(out, 1, format_args!("{label}: {list}"));
    }
}

fn short(checksum: &Checksum) -> String {
    checksum.to_string()[..SHORT_CHECKSUM].to_owned()
}

/// Appends `text` at `depth`. Writing to a `String` cannot fail, so the
/// result is ignored.
fn line(out: &mut String, depth: usize, text: std::fmt::Arguments<'_>) {
    let _ = writeln!(out, "{:width$}{text}", "", width = depth * 2);
}
