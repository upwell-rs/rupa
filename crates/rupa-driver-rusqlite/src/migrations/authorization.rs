use rusqlite::hooks::{AuthAction, AuthContext, Authorization};

use crate::{SqliteError, SqliteExecutor};

/// Scope-bound native authorization, removed before framework history writes.
pub(super) struct Authorized<'a> {
    pub(super) executor: &'a mut SqliteExecutor,
}

impl<'a> Authorized<'a> {
    pub(super) fn new(executor: &'a mut SqliteExecutor) -> Result<Self, SqliteError> {
        executor.connection().flush_prepared_statement_cache();
        executor.connection().authorizer(Some(authorize))?;

        Ok(Self { executor })
    }
}

impl Drop for Authorized<'_> {
    fn drop(&mut self) {
        self.executor.connection().flush_prepared_statement_cache();
        // This hook is owned by a dedicated connection, which is discarded if unwinding interrupts cleanup.
        let _ = self
            .executor
            .connection()
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    }
}

fn reserved(name: &str) -> bool {
    name.to_ascii_lowercase().starts_with("_rupa_")
}

fn authorize(context: AuthContext<'_>) -> Authorization {
    let allowed = if context.database_name == Some("temp") {
        match context.action {
            AuthAction::Read { table_name, .. } | AuthAction::Update { table_name, .. } => {
                matches!(table_name, "sqlite_temp_master" | "sqlite_temp_schema")
            }
            _ => false,
        }
    } else if context.database_name.is_some_and(|name| name != "main") {
        false
    } else {
        match context.action {
            AuthAction::CreateTable { table_name }
            | AuthAction::DropTable { table_name }
            | AuthAction::Insert { table_name }
            | AuthAction::Delete { table_name }
            | AuthAction::Update { table_name, .. }
            | AuthAction::Read { table_name, .. }
            | AuthAction::AlterTable { table_name, .. }
            | AuthAction::Analyze { table_name } => !reserved(table_name),
            AuthAction::CreateIndex {
                index_name,
                table_name,
            }
            | AuthAction::DropIndex {
                index_name,
                table_name,
            } => !reserved(index_name) && !reserved(table_name),
            AuthAction::CreateTrigger {
                trigger_name,
                table_name,
            }
            | AuthAction::DropTrigger {
                trigger_name,
                table_name,
            } => !reserved(trigger_name) && !reserved(table_name),
            AuthAction::CreateView { view_name } | AuthAction::DropView { view_name } => {
                !reserved(view_name)
            }
            AuthAction::Reindex { index_name } => !reserved(index_name),
            AuthAction::Function { function_name } => {
                !function_name.eq_ignore_ascii_case("load_extension")
            }
            AuthAction::Select | AuthAction::Recursive => true,
            _ => false,
        }
    };

    if allowed {
        Authorization::Allow
    } else {
        Authorization::Deny
    }
}
