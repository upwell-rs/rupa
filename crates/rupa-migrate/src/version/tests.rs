use super::*;

#[test]
fn numbered_versions_compare_numerically_and_allow_gaps() {
    let inputs = ["10", "2", "18446744073709551615"];

    let mut versions = inputs.map(|s| VersionScheme::Numbered.parse(s).expect("valid number"));

    versions.sort();
    assert_eq!(
        versions.map(|v| v.to_string()),
        ["2", "10", "18446744073709551615"]
    );
}

#[test]
fn numbered_versions_reject_aliases_and_overflow() {
    let inputs = [
        "0",
        "01",
        "+1",
        "-1",
        " 1",
        "1 ",
        "1_000",
        "١",
        "1.2",
        "18446744073709551616",
    ];

    for input in inputs {
        assert!(VersionScheme::Numbered.parse(input).is_err(), "{input}");
    }
}

#[test]
fn utc_versions_validate_calendar_and_preserve_width() {
    let inputs = [
        "00010101000000",
        "20000229000000",
        "20240229000000",
        "99991231235959",
    ];

    for input in inputs {
        let version = VersionScheme::UtcTimestamp
            .parse(input)
            .expect("valid UTC timestamp");

        assert_eq!(version.to_string(), input);
    }
}

#[test]
fn utc_versions_reject_impossible_dates_times_and_spelling() {
    let inputs = [
        "00000101000000",
        "19000229000000",
        "20230229000000",
        "20261301000000",
        "20260431000000",
        "20261019240000",
        "20261019096000",
        "20261019093060",
        "2026101909300",
        "202610190930000",
        "20261019093000Z",
    ];

    for input in inputs {
        assert!(VersionScheme::UtcTimestamp.parse(input).is_err(), "{input}");
    }
}
