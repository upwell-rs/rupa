use super::*;
use crate::{CoverageMember, CoverageSource, Init, RevertTarget};

/// A claim through `boundary` by `source`, listing `members` with the
/// checksums `catalog` holds for them.
fn claim(
    catalog: &Catalog<'_, Entry>,
    boundary: &str,
    source: CoverageSource,
    members: &[&str],
) -> Coverage {
    let members = members
        .iter()
        .map(|version| CoverageMember {
            version: (*version).into(),
            checksum: installed(catalog, version, 1).checksum,
        })
        .collect();

    Coverage {
        boundary: boundary.into(),
        source,
        members,
    }
}

/// An adoption through `boundary` listing `members`.
fn adopted(catalog: &Catalog<'_, Entry>, boundary: &str, members: &[&str]) -> Coverage {
    claim(
        catalog,
        boundary,
        CoverageSource::Adoption { id: 2 },
        members,
    )
}

/// The status of a target with `coverage` and then `history` installed.
fn status_with<'c>(
    catalog: &'c Catalog<'_, Entry>,
    coverage: Option<Coverage>,
    history: &[&str],
) -> Status<'c> {
    let history = history
        .iter()
        .zip(2..)
        .map(|(version, rank)| installed(catalog, version, rank))
        .collect();

    catalog.classify(inspection(
        Exclusion::Free,
        journal(history, coverage, Vec::new()),
    ))
}

/// The steps of a plan, a snapshot's prefixed with `s`, and what it covers.
fn steps(plan: &ApplyPlan<'_>) -> (Vec<String>, Vec<Version>) {
    let steps = plan
        .steps
        .iter()
        .map(|step| match step.entry.metadata.kind {
            MigrationKind::Snapshot => format!("s{}", step.entry.version),
            MigrationKind::Incremental => step.entry.version.to_string(),
        })
        .collect();

    (steps, plan.covered.clone())
}

fn plan<'c>(
    catalog: &'c Catalog<'_, Entry>,
    status: Status<'c>,
    options: &ApplyOptions,
) -> ApplyPlan<'c> {
    catalog
        .plan_for(status, DialectId::Sqlite, options)
        .expect("renderable plan")
}

fn through(text: &str) -> ApplyOptions {
    ApplyOptions {
        through: Some(version(text)),
        ..ApplyOptions::DEFAULT
    }
}

#[test]
fn a_fresh_target_runs_the_newest_eligible_snapshot_and_then_the_tail() {
    let entries = [
        imperative("1"),
        imperative("2"),
        imperative("3"),
        imperative("4"),
        imperative("5"),
        snapshot("2"),
        snapshot("4"),
    ];
    let catalog = catalog(&entries);
    let fresh = || catalog.classify(inspection(Exclusion::Free, Journal::Fresh));

    let to_three = plan(&catalog, fresh(), &through("3"));
    let everything = plan(&catalog, fresh(), &ApplyOptions::DEFAULT);

    assert_eq!(
        steps(&to_three),
        (
            vec!["s2".into(), "3".into()],
            vec![version("1"), version("2")]
        ),
        "snapshot 4 is above the target"
    );
    assert_eq!(
        steps(&everything).0,
        ["s4", "5"],
        "the newest snapshot wins without a target"
    );
}

#[test]
fn without_an_eligible_snapshot_or_with_full_replay_incrementals_run() {
    let entries = [
        imperative("1"),
        imperative("2"),
        imperative("3"),
        imperative("4"),
        snapshot("2"),
        snapshot("4"),
    ];
    let catalog = catalog(&entries);
    let fresh = || catalog.classify(inspection(Exclusion::Free, Journal::Fresh));
    let replay = ApplyOptions {
        init: Init::FullReplay,
        ..through("3")
    };

    let below = plan(&catalog, fresh(), &through("1"));
    let replayed = plan(&catalog, fresh(), &replay);

    assert_eq!(steps(&below), (vec!["1".into()], Vec::new()));
    assert_eq!(
        steps(&replayed),
        (vec!["1".into(), "2".into(), "3".into()], Vec::new())
    );
}

#[test]
fn a_managed_target_never_runs_a_snapshot() {
    let entries = [
        imperative("1"),
        imperative("2"),
        imperative("3"),
        snapshot("3"),
    ];
    let catalog = catalog(&entries);

    let incremental = plan(
        &catalog,
        status_with(&catalog, None, &["1", "2"]),
        &ApplyOptions::DEFAULT,
    );
    let adopted = plan(
        &catalog,
        status_with(&catalog, Some(adopted(&catalog, "1", &["1"])), &[]),
        &ApplyOptions::DEFAULT,
    );

    assert_eq!(steps(&incremental), (vec!["3".into()], Vec::new()));
    assert_eq!(
        steps(&adopted),
        (vec!["2".into(), "3".into()], Vec::new()),
        "coverage makes a target managed even without installations"
    );
}

