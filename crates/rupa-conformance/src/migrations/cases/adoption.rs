use rupa::migrate::{
    Adoption, AdoptionError, Blocker, CoverageSource, Error, RevertTarget, RunError,
};

use super::support::{ALLOW_LATE, blocked, history, script, undoable, version};
use super::{Adopt, Apply, Harness, Inspect, Revert, catalog, versions};

/// The audit reason the cases adopt with.
const REASON: &str = "restored from the legacy schema";

pub(super) fn case_an_unmanaged_target_is_adopted_without_running_bodies<H: Harness>(h: &mut H) {
    let entries = [
        script::<H>("1", "CREATE TABLE t1 (id INTEGER);"),
        script::<H>("2", "CREATE TABLE t2 (id INTEGER);"),
        script::<H>("3", "CREATE TABLE t3 (id INTEGER);"),
    ];
    let legacy = catalog(&entries);
    let adoption = Adoption::new(version("2"), REASON).with_evidence("schema comparison");

    h.execute("CREATE TABLE t1 (id INTEGER)");
    h.execute("CREATE TABLE t2 (id INTEGER)");

    let refused = wait!(h, legacy.apply(h.connect()));
    let report = wait!(h, legacy.adopt(h.connect(), &adoption)).expect("adopt");
    let status = wait!(h, legacy.status(h.connect())).expect("status");
    let tail = wait!(h, legacy.apply(h.connect())).expect("apply the tail");

    assert!(
        matches!(
            refused,
            Err(RunError::Infrastructure {
                source: Error::Unmanaged,
                ..
            })
        ),
        "ordinary apply never adopts"
    );
    assert_eq!(report.adopted, [version("1"), version("2")]);
    assert!(
        matches!(
            &status.coverage,
            Some(coverage) if coverage.boundary == "2"
                && coverage.members.len() == 2
                && matches!(coverage.source, CoverageSource::Adoption { .. })
        ),
        "{status:?}"
    );
    assert!(
        status.history.is_empty(),
        "adopted incrementals are not installations"
    );
    assert_eq!(
        status
            .pending
            .iter()
            .map(|entry| entry.version.to_string())
            .collect::<Vec<_>>(),
        ["3"]
    );
    assert_eq!(versions(&tail), ["3"], "adopted bodies never run");
    assert!(tail.snapshot.is_none());
    assert_eq!(history(h), ["3"]);
    assert_eq!(
        h.count(&format!(
            "SELECT count(*) FROM _rupa_migrations WHERE kind = 'adopt' AND reason = '{REASON}' AND evidence = 'schema comparison'"
        )),
        1,
        "the adoption is audited"
    );
}

pub(super) fn case_adoption_needs_an_unmanaged_target_a_reason_and_coverage<H: Harness>(h: &mut H) {
    let entries = [
        script::<H>("2", "CREATE TABLE t2 (id INTEGER);"),
        script::<H>("3", "CREATE TABLE t3 (id INTEGER);"),
    ];
    let legacy = catalog(&entries);
    let adopt = |h: &H, through: &str, reason: &str| {
        wait!(
            h,
            legacy.adopt(h.connect(), &Adoption::new(version(through), reason))
        )
    };
    let refused = |result: Result<_, Error<_>>, expected: AdoptionError| matches!(result, Err(Error::Adoption(error)) if error == expected);

    assert!(refused(adopt(h, "2", REASON), AdoptionError::Fresh));
    assert!(
        !h.table_exists("_rupa_migrations"),
        "a refused adoption writes nothing"
    );

    h.execute("CREATE TABLE t2 (id INTEGER)");

    assert!(refused(adopt(h, "2", " "), AdoptionError::MissingReason));
    assert!(refused(
        adopt(h, "1", REASON),
        AdoptionError::NothingCovered(version("1"))
    ));
    assert!(!h.table_exists("_rupa_migrations"));

    adopt(h, "2", REASON).expect("adopt");

    assert!(
        refused(adopt(h, "3", REASON), AdoptionError::Managed),
        "a managed target is never adopted again"
    );

    h.reset();
    wait!(h, legacy.apply(h.connect())).expect("apply");

    assert!(refused(adopt(h, "2", REASON), AdoptionError::Managed));
}

pub(super) fn case_an_adopted_boundary_refuses_uncovered_work_and_deeper_reverts<H: Harness>(
    h: &mut H,
) {
    let one = script::<H>("1", "CREATE TABLE t1 (id INTEGER);");
    let two = script::<H>("2", "CREATE TABLE t2 (id INTEGER);");
    let three = script::<H>("3", "CREATE TABLE t3 (id INTEGER);");
    let four = undoable::<H>("4", "CREATE TABLE t4 (id INTEGER);", "DROP TABLE t4;");
    let adopted = [one, three];
    let late = [one, two, three, four];
    let tail = [one, three, four];

    h.execute("CREATE TABLE t1 (id INTEGER)");
    h.execute("CREATE TABLE t3 (id INTEGER)");
    wait!(
        h,
        catalog(&adopted).adopt(h.connect(), &Adoption::new(version("3"), REASON))
    )
    .expect("adopt 1 and 3");

    assert_eq!(
        blocked(wait!(
            h,
            catalog(&late).apply_with(h.connect(), &ALLOW_LATE)
        )),
        [Blocker::Uncovered {
            version: version("2"),
            boundary: version("3"),
        }],
        "an adoption never silently covers a later historical artifact"
    );

    let tail = catalog(&tail);

    wait!(h, tail.apply(h.connect())).expect("apply 4");

    assert_eq!(
        blocked(wait!(
            h,
            tail.revert(h.connect(), RevertTarget::To(version("1")))
        )),
        [Blocker::Boundary {
            requested: version("1"),
            boundary: version("3"),
        }]
    );
    assert_eq!(
        wait!(h, tail.revert(h.connect(), RevertTarget::To(version("3"))))
            .expect("revert the tail")
            .reverted,
        [version("4")]
    );
    assert!(h.table_exists("t1") && h.table_exists("t3") && !h.table_exists("t4"));
}
