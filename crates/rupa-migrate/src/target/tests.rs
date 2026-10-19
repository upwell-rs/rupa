use super::*;

fn target(namespace: &str) -> Target<'_> {
    Target {
        engine: Engine::Postgres,
        database: "app",
        namespace,
    }
}

fn hex(bytes: [u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn digest_matches_independently_computed_vectors() {
    let public = target("public");
    let tenant = target("tenant");

    assert_eq!(
        hex(public.digest()),
        "c455357299b550c508a409462df5e010f8a55bc3eb3b11338edceb1db4de21a1"
    );
    assert_eq!(public.advisory_key(), -4299471502933929787);
    assert_eq!(tenant.advisory_key(), 4682244863844517465);
}

#[test]
fn length_prefixes_keep_component_boundaries_distinct() {
    let split = Target {
        engine: Engine::Postgres,
        database: "ab",
        namespace: "c",
    };
    let shifted = Target {
        engine: Engine::Postgres,
        database: "a",
        namespace: "bc",
    };

    assert_ne!(split.digest(), shifted.digest());
}
