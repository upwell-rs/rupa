//! The sources `create` writes.

/// The source of a new `PlannedMigration`.
pub(super) fn planned(description: &str, name: &str, down: bool) -> String {
    let mut methods = String::from("    fn plan(&self) -> Plan {\n        Plan::new()\n    }\n");

    if down {
        methods
            .push_str("\n    fn down(&self) -> Option<Plan> {\n        Some(Plan::new())\n    }\n");
    }

    format!(
        "//! {description}\n\nuse rupa::migrate::{{Plan, PlannedMigration}};\n\n/// {description}\n#[derive(Debug)]\npub struct {name};\n\nimpl PlannedMigration for {name} {{\n{methods}}}\n"
    )
}

/// The source of a new imperative `Migration` in `module`'s mode, whose
/// methods start with `prefix`.
pub(super) fn imperative(
    module: &str,
    prefix: &str,
    description: &str,
    name: &str,
    down: bool,
) -> String {
    let method = |method: &str| {
        format!(
            "    {prefix}fn {method}(&self, context: &mut Context<'_, S>) -> Result<(), Self::Error> {{\n        context.script(\"\")\n    }}\n"
        )
    };
    let mut methods = method("up");

    if down {
        methods.push('\n');
        methods.push_str(&method("down"));
    }

    format!(
        "//! {description}\n\nuse rupa::migrate::Error;\nuse rupa::migrate::{module}::{{Context, Migration, Session}};\n\n/// {description}\n#[derive(Debug)]\npub struct {name};\n\nimpl<S: Session> Migration<S> for {name} {{\n    type Error = Error<S::Error>;\n\n{methods}}}\n"
    )
}

/// The type name rupa-build expects for `name`.
pub(super) fn pascal(name: &str) -> String {
    name.split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            let first = chars.next().map(|c| c.to_ascii_uppercase());

            first.into_iter().chain(chars).collect::<String>()
        })
        .collect()
}

/// `name` as a sentence: `add_slugs` becomes `Add slugs.`.
pub(super) fn description(name: &str) -> String {
    let words = name.replace('_', " ");
    let mut chars = words.trim().chars();
    let first = chars.next().map(|c| c.to_ascii_uppercase());

    format!("{}.", first.into_iter().chain(chars).collect::<String>())
}
