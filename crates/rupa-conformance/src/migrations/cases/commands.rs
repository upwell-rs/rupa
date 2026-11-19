use rupa::DialectId;
use rupa::migrate::commands::{Command, Exit, Format, Redactor, Report};
use rupa::migrate::{Adoption, ApplyOptions, Engine, Recovery, RecoveryAction, RevertTarget};

use super::support::{dialect, history, script, undoable, version};
use super::{Harness, catalog, operate, unreachable};
use crate::migrations::fixture::Body;

/// `1` and `2`, each creating a table its down body drops.
fn pair<H: Harness>() -> [Body; 2] {
    [
        undoable::<H>("1", "CREATE TABLE c1 (id INTEGER);", "DROP TABLE c1;"),
        undoable::<H>("2", "CREATE TABLE c2 (id INTEGER);", "DROP TABLE c2;"),
    ]
}

/// The journal rows of every kind.
fn journal<H: Harness>(h: &H) -> i64 {
    h.count("SELECT count(*) FROM _rupa_migrations")
}

pub(super) fn case_inspection_commands_never_write_or_run_bodies<H: Harness>(h: &mut H) {
    let entries = pair::<H>();
    let pair = catalog(&entries);
    let reads = [
        Command::Status,
        Command::Plan(ApplyOptions::DEFAULT),
        Command::RevertPlan(RevertTarget::Last(1)),
    ];

    for command in &reads {
        let outcome = operate(h, &pair, command);

        assert!(outcome.failure.is_none(), "{outcome:?}");
        assert!(
            !h.table_exists("_rupa_migrations") && !h.table_exists("c1"),
            "{} on a fresh target creates nothing",
            command.name()
        );
    }

    let applied = operate(h, &pair, &Command::Apply(ApplyOptions::DEFAULT));
    let rows = journal(h);

    assert_eq!(applied.exit(), Exit::Success, "{applied:?}");

    for command in &reads {
        let outcome = operate(h, &pair, command);

        assert_eq!(outcome.exit(), Exit::Success, "{outcome:?}");
        assert_eq!(journal(h), rows, "{} writes nothing", command.name());
    }

    assert!(h.table_exists("c2"), "the revert preview ran no down body");
}

pub(super) fn case_commands_apply_and_revert_through_the_library_engine<H: Harness>(h: &mut H) {
    let entries = pair::<H>();
    let pair = catalog(&entries);
    let first = [entries[0]];
    let first = catalog(&first);

    let applied = operate(h, &first, &Command::Apply(ApplyOptions::DEFAULT));
    let late = operate(h, &pair, &Command::Plan(ApplyOptions::DEFAULT));
    let rest = operate(h, &pair, &Command::Apply(ApplyOptions::DEFAULT));
    let reverted = operate(h, &pair, &Command::Revert(RevertTarget::Last(1)));
    let json: serde_json::Value =
        serde_json::from_str(&reverted.render(Format::Json, &Redactor::new())).expect("JSON");

    assert!(
        matches!(&applied.report, Some(Report::Applied(report)) if report.applied == [version("1")]),
        "{applied:?}"
    );
    assert!(
        matches!(&late.report, Some(Report::Plan(plan)) if plan.steps.len() == 1),
        "{late:?}"
    );
    assert_eq!(rest.exit(), Exit::Success);
    assert!(
        matches!(&reverted.report, Some(Report::Reverted(report)) if report.reverted == [version("2")]),
        "{reverted:?}"
    );
    assert_eq!(json["exit"]["code"], 0);
    assert_eq!(json["report"]["remaining"], serde_json::json!(["1"]));
    assert_eq!(history(h), ["1"]);
    assert!(h.table_exists("c1") && !h.table_exists("c2"));

    let blocked = operate(h, &pair, &Command::Revert(RevertTarget::Last(2)));

    assert_eq!(blocked.exit(), Exit::Blocked, "{blocked:?}");
    assert_eq!(history(h), ["1"], "a refused revert undoes nothing");
}

pub(super) fn case_assertion_commands_keep_their_audit_and_refuse_without_writing<H: Harness>(
    h: &mut H,
) {
    let entries = [script::<H>("1", "CREATE TABLE legacy (id INTEGER);")];
    let legacy = catalog(&entries);
    let adoption = Adoption::new(version("1"), "restored from backup").with_evidence("TICKET-7");
    let unknown = Recovery::new(RecoveryAction::Completed(9_999), "the body finished");

    h.execute("CREATE TABLE legacy (id INTEGER)");

    let unmanaged = operate(h, &legacy, &Command::Apply(ApplyOptions::DEFAULT));
    let adopted = operate(h, &legacy, &Command::Adopt(adoption));
    let rows = journal(h);
    let again = operate(h, &legacy, &Command::Adopt(adoption));
    let recovered = operate(h, &legacy, &Command::Recover(unknown));
    let human = adopted.render(Format::Human, &Redactor::new());

    assert_eq!(unmanaged.exit(), Exit::Blocked, "{unmanaged:?}");
    assert_eq!(adopted.exit(), Exit::Success, "{adopted:?}");
    assert!(human.contains("nothing verified it"), "{human}");
    assert_eq!(
        again.exit(),
        Exit::Blocked,
        "a managed target is never adopted again"
    );
    assert_eq!(recovered.exit(), Exit::Blocked, "{recovered:?}");
    assert_eq!(journal(h), rows, "refused assertions record nothing");
    assert_eq!(
        h.count(
            "SELECT count(*) FROM _rupa_migrations WHERE kind = 'adopt' AND reason = 'restored from backup' AND evidence = 'TICKET-7'"
        ),
        1,
        "the adoption is audited"
    );
}

pub(super) fn case_offline_commands_never_connect<H: Harness>(h: &mut H) {
    let entries = pair::<H>();
    let pair = catalog(&entries);
    let engine = match dialect(h) {
        DialectId::Postgres => Engine::Postgres,
        DialectId::MySql => Engine::MySql,
        _ => Engine::Sqlite,
    };

    let validated = unreachable::<H, _>(&pair, &Command::Validate(engine));
    let status = unreachable::<H, _>(&pair, &Command::Status);

    assert_eq!(validated.exit(), Exit::Success, "{validated:?}");
    assert_eq!(status.exit(), Exit::Failed);
    assert!(
        status
            .failure
            .as_ref()
            .is_some_and(|failure| failure.message == "connection refused"),
        "{status:?}"
    );
}