#[test]
fn only_a_journal_that_was_never_initialized_may_start_from_a_snapshot() {
    let entries = [imperative("1"), imperative("2"), snapshot("2")];
    let catalog = catalog(&entries);
    let journal = |initialized| {
        Journal::Managed(Records {
            stream: "primary".into(),
            scheme: "numbered".into(),
            history: Vec::new(),
            coverage: None,
            initialized,
            attempts: Vec::new(),
        })
    };
    let steps_for = |initialized| {
        let status = catalog.classify(inspection(Exclusion::Free, journal(initialized)));

        steps(&plan(&catalog, status, &ApplyOptions::DEFAULT)).0
    };

    assert_eq!(
        steps_for(false),
        ["s2"],
        "a journal of failed attempts only can still start from the snapshot"
    );
    assert_eq!(
        steps_for(true),
        ["1", "2"],
        "a target whose installations were all reverted replays instead"
    );
}

#[test]
fn covered_incrementals_count_as_present_and_only_the_tail_is_pending() {
    let entries = [imperative("1"), imperative("2"), imperative("3")];
    let catalog = catalog(&entries);
    let coverage = adopted(&catalog, "2", &["1", "2"]);

    let status = status_with(&catalog, Some(coverage.clone()), &[]);

    assert_eq!(status.classification, Classification::Managed);
    assert!(status.blockers.is_empty(), "{:?}", status.blockers);
    assert_eq!(pending(&status), ["3"]);
    assert_eq!(status.coverage, Some(coverage));
    assert!(
        status.history.is_empty(),
        "covered incrementals are not installations"
    );
}

#[test]
fn an_uncovered_incremental_at_or_below_the_boundary_always_blocks() {
    let entries = [
        imperative("1"),
        imperative("2"),
        imperative("3"),
        imperative("4"),
    ];
    let catalog = catalog(&entries);
    let allow_late = ApplyOptions {
        order: Order::AllowLate,
        ..ApplyOptions::DEFAULT
    };
    let gap = adopted(&catalog, "3", &["1", "3"]);
    let at_boundary = adopted(&catalog, "3", &["1", "2"]);

    let gap = plan(
        &catalog,
        status_with(&catalog, Some(gap), &["4"]),
        &allow_late,
    );
    let at_boundary = status_with(&catalog, Some(at_boundary), &[]);

    assert_eq!(
        gap.status.blockers,
        [Blocker::Uncovered {
            version: version("2"),
            boundary: version("3"),
        }],
        "late permission never covers a historical gap"
    );
    assert!(gap.late.is_empty());
    assert_eq!(
        at_boundary.blockers,
        [Blocker::Uncovered {
            version: version("3"),
            boundary: version("3"),
        }],
        "an artifact at the boundary is not part of the tail"
    );
}

#[test]
fn changed_members_and_a_missing_or_changed_snapshot_are_reported() {
    let entries = [imperative("1"), imperative("2"), snapshot("2")];
    let catalog = catalog(&entries);
    let snapshot = catalog.snapshots().next().expect("snapshot");
    let executed = |checksum| CoverageSource::Snapshot {
        installation_rank: 1,
        checksum,
        provenance: Provenance::Executed,
    };
    let mut changed = claim(&catalog, "2", executed(snapshot.checksum), &["1", "2"]);
    let other = Checksum::from_bytes([9; 32]);

    changed.members[0].checksum = other;

    let members = status_with(&catalog, Some(changed), &[]);
    let edited = status_with(
        &catalog,
        Some(claim(&catalog, "2", executed(other), &["1", "2"])),
        &[],
    );
    let missing = status_with(
        &catalog,
        Some(claim(&catalog, "1", executed(other), &["1"])),
        &[],
    );

    assert!(matches!(
        members.blockers.as_slice(),
        [Blocker::Changed { version: changed, installed, .. }]
            if *changed == version("1") && *installed == other
    ));
    assert!(matches!(
        edited.blockers.as_slice(),
        [Blocker::Changed { version: changed, .. }] if *changed == version("2")
    ));
    assert!(matches!(
        missing.blockers.as_slice(),
        [Blocker::Missing { version, .. }] if version == "1"
    ));
}

#[test]
fn a_revert_reaches_the_boundary_but_never_crosses_it() {
    let entries = [reversible("1"), reversible("2"), reversible("3")];
    let catalog = catalog(&entries);
    let status = || status_with(&catalog, Some(adopted(&catalog, "2", &["1", "2"])), &["3"]);
    let revert = |target| {
        catalog
            .revert_for(status(), DialectId::Sqlite, target)
            .expect("renderable revert plan")
    };

    let to_boundary = revert(RevertTarget::To(version("2")));
    let below = revert(RevertTarget::To(version("1")));
    let beyond_tail = revert(RevertTarget::Last(2));

    assert!(to_boundary.status.blockers.is_empty());
    assert_eq!(to_boundary.steps.len(), 1);
    assert_eq!(
        below.status.blockers,
        [Blocker::Boundary {
            requested: version("1"),
            boundary: version("2"),
        }]
    );
    assert_eq!(
        beyond_tail.status.blockers,
        [Blocker::Exceeds {
            requested: 2,
            installed: 1,
        }],
        "covered incrementals are not installations a count can reach"
    );
}
