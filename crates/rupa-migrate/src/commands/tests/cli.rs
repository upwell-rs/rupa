use clap::Parser as _;

use super::super::cli::Migrate;
use super::*;
use crate::{ApplyOptions, Init, Order, RecoveryAction, RevertTarget};

fn parse(args: &[&str]) -> Migrate {
    Migrate::try_parse_from(["migrate"].iter().chain(args)).expect("valid arguments")
}

#[test]
fn arguments_become_the_typed_commands() {
    let apply = parse(&["apply", "--through", "2", "--allow-late", "--full-replay"]);
    let revert = parse(&["revert", "--to", "1", "--dry-run"]);
    let last = parse(&["revert", "--last", "2"]);
    let recover = parse(&[
        "recover",
        "completed",
        "7",
        "--reason",
        "the index exists",
        "--evidence",
        "psql \\d",
    ]);

    assert!(matches!(
        apply.command(VersionScheme::Numbered),
        Ok(Command::Apply(ApplyOptions {
            order: Order::AllowLate,
            through: Some(through),
            init: Init::FullReplay,
        })) if through == version("2")
    ));
    assert!(matches!(
        revert.command(VersionScheme::Numbered),
        Ok(Command::RevertPlan(RevertTarget::To(to))) if to == version("1")
    ));
    assert!(matches!(
        last.command(VersionScheme::Numbered),
        Ok(Command::Revert(RevertTarget::Last(2)))
    ));
    assert!(matches!(
        recover.command(VersionScheme::Numbered),
        Ok(Command::Recover(recovery)) if recovery.action == RecoveryAction::Completed(7)
            && recovery.reason == "the index exists"
            && recovery.evidence == Some("psql \\d")
    ));
    assert_eq!(revert.operation.name(), "revert-plan");
}

#[test]
fn assertions_need_a_reason_and_reverts_a_selection() {
    for args in [
        &["adopt", "--through", "2"][..],
        &["recover", "restored", "7"],
        &["revert"],
        &["revert", "--to", "1", "--last", "1"],
        &["create", "notes", "--snapshot", "--down"],
    ] {
        let parsed = Migrate::try_parse_from(["migrate"].iter().chain(args));

        assert!(parsed.is_err(), "{args:?} must be refused");
    }
}

#[test]
fn versions_are_parsed_with_the_catalog_scheme() {
    let adopt = parse(&["adopt", "--through", "0002", "--reason", "legacy"]);

    assert!(
        adopt.command(VersionScheme::Numbered).is_err(),
        "not canonical"
    );
    assert!(adopt.command(VersionScheme::UtcTimestamp).is_err());
}

#[test]
fn the_url_is_masked_and_global() {
    let migrate = parse(&[
        "status",
        "--url",
        "postgres://app:s3cret@db/app",
        "--format",
        "json",
    ]);

    assert_eq!(migrate.format, Format::Json);
    assert_eq!(
        migrate.redactor().redact("postgres://app:s3cret@db/app"),
        "postgres://app:***@db/app"
    );
}
