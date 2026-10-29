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
            inspection: [
                case_inspection_never_initializes_a_target,
                case_status_and_plan_report_without_writing,
                case_status_sees_a_live_runner_without_waiting,
                case_status_reports_attempts_and_leaves_them_running,
                case_status_reports_every_history_mismatch,
            ],
            ordering: [
                case_pending_versions_run_in_version_order,
                case_strict_apply_refuses_a_late_version_before_running_it,
                case_permitted_late_version_runs_last_with_a_new_rank,
                case_a_failed_permitted_run_still_reports_its_late_commits,
                case_late_permission_never_lifts_other_blockers,
            ],
            reversal: [
                case_revert_to_a_version_undoes_the_newest_installations_first,
                case_a_revert_that_is_not_a_suffix_is_refused_before_any_down,
                case_undoing_the_last_installation_makes_it_late_again,
                case_an_irreversible_installation_refuses_the_whole_revert,
                case_a_failed_down_keeps_earlier_undos_and_reports_the_rest,
                case_planned_and_sql_down_bodies_run_as_checksummed,
                case_a_declared_down_without_a_body_rolls_back,
                case_an_uncertain_down_is_recovered_like_an_up,
            ],
            snapshots: [
                case_a_fresh_target_runs_the_snapshot_then_the_tail,
                case_the_target_version_selects_the_snapshot_and_replay_skips_it,
                case_a_managed_target_never_runs_a_newer_snapshot,
                case_only_a_never_initialized_journal_starts_from_a_snapshot,
                case_snapshot_and_replay_reach_the_same_state,
                case_work_added_at_or_below_a_snapshot_is_uncovered,
                case_covered_sources_must_stay_unchanged_and_present,
                case_a_revert_stops_at_the_snapshot_boundary,
                case_an_uncertain_snapshot_is_recovered_never_replayed,
            ],
            adoption: [
                case_an_unmanaged_target_is_adopted_without_running_bodies,
                case_adoption_needs_an_unmanaged_target_a_reason_and_coverage,
                case_an_adopted_boundary_refuses_uncovered_work_and_deeper_reverts,
            ],
            commands_cases: [
                case_inspection_commands_never_write_or_run_bodies,
                case_commands_apply_and_revert_through_the_library_engine,
                case_assertion_commands_keep_their_audit_and_refuse_without_writing,
                case_offline_commands_never_connect,
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
                case_concurrent_fresh_runners_run_a_snapshot_once,
                case_apply_takes_and_closes_a_native_connection,
            ],
            recovery: [
                case_interrupted_transactional_attempt_is_reconciled,
                case_completed_recovery_records_without_running,
                case_restored_recovery_allows_a_new_apply,
                case_recovery_requires_exclusion,
                case_recovery_refuses_what_it_cannot_assert,
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
                case_body_refused_before_dispatch_fails_without_uncertainty,
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
