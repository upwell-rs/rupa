use rupa_migrate::protocol::sqlite::{action, authorize};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};

use crate::SqliteError;

/// Installs or removes the shared migration authorizer. Prepared statements
/// were authorized under the previous policy, so the cache is flushed first.
pub(super) fn install(conn: &mut rusqlite::Connection, enabled: bool) -> Result<(), SqliteError> {
    conn.flush_prepared_statement_cache();

    if enabled {
        conn.authorizer(Some(decide))?;
    } else {
        conn.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)?;
    }

    Ok(())
}

fn decide(context: AuthContext<'_>) -> Authorization {
    let allowed = raw(&context.action)
        .is_some_and(|(code, first, second)| authorize(code, first, second, context.database_name));

    if allowed {
        Authorization::Allow
    } else {
        Authorization::Deny
    }
}

/// The authorizer action code and arguments of an action the policy can
/// allow; every other action is denied.
fn raw<'c>(action: &AuthAction<'c>) -> Option<(i32, Option<&'c str>, Option<&'c str>)> {
    let named = |code, first| Some((code, Some(first), None));
    let pair = |code, first, second| Some((code, Some(first), Some(second)));

    match *action {
        AuthAction::CreateIndex {
            index_name,
            table_name,
        } => pair(action::CREATE_INDEX, index_name, table_name),
        AuthAction::DropIndex {
            index_name,
            table_name,
        } => pair(action::DROP_INDEX, index_name, table_name),
        AuthAction::CreateTrigger {
            trigger_name,
            table_name,
        } => pair(action::CREATE_TRIGGER, trigger_name, table_name),
        AuthAction::DropTrigger {
            trigger_name,
            table_name,
        } => pair(action::DROP_TRIGGER, trigger_name, table_name),
        AuthAction::Read {
            table_name,
            column_name,
        } => pair(action::READ, table_name, column_name),
        AuthAction::Update {
            table_name,
            column_name,
        } => pair(action::UPDATE, table_name, column_name),
        AuthAction::AlterTable {
            database_name,
            table_name,
        } => pair(action::ALTER_TABLE, database_name, table_name),
        AuthAction::CreateTable { table_name } => named(action::CREATE_TABLE, table_name),
        AuthAction::DropTable { table_name } => named(action::DROP_TABLE, table_name),
        AuthAction::Insert { table_name } => named(action::INSERT, table_name),
        AuthAction::Delete { table_name } => named(action::DELETE, table_name),
        AuthAction::Analyze { table_name } => named(action::ANALYZE, table_name),
        AuthAction::CreateView { view_name } => named(action::CREATE_VIEW, view_name),
        AuthAction::DropView { view_name } => named(action::DROP_VIEW, view_name),
        AuthAction::Reindex { index_name } => named(action::REINDEX, index_name),
        AuthAction::Function { function_name } => {
            Some((action::FUNCTION, None, Some(function_name)))
        }
        AuthAction::Select => Some((action::SELECT, None, None)),
        AuthAction::Recursive => Some((action::RECURSIVE, None, None)),
        _ => None,
    }
}
