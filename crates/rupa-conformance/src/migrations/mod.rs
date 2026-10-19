//! Migration cases shared by every driver with a migration session. A
//! driver's tests supply a [`MigrationHarness`] over a real target and call
//! [`run_migrations`]; the same history, exclusion and control guarantees
//! must then hold on every engine.

mod control;
mod exclusion;
pub mod fixture;
mod history;
mod schema;

use rupa::migrate::{ApplyReport, Catalog, MigrationInfo, Session, VersionScheme};

/// A migration target handing out independent dedicated connections.
pub trait MigrationHarness {
    /// The driver's native migration connection.
    type Session: Session + Send;

    /// Empties the target's owned namespace.
    fn reset(&mut self);

    /// Opens an independent connection to the target. Lock waits should be
    /// short, since exclusion cases expect a busy competitor to give up.
    fn connect(&self) -> Self::Session;

    /// Runs setup SQL on an independent connection, outside any migration.
    fn execute(&self, sql: &str);

    /// The single integer `sql` selects, read on an independent connection.
    fn count(&self, sql: &str) -> i64;

    /// Whether the owned namespace holds a table named `name`.
    fn table_exists(&self, name: &str) -> bool;

    /// Scripts this engine must refuse beyond the shared ones.
    fn forbidden_scripts(&self) -> &'static [&'static str];
}

/// Runs every migration case, each on a freshly reset target. The case name
/// is printed first, so a failure's captured output says which case it was.
pub fn run_migrations<H: MigrationHarness>(h: &mut H) {
    macro_rules! cases {
        ($($module:ident: [$($case:ident),* $(,)?]),* $(,)?) => {$($(
            h.reset();
            eprintln!("migration conformance {}", stringify!($case));
            $module::$case(h);
        )*)*};
    }

    cases!(
        history: [
            case_mixed_apply_records_history_and_never_replays,
            case_failed_body_rolls_back_and_preserves_prior_commits,
            case_changed_applied_body_is_rejected_before_pending_work,
            case_missing_source_and_late_arrival_are_rejected,
            case_unmanaged_objects_are_never_a_fresh_target,
            case_incompatible_reserved_metadata_is_preserved,
            case_inspection_does_not_initialize_history,
            case_version_scheme_cannot_change,
            case_unsupported_policy_is_rejected_before_bootstrap,
            case_malformed_planned_sql_is_rejected_before_bootstrap,
            case_application_errors_keep_their_types,
        ],
        schema: [
            case_planned_schema_creates_entity_tables,
            case_changed_plan_is_reported_as_changed_history,
            case_scripted_migrations_run_schema_operations,
            case_ordinary_renames_and_indexes_work,
        ],
        exclusion: [
            case_owner_excludes_competitors_between_commits,
            case_concurrent_first_runners_execute_once,
            case_interrupted_body_leaves_an_unresolved_attempt,
            case_apply_takes_and_closes_a_native_connection,
        ],
        control: [
            case_scripts_cannot_escape_the_runner,
            case_raw_queries_cannot_escape_the_runner,
            case_renaming_into_reserved_names_cannot_commit,
        ],
    );
}

fn catalog<M: MigrationInfo>(entries: &[M]) -> Catalog<'_, M> {
    Catalog::new("primary", VersionScheme::Numbered, entries).expect("valid catalog")
}

fn versions(report: &ApplyReport) -> Vec<String> {
    report.applied.iter().map(ToString::to_string).collect()
}
