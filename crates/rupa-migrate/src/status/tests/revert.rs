use super::*;
use crate::RevertTarget;

/// The status of a target whose history installed `order`, in that order.
fn status_after<'c>(catalog: &'c Catalog<'_, Entry>, order: &[&str]) -> Status<'c> {
    let history = order
        .iter()
        .zip(1..)
        .map(|(version, rank)| installed(catalog, version, rank))
        .collect();

    catalog.classify(inspection(Exclusion::Free, managed(history, Vec::new())))
}

/// `(rank, version)` of each step, in run order.
fn steps(plan: &RevertPlan<'_>) -> Vec<(u64, String)> {
    plan.steps
        .iter()
        .map(|step| (step.installation_rank, step.entry.version.to_string()))
        .collect()
}

fn revert_plan<'c>(
    catalog: &'c Catalog<'_, Entry>,
    order: &[&str],
    target: RevertTarget,
) -> RevertPlan<'c> {
    catalog
        .revert_for(status_after(catalog, order), DialectId::Sqlite, target)
        .expect("renderable revert plan")
}

#[test]
fn a_version_target_undoes_the_installations_above_it_newest_first() {
    let entries = [reversible("100"), reversible("101"), reversible("102")];
    let catalog = catalog(&entries);

    let plan = revert_plan(
        &catalog,
        &["100", "102", "101"],
        RevertTarget::To(version("100")),
    );

    assert!(
        plan.status.blockers.is_empty(),
        "{:?}",
        plan.status.blockers
    );
    assert_eq!(steps(&plan), [(3, "101".into()), (2, "102".into())]);
    assert_eq!(plan.remaining, [version("100")]);
}

#[test]
fn a_retained_installation_among_the_undone_refuses_the_revert() {
    let entries = [reversible("100"), reversible("101"), reversible("102")];
    let catalog = catalog(&entries);

    let plan = revert_plan(
        &catalog,
        &["100", "102", "101"],
        RevertTarget::To(version("101")),
    );

    assert_eq!(
        plan.status.blockers,
        [Blocker::Interleaved(version("101"))],
        "undoing only 102 would leave 101 running on top of it"
    );
}

#[test]
fn a_count_undoes_the_newest_installations_whatever_their_versions() {
    let entries = [reversible("100"), reversible("101"), reversible("102")];
    let catalog = catalog(&entries);

    let plan = revert_plan(&catalog, &["100", "102", "101"], RevertTarget::Last(1));

    assert!(plan.status.blockers.is_empty());
    assert_eq!(steps(&plan), [(3, "101".into())]);
    assert_eq!(plan.remaining, [version("100"), version("102")]);
}

#[test]
fn an_irreversible_installation_refuses_the_whole_revert() {
    let entries = [reversible("1"), imperative("2"), undone("3")];
    let catalog = catalog(&entries);

    let plan = revert_plan(&catalog, &["1", "2", "3"], RevertTarget::Last(3));

    assert_eq!(plan.status.blockers, [Blocker::Irreversible(version("2"))]);
    assert_eq!(
        steps(&plan),
        [(3, "3".into()), (1, "1".into())],
        "the steps that would run are still listed beside the blocker"
    );
}

#[test]
fn counting_past_the_installations_undoes_nothing() {
    let entries = [reversible("1"), reversible("2")];
    let catalog = catalog(&entries);

    let plan = revert_plan(&catalog, &["1", "2"], RevertTarget::Last(3));

    assert_eq!(
        plan.status.blockers,
        [Blocker::Exceeds {
            requested: 3,
            installed: 2,
        }]
    );
    assert!(plan.steps.is_empty());
    assert_eq!(plan.remaining, [version("1"), version("2")]);
}

#[test]
fn late_artifacts_never_block_a_revert_but_changed_history_does() {
    let entries = [reversible("1"), reversible("2"), reversible("3")];
    let catalog = catalog(&entries);
    let changed = HistoryEntry {
        checksum: Checksum::from_bytes([1; 32]),
        ..installed(&catalog, "3", 2)
    };

    let late = revert_plan(&catalog, &["1", "3"], RevertTarget::Last(1));
    let status = catalog.classify(inspection(
        Exclusion::Free,
        managed(vec![installed(&catalog, "1", 1), changed], Vec::new()),
    ));
    let blocked = catalog
        .revert_for(status, DialectId::Sqlite, RevertTarget::Last(1))
        .expect("renderable revert plan");

    assert!(
        late.status.blockers.is_empty(),
        "{:?}",
        late.status.blockers
    );
    assert_eq!(steps(&late), [(2, "3".into())]);
    assert!(matches!(
        blocked.status.blockers.as_slice(),
        [Blocker::Changed { version: changed, .. }] if *changed == version("3")
    ));
}

#[test]
fn planned_down_bodies_render_and_imperative_ones_stay_opaque() {
    let entries = [undone("1"), reversible("2")];
    let catalog = catalog(&entries);

    let plan = revert_plan(&catalog, &["1", "2"], RevertTarget::Last(2));

    assert_eq!(
        plan.steps
            .iter()
            .map(|step| step.execution.clone())
            .collect::<Vec<_>>(),
        [
            Execution::Opaque,
            Execution::Static(vec!["DROP TABLE planned".into()]),
        ]
    );
}
