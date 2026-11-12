//! What a target and its catalog look like to people.

use super::{line, short};
use crate::{
    Attempt, Blocker, CatalogEntry, Classification, Coverage, CoverageSource, Direction, Exclusion,
    Execution, HistoryEntry, MigrationKind, Reversal, Status,
};

pub(super) fn status(out: &mut String, status: &Status<'_>) {
    let classification = match status.classification {
        Classification::Fresh => "fresh",
        Classification::Managed => "managed",
        Classification::Unmanaged => "unmanaged",
        Classification::Uncertain => "uncertain",
    };
    let exclusion = match status.exclusion {
        Exclusion::Free => "free",
        Exclusion::Locked => "locked by a live runner",
        Exclusion::Claimed => "claimed by a SQLite owner",
    };
    let initialized = if status.initialized {
        "initialized"
    } else {
        "never initialized"
    };

    line(
        out,
        1,
        format_args!("target: {classification}, {initialized}, exclusion {exclusion}"),
    );

    if !status.history.is_empty() {
        line(out, 1, format_args!("installed:"));

        for entry in &status.history {
            history(out, entry);
        }
    }

    if let Some(coverage) = &status.coverage {
        self::coverage(out, coverage);
    }

    if !status.attempts.is_empty() {
        line(out, 1, format_args!("running attempts:"));

        for attempt in &status.attempts {
            line(out, 2, format_args!("{}", self::attempt(attempt)));
        }
    }

    if status.pending.is_empty() {
        line(out, 1, format_args!("pending: none"));
    } else {
        line(out, 1, format_args!("pending:"));

        for entry in &status.pending {
            line(out, 2, format_args!("{}", self::entry(entry)));
        }
    }

    blockers(out, &status.blockers);
}

fn history(out: &mut String, entry: &HistoryEntry) {
    line(
        out,
        2,
        format_args!(
            "#{} {} {} {}",
            entry.installation_rank,
            entry.version,
            entry.provenance.as_str(),
            short(&entry.checksum)
        ),
    );
}

fn coverage(out: &mut String, coverage: &Coverage) {
    let source = match coverage.source {
        CoverageSource::Snapshot {
            installation_rank,
            provenance,
            ..
        } => format!(
            "snapshot {} (#{installation_rank}, {})",
            coverage.boundary,
            provenance.as_str()
        ),
        CoverageSource::Adoption { id } => {
            format!("adoption through {} (event {id})", coverage.boundary)
        }
    };
    let members = coverage
        .members
        .iter()
        .map(|member| member.version.as_str())
        .collect::<Vec<_>>()
        .join(", ");

    line(out, 1, format_args!("covered by {source}: {members}"));
}

fn attempt(attempt: &Attempt) -> String {
    let direction = match attempt.direction {
        Direction::Up => "up".to_owned(),
        Direction::Down { undoes } => format!("down, undoing #{undoes}"),
    };
    let kind = match attempt.kind {
        MigrationKind::Incremental => "",
        MigrationKind::Snapshot => " snapshot",
    };

    format!(
        "attempt {}: {}{kind} {direction}, {}",
        attempt.id,
        attempt.version,
        attempt.policy.as_str()
    )
}

fn entry(entry: &CatalogEntry) -> String {
    let kind = match entry.metadata.kind {
        MigrationKind::Incremental => "",
        MigrationKind::Snapshot => "snapshot ",
    };
    let reversible = match entry.reversal {
        Reversal::Irreversible => "",
        Reversal::Planned(_) | Reversal::Imperative => ", reversible",
    };
    let source = entry
        .metadata
        .inputs
        .first()
        .map(|input| format!(" [{}]", input.path))
        .unwrap_or_default();

    format!(
        "{kind}{} ({}{reversible}){source}",
        entry.version,
        entry.metadata.policy.as_str()
    )
}

pub(super) fn steps<'a>(
    out: &mut String,
    label: &str,
    steps: impl Iterator<Item = (Option<u64>, &'a CatalogEntry, &'a Execution)>,
) {
    let mut steps = steps.peekable();

    if steps.peek().is_none() {
        line(out, 1, format_args!("{label}: nothing"));

        return;
    }

    line(out, 1, format_args!("{label}:"));

    for (rank, entry, execution) in steps {
        let rank = rank.map(|rank| format!("#{rank} ")).unwrap_or_default();

        line(out, 2, format_args!("{rank}{}", self::entry(entry)));

        match execution {
            Execution::Static(scripts) => {
                for script in scripts {
                    for text in script.trim().lines() {
                        line(out, 3, format_args!("{text}"));
                    }
                }
            }
            Execution::Opaque => line(out, 3, format_args!("(imperative Rust body)")),
        }
    }
}

pub(super) fn blockers(out: &mut String, blockers: &[Blocker]) {
    if blockers.is_empty() {
        return;
    }

    line(out, 1, format_args!("blocked by:"));

    for blocker in blockers {
        line(out, 2, format_args!("{}", describe(blocker)));
    }
}

/// One line saying what `blocker` refuses and why.
pub(in crate::commands) fn describe(blocker: &Blocker) -> String {
    match blocker {
        Blocker::Unmanaged => "the target has user objects but no journal: adopt it".into(),
        Blocker::Stream { stream, scheme } => {
            format!("the journal belongs to stream {stream} ({scheme})")
        }
        Blocker::Locked => "another runner holds the target's lock".into(),
        Blocker::Claimed => {
            "SQLite's owner claim is set: release it once its runner has stopped".into()
        }
        Blocker::Interrupted(attempt) => format!(
            "interrupted {}; apply records it rolled back",
            self::attempt(attempt)
        ),
        Blocker::Unresolved(attempt) => format!(
            "unresolved {}; its effects may persist: recover it",
            self::attempt(attempt)
        ),
        Blocker::Missing { version, checksum } => format!(
            "{version} ({}) is recorded but missing from the catalog",
            short(checksum)
        ),
        Blocker::Changed {
            version,
            installed,
            catalog,
        } => format!(
            "{version} changed since it ran: recorded {}, catalog {}",
            short(installed),
            short(catalog)
        ),
        Blocker::Duplicate(version) => format!("{version} is recorded more than once"),
        Blocker::Late { version, .. } => {
            format!("{version} is pending below an installed version (late)")
        }
        Blocker::Uncovered { version, boundary } => {
            format!("{version} is at or below the coverage boundary {boundary} but not covered")
        }
        Blocker::Irreversible(version) => format!("{version} has no down body"),
        Blocker::Interleaved(version) => {
            format!("{version} stays installed among the installations to undo")
        }
        Blocker::Exceeds {
            requested,
            installed,
        } => format!("{requested} installations requested, {installed} installed"),
        Blocker::Boundary {
            requested,
            boundary,
        } => format!("{requested} is below the coverage boundary {boundary}"),
    }
}
