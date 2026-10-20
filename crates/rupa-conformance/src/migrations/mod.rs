//! Migration cases shared by every driver with a migration session, in both
//! execution modes. Each case is written once and compiled against
//! `rupa::migrate::sync` and `rupa::migrate::asynchronous`, whose identical
//! names make the same source valid in either mode. A driver's tests supply a
//! [`sync::Harness`] over a real target and call [`sync::run`], and run the
//! async contracts through [`asynchronous::Blocked`].

use rupa::migrate::{ApplyReport, Catalog, MigrationInfo, VersionScheme};

/// Runs every listed case, each on a
/// freshly reset target, printing its name first so a failure's captured
/// output says which case it was.
macro_rules! cases {
    ($mode:literal, $h:ident, $($module:ident: [$($case:ident),* $(,)?]),* $(,)?) => {$($(
        $h.reset();
        eprintln!("migration conformance {} {}", $mode, stringify!($case));
        $module::$case($h);
    )*)*};
}

/// Every case, grouped by module, in run order.
macro_rules! all_cases {
    ($mode:literal, $h:ident) => {
        cases!(
            $mode,
            $h,
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
            nontransactional: [
                case_policies_apply_in_catalog_order_once,
                case_failed_body_keeps_effects_and_blocks_replay,
                case_interrupted_body_is_uncertain,
                case_statements_outside_transactions_need_the_policy,
            ],
        )
    };
}

pub mod asynchronous;
pub mod fixture;
pub mod sync;

/// Inspection and setup of a migration target, independent of the mode.
pub trait Target {
    /// Empties the target's owned namespace.
    fn reset(&mut self);

    /// Runs setup SQL on an independent connection, outside any migration.
    fn execute(&self, sql: &str);

    /// The single integer `sql` selects, read on an independent connection.
    fn count(&self, sql: &str) -> i64;

    /// Whether the owned namespace holds a table named `name`.
    fn table_exists(&self, name: &str) -> bool;

    /// Scripts this engine must refuse beyond the shared ones.
    fn forbidden_scripts(&self) -> &'static [&'static str];

    /// Statements that run against the `notes (id, title)` table this engine
    /// refuses inside a transaction, such as concurrent index creation. Only
    /// read when the session supports nontransactional bodies.
    fn outside_transaction_script(&self) -> &'static str {
        ""
    }
}

fn catalog<M: MigrationInfo>(entries: &[M]) -> Catalog<'_, M> {
    Catalog::new("primary", VersionScheme::Numbered, entries).expect("valid catalog")
}

fn versions(report: &ApplyReport) -> Vec<String> {
    report.applied.iter().map(ToString::to_string).collect()
}
